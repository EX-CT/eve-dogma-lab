// Node check of the WebAssembly build: every bench 1.8.0 case vs the Pyfa-expected values (bench tolerance), and,
// when the native binary is built, byte-identical output to the CLI.
//   cd variant-h/wasm && cargo build --release --target wasm32-unknown-unknown
//   node web/test-node.mjs [path/to.wasm] [dataset.json.gz]
import { readFileSync, existsSync } from "node:fs";
import { gunzipSync } from "node:zlib";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { load } from "./eve-dogma-h.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const wasmPath = process.argv[2] || [process.env.CARGO_TARGET_DIR, join(root, "wasm/target")]
  .filter(Boolean).map((d) => join(d, "wasm32-unknown-unknown/release/eve_dogma_h_wasm.wasm")).find(existsSync);
const dataset = process.argv[3] || process.env.EVE_DOGMA_DATASET || "/workspace/exct-eve/data/dataset-3569502.json.gz";
if (!wasmPath) throw new Error("wasm not built (see header)");

const t0 = performance.now();
const h = await load(readFileSync(wasmPath), readFileSync(dataset));
const tLoad = performance.now() - t0;

const cases = gunzipSync(readFileSync(join(root, "tests/fixtures/bench-1.8.0.jsonl.gz"))).toString().trim().split("\n").map((l) => JSON.parse(l));

const pointer = (doc, ptr) => {
  let cur = doc;
  for (const part of ptr.replace(/^\/+|\/+$/g, "").split("/")) {
    const m = part.match(/^(.*)\[(.*)=(.*)\]$/);
    if (m) {
      const arr = cur?.[m[1]];
      if (!Array.isArray(arr)) return null;
      cur = arr.find((e) => e && String(e[m[2]]) === m[3]);
      if (cur === undefined) return null;
    } else if (cur && typeof cur === "object" && part in cur) cur = cur[part];
    else return null;
  }
  return cur;
};
const extract = (doc, expr) => {
  if (!expr.includes("+")) return pointer(doc, expr);
  const v = expr.split("+").map((p) => pointer(doc, p));
  return v.every((x) => x == null) ? null : v.reduce((s, x) => s + Number(x || 0), 0);
};
const close = (g, w) => {
  if (typeof g === "boolean" || typeof w === "boolean") return g != null && !!g === !!w;
  if (g == null || w == null) return g === w;
  return Math.abs(g - w) <= Math.max(1e-3, 1e-4 * Math.abs(w));
};

let okCases = 0, okVals = 0, nVals = 0;
const outs = [];
const t1 = performance.now();
for (const c of cases) {
  const text = h.calcText(JSON.stringify(c.request));
  outs.push(text);
  const out = JSON.parse(text);
  let ok = !out.error;
  for (const [, ptr, want] of c.expect) {
    nVals++;
    if (close(extract(out, ptr), want)) okVals++; else ok = false;
  }
  if (ok) okCases++;
}
const tCalc = (performance.now() - t1) / cases.length;
console.log(`wasm ${wasmPath}: dataset load ${tLoad.toFixed(0)} ms, ${tCalc.toFixed(3)} ms/calc`);
console.log(`vs Pyfa (bench 1.8.0): cases ${okCases}/${cases.length}, values ${okVals}/${nVals}`);

const native = join(root, "target/release/eve-dogma-h");
let identical = null;
if (existsSync(native)) {
  const input = cases.map((c) => JSON.stringify(c.request)).join("\n") + "\n";
  const nat = execFileSync(native, ["--dataset", dataset, "batch"], { input, maxBuffer: 1 << 30 }).toString().trim().split("\n");
  identical = nat.filter((l, i) => l === outs[i]).length;
  console.log(`identical to the native CLI: ${identical}/${cases.length}`);
}
const s = h.search("rifter");
console.log(`search("rifter")[0] = ${s[0]?.name}`);
if (okCases !== cases.length || (identical !== null && identical !== cases.length)) process.exit(1);
