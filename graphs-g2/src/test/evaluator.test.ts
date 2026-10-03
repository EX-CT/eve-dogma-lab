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

test("graph registry lists all ten graphs", () => {
  assert.deepEqual(Object.keys(graphs()).sort(), ["application_profile", "capacitor", "damage", "ecm_burst", "ewar", "lock_time", "mobility", "remote_reps", "shield_regen", "warp_time"]);
});

test("unknown graph / axis errors", () => {
  assert.throws(() => evaluate({ graph: "nope", fit: {}, x: { axis: "distance_m", values: [0] }, y: ["dps"] } as any, {} as any), /unknown graph/);
  assert.throws(() => evaluate({ graph: "damage", fit: {}, x: { axis: "cap_pct", values: [0] }, y: ["dps"] } as any, {} as any), /no x axis/);
});

test("validation order and empty x", () => {
  assert.throws(() => evaluate({ graph: "nope", x: { axis: "distance_m", values: [0] }, y: ["dps"] } as any, {} as any), /fit missing/);
  assert.throws(() => evaluate({ graph: "damage", fit: {}, x: { axis: "distance_m", values: [0, null] }, y: ["dps"] } as any, {} as any), /finite/);
  assert.throws(() => evaluate({ graph: "damage", fit: {}, x: { axis: "distance_m", values: [0] }, y: [] } as any, {} as any), /y missing/);
  const r = evaluate({ graph: "damage", fit: {}, x: { axis: "distance_m", values: [] }, y: ["dps", "volley"] } as any, {} as any);
  assert.deepEqual(r.x, []);
  assert.deepEqual(r.series, { dps: [], volley: [] });
});

test("ecm_burst: lock time and src_damage loop", () => {
  const prim: any = { source: { items: [], ship: { attrs: { signatureRadius: 100 } }, stats: { offense: { total: { weapon_dps: 100, drone_dps: 50, fighter_dps: 0 } }, defense: { ehp: { total: 3000 } } } } };
  const lock = 40000 / 700 / Math.asinh(100) ** 2;
  const r = evaluate({ graph: "ecm_burst", fit: {}, x: { axis: "tgt_scan_res_mm", values: [700, 0.5] }, y: ["tgt_lock_time_s", "tgt_lock_uptime_s", "src_damage"] } as any, prim);
  assert.ok(Math.abs(r.series.tgt_lock_time_s[0]! - lock) < 1e-9);
  assert.equal(r.series.tgt_lock_time_s[1], null);
  const up = 30 - lock - 1;
  const alive1 = 30 - up + Math.min(up, 3000 / 200); // dies in the first cycle (rem < 0 stops the loop)
  const dmg = alive1 * 100 + (alive1 - 3) * 50;
  assert.ok(Math.abs(r.series.src_damage[0]! - dmg) < 1e-6, `${r.series.src_damage[0]} vs ${dmg}`);
  assert.throws(() => evaluate({ graph: "ecm_burst", fit: {}, x: { axis: "tgt_dps", values: [1] }, y: ["tgt_lock_time_s"] } as any, prim), /not defined/);
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
