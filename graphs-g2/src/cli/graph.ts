#!/usr/bin/env node
// graph [FILE]: one GraphRequest (stdin or FILE) -> one GraphResult on stdout; exit 2 with {"error":…} on failure.
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { arg, defaultEngine, errObj, finish, precheck } from "./common.js";

const engine = arg("--engine") ?? defaultEngine();
const dataset = arg("--dataset") ?? process.env.EVE_DOGMA_DATASET ?? "";
const rest = process.argv.slice(2).filter((a, i, all) => !a.startsWith("--") && !(i > 0 && all[i - 1].startsWith("--")));
const text = rest[0] ? readFileSync(rest[0], "utf8") : readFileSync(0, "utf8");
let out: any;
try {
  const req = JSON.parse(text);
  out = precheck(req);
  if (!out) {
    const r = spawnSync(engine, ["--dataset", dataset, "graph-primitives"], { input: JSON.stringify(req) + "\n", maxBuffer: 1 << 30 });
    const line = r.stdout.toString("utf8").split("\n").find((l) => l.length);
    out = line ? finish(req, line) : { error: { code: "INTERNAL", message: `engine produced no output (exit ${r.status})`, path: "" } };
  }
} catch (e) {
  out = errObj(e);
}
process.stdout.write(JSON.stringify(out) + "\n");
process.exit(out?.error ? 2 : 0);
