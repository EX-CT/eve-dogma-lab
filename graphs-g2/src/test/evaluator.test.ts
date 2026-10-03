// Unit + golden regression tests for the portable evaluator (node --test dist/test/).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { evaluate, graphs } from "../evaluator/index.js";
import { rangeFactor, stackMultiply } from "../evaluator/math.js";

test("range factor: full inside optimal, half at optimal + falloff, zero past optimal + 3 falloff", () => {
  assert.equal(rangeFactor(1000, 500, 800), 1);
  assert.ok(Math.abs(rangeFactor(1000, 500, 1500) - 0.5) < 1e-12);
  assert.equal(rangeFactor(1000, 500, 2600), 0);
  assert.equal(rangeFactor(1000, 0, 1000), 1);
  assert.equal(rangeFactor(1000, 0, 1001), 0);
});

test("stacking penalty: the second-strongest multiplier counts ~86.9 %", () => {
  const m = stackMultiply([0.5, 0.5]);
  assert.ok(Math.abs(m - 0.5 * (1 - 0.5 * 0.8691199806)) < 1e-6, String(m));
});

test("graph registry lists all nine graphs", () => {
  assert.deepEqual(Object.keys(graphs()).sort(), ["application_profile", "capacitor", "damage", "ewar", "lock_time", "mobility", "remote_reps", "shield_regen", "warp_time"]);
});

test("unknown graph / axis errors", () => {
  assert.throws(() => evaluate({ graph: "nope", x: { axis: "distance_m", values: [0] }, y: ["dps"] } as any, {} as any), /unknown graph/);
  assert.throws(() => evaluate({ graph: "damage", x: { axis: "cap_pct", values: [0] }, y: ["dps"] } as any, {} as any), /no x axis/);
});

test("golden: evaluator output for stored primitives is unchanged", () => {
  const file = fileURLToPath(new URL("../../testdata/golden.jsonl", import.meta.url));
  const lines = readFileSync(file, "utf8").trim().split("\n");
  assert.ok(lines.length >= 5);
  for (const l of lines) {
    const g = JSON.parse(l);
    const got = evaluate(g.request, g.primitives);
    for (const [k, want] of Object.entries(g.result.series as Record<string, (number | null)[]>)) {
      want.forEach((w, i) => {
        const v = got.series[k][i];
        if (w === null) assert.equal(v, null, `${g.case} ${k}[${i}]`);
        else assert.ok(v !== null && Math.abs(v - w) <= 1e-9 * Math.max(1, Math.abs(w)), `${g.case} ${k}[${i}]: ${v} != ${w}`);
      });
    }
  }
});
