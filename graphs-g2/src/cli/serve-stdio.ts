#!/usr/bin/env node
// serve-stdio: the engine's JSONL RPC plus the contract's `graph` method.
//   {"id","method":"graph","params":<GraphRequest>} -> {"id","result":<GraphResult | {"error":…}>}
// Every other method is proxied unchanged to `eve-dogma-go serve-stdio`. Graph requests go to one persistent
// `eve-dogma-go graph-primitives` process and are evaluated here. Replies keep request order and are flushed per reply.
// Usage: serve-stdio --engine <bin> --dataset <path> [-j N]
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createInterface } from "node:readline";
import { arg, defaultEngine, finish, precheck } from "./common.js";

const engine = arg("--engine") ?? defaultEngine();
const dataset = arg("--dataset") ?? process.env.EVE_DOGMA_DATASET ?? "";
const j = arg("-j");

/** a long-running ordered JSONL child: write a line, get its reply line back in order */
class Child {
  private p: ChildProcessWithoutNullStreams | null = null;
  private waiting: ((line: string) => void)[] = [];
  constructor(private mode: string) {}
  send(line: string): Promise<string> {
    if (!this.p) this.start();
    return new Promise((res) => {
      this.waiting.push(res);
      this.p!.stdin.write(line + "\n");
    });
  }
  private start() {
    const a = ["--dataset", dataset, this.mode];
    if (j) a.push("-j", j);
    const p = spawn(engine, a, { stdio: ["pipe", "pipe", "pipe"] });
    p.stderr.on("data", (d) => {
      const s = String(d);
      if (!/ready \(sde/.test(s)) process.stderr.write(s);
    });
    createInterface({ input: p.stdout }).on("line", (l) => this.waiting.shift()?.(l));
    const fail = (why: string) => {
      const w = this.waiting;
      this.waiting = [];
      this.p = null;
      for (const r of w) r(JSON.stringify({ error: { code: "INTERNAL", message: `engine ${this.mode}: ${why}`, path: "" } }));
    };
    p.on("error", (e) => fail(e.message));
    p.on("exit", (c) => fail(`exited (${c})`));
    this.p = p;
  }
  close() {
    this.p?.stdin.end();
  }
}

const rpcEngine = new Child("serve-stdio");
const primEngine = new Child("graph-primitives");

// ordered output: each input line gets a slot; slots are written as soon as all earlier ones are done
const slots: (string | undefined)[] = [];

let base = 0;
function fill(i: number, s: string) {
  slots[i - base] = s;
  while (slots.length && slots[0] !== undefined) {
    process.stdout.write(slots.shift()! + "\n");
    base++;
  }
}

let n = 0;
let pending = 0;
let closed = false;
const done = () => {
  if (closed && pending === 0) {
    rpcEngine.close();
    primEngine.close();
  }
};

createInterface({ input: process.stdin, crlfDelay: Infinity }).on("line", (line) => {
  if (!line.trim()) return;
  const i = n++;
  slots.push(undefined);
  pending++;
  const reply = (s: string) => {
    fill(i, s);
    pending--;
    done();
  };
  let msg: any;
  try {
    msg = JSON.parse(line);
  } catch (e) {
    reply(JSON.stringify({ id: null, error: { code: "BAD_JSON", message: (e as Error).message } }));
    return;
  }
  if (msg?.method !== "graph") {
    rpcEngine.send(line).then(reply);
    return;
  }
  const id = msg.id ?? null;
  const req = msg.params;
  const early = precheck(req);
  if (early) {
    reply(JSON.stringify({ id, result: early }));
    return;
  }
  primEngine.send(JSON.stringify(req)).then((prim) => reply(JSON.stringify({ id, result: finish(req, prim) })));
}).on("close", () => {
  closed = true;
  done();
});
