// Portable graph evaluator (approach G2): GraphRequest + engine primitives -> GraphResult. No I/O, no deps.
import { capacitor } from "./capacitor.js";
import { ewar } from "./ewar.js";
import { damageGraph } from "./damage.js";
import { remoteReps } from "./rr.js";
import { lockTime, mobility, shieldRegen, warpTime } from "./simple.js";
import type { GraphRequest, GraphResult, Primitives } from "./types.js";
import { GraphError } from "./types.js";
export * from "./types.js";

type Fn = (req: GraphRequest, p: Primitives) => Record<string, (number | null)[]>;
const registry: Record<string, { axes: string[]; ys: string[]; fn: Fn }> = {};

export function registerGraph(name: string, axes: string[], ys: string[], fn: Fn) {
  registry[name] = { axes, ys, fn };
}

registerGraph("lock_time", ["tgt_sig_m"], ["time_s"], lockTime);
registerGraph("warp_time", ["distance_m"], ["time_s"], warpTime);
registerGraph("mobility", ["time_s"], ["speed_mps", "distance_m", "momentum_kg_mps", "bump_speed_mps", "bump_distance_m"], mobility);
registerGraph("shield_regen", ["time_s", "shield_pct"], ["shield_hp", "shield_regen_hp_s"], shieldRegen);
registerGraph("ewar", ["distance_m"], ["neut_gj_s", "web_pct", "ecm_strength", "damp_lock_range_pct", "td_optimal_pct", "gd_range_pct", "tp_sig_pct"], ewar);
registerGraph("remote_reps", ["distance_m", "time_s"], ["rps", "total"], remoteReps);
registerGraph("damage", ["distance_m", "time_s", "tgt_speed_mps", "tgt_sig_m"], ["dps", "volley", "damage"], damageGraph);
registerGraph("capacitor", ["time_s", "cap_pct"], ["cap_gj", "cap_regen_gj_s"], capacitor);

export function graphs() {
  return Object.fromEntries(Object.entries(registry).map(([k, v]) => [k, { axes: v.axes, y: v.ys }]));
}

const clean = (v: number | null) => (v === null || !Number.isFinite(v) ? null : v);

export function evaluate(req: GraphRequest, prim: Primitives): GraphResult {
  const g = registry[req.graph];
  if (!g) throw new GraphError("UNKNOWN_GRAPH", `unknown graph ${req.graph}`, "graph");
  if (!g.axes.includes(req.x?.axis)) throw new GraphError("BAD_AXIS", `graph ${req.graph} has no x axis ${req.x?.axis}`, "x.axis");
  for (const y of req.y ?? []) if (!g.ys.includes(y) && !y.endsWith("_charge_type_id")) throw new GraphError("BAD_AXIS", `graph ${req.graph} has no series ${y}`, "y");
  const series = g.fn(req, prim);
  const out: Record<string, (number | null)[]> = {};
  for (const [k, v] of Object.entries(series)) out[k] = v.map((x) => (k.endsWith("_charge_type_id") ? x : clean(x)));
  return { graph: req.graph, x_axis: req.x.axis, x: req.x.values, series: out };
}
