#!/usr/bin/env node
// graph-batch: GraphRequest JSONL on stdin -> GraphResult JSONL on stdout (same order).
// Stage 1: the engine (`eve-dogma-go graph-primitives`) turns each request into primitives (one process,
// parallel, ordered). Stage 2: the portable evaluator computes the requested points.
// Usage: graph-batch --engine <bin> --dataset <path> [--eval-only primitives.jsonl] [--dump-primitives file]
import { spawn } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { evaluate, GraphError } from "../evaluator/index.js";

function arg(n: string): string | undefined {
  const i = process.argv.indexOf(n);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

async function readStdin(): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const c of process.stdin) chunks.push(c as Buffer);
  return Buffer.concat(chunks).toString("utf8");
}

function primitives(engine: string, dataset: string, lines: string[]): Promise<string[]> {
  return new Promise((res, rej) => {
    const p = spawn(engine, ["--dataset", dataset, "graph-primitives"], { stdio: ["pipe", "pipe", "inherit"] });
    const out: Buffer[] = [];
    p.stdout.on("data", (d) => out.push(d));
    p.on("error", rej);
    p.on("close", (code) => {
      const r = Buffer.concat(out).toString("utf8").split("\n").filter((l) => l.length);
      if (r.length !== lines.length) rej(new Error(`engine returned ${r.length} primitives for ${lines.length} requests (exit ${code})`));
      else res(r);
    });
    p.stdin.end(lines.join("\n") + "\n");
  });
}

function errLine(e: any): string {
  if (e instanceof GraphError) return JSON.stringify({ error: { code: e.code, message: e.message, path: e.path } });
  return JSON.stringify({ error: { code: "INTERNAL", message: String(e?.message ?? e), path: "" } });
}

async function main() {
  const engine = arg("--engine") ?? new URL("../../../variant-c/bin/eve-dogma-go", import.meta.url).pathname;
  const dataset = arg("--dataset") ?? process.env.EVE_DOGMA_DATASET ?? "";
  const lines = (await readStdin()).split("\n").filter((l) => l.trim().length);
  const evalOnly = arg("--eval-only");
  const prims = evalOnly ? readFileSync(evalOnly, "utf8").split("\n").filter((l) => l.length) : await primitives(engine, dataset, lines);
  const dump = arg("--dump-primitives");
  if (dump) writeFileSync(dump, prims.join("\n") + "\n");
  const out: string[] = [];
  for (let i = 0; i < lines.length; i++) {
    try {
      const req = JSON.parse(lines[i]);
      const prim = JSON.parse(prims[i]);
      if (prim.error) {
        out.push(JSON.stringify({ error: prim.error }));
        continue;
      }
      out.push(JSON.stringify(evaluate(req, prim)));
    } catch (e) {
      out.push(errLine(e));
    }
  }
  process.stdout.write(out.join("\n") + "\n");
}

main().catch((e) => {
  process.stderr.write(String(e?.stack ?? e) + "\n");
  process.exit(1);
});
