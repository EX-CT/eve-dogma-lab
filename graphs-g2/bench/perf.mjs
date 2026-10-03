// G2 perf probe: node bench/perf.mjs <requests.jsonl> <primitives.jsonl> [dense_case_name]
// (a) evaluator-only points/s over the corpus (in-process, primitives cached), (b) dense interactive latency:
// damage vs distance, 500 points, evaluator only (the browser use case: primitives fetched once, sliders re-evaluate).
import { readFileSync } from "node:fs";
import { evaluate } from "../dist/evaluator/index.js";
const [reqf, primf] = process.argv.slice(2);
const reqs = readFileSync(reqf, "utf8").trim().split("\n").map((l) => JSON.parse(l));
const prims = readFileSync(primf, "utf8").trim().split("\n").map((l) => JSON.parse(l));
const pts = reqs.reduce((s, r) => s + r.x.values.length * r.y.length, 0);
for (let i = 0; i < reqs.length; i++) evaluate(reqs[i], prims[i]); // warm-up
let n = 0, t0 = performance.now();
while (performance.now() - t0 < 2000) { for (let i = 0; i < reqs.length; i++) evaluate(reqs[i], prims[i]); n++; }
const dt = (performance.now() - t0) / 1000;
console.log(`corpus: ${reqs.length} requests, ${pts} points; evaluator-only ${(n * pts / dt).toFixed(0)} points/s, ${(n * reqs.length / dt).toFixed(0)} requests/s`);
const k = reqs.findIndex((r) => r.graph === "damage" && r.x.axis === "distance_m" && !(r.target && r.target.fit));
const dense = { ...reqs[k], x: { axis: "distance_m", values: Array.from({ length: 500 }, (_, i) => i * 200) }, y: ["dps", "volley"] };
for (let i = 0; i < 20; i++) evaluate(dense, prims[k]);
const m = 200; t0 = performance.now();
for (let i = 0; i < m; i++) evaluate(dense, prims[k]);
console.log(`dense damage/distance 500 points (${reqs[k].fit.ship_type_id ?? ""}): ${((performance.now() - t0) / m).toFixed(3)} ms per request (evaluator only)`);
for (const g of ["application_profile"]) {
  const j = reqs.findIndex((r) => r.graph === g);
  const d2 = { ...reqs[j], x: { axis: "distance_m", values: Array.from({ length: 500 }, (_, i) => i * 200) } };
  evaluate(d2, prims[j]); t0 = performance.now();
  for (let i = 0; i < 20; i++) evaluate(d2, prims[j]);
  console.log(`dense ${g} 500 points: ${((performance.now() - t0) / 20).toFixed(2)} ms per request (${prims[j].charges.variants.length} charges)`);
}
