// Portable graph evaluator (approach G2): GraphRequest + engine primitives -> GraphResult. No I/O, no deps.
import { capacitor } from "./capacitor.js";
import { ewar } from "./ewar.js";
import { damageGraph } from "./damage.js";
import { applicationProfile } from "./app.js";
import { remoteReps } from "./rr.js";
import { lockTime, mobility, shieldRegen, warpTime } from "./simple.js";
import type { GraphRequest, GraphResult, Primitives } from "./types.js";
import { GraphError } from "./types.js";
export * from "./types.js";

type Fn = (req: GraphRequest, p: Primitives) => Record<string, (number | null)[]>;
const registry: Record<string, { axes: string[]; ys: string[]; fn: Fn; pairs?: (x: string, y: string) => boolean }> = {};

export function registerGraph(name: string, axes: string[], ys: string[], fn: Fn, pairs?: (x: string, y: string) => boolean) {
  registry[name] = { axes, ys, fn, pairs };
}

registerGraph("lock_time", ["tgt_sig_m"], ["time_s"], lockTime);
registerGraph("warp_time", ["distance_m"], ["time_s"], warpTime);
registerGraph("mobility", ["time_s"], ["speed_mps", "distance_m", "momentum_kg_mps", "bump_speed_mps", "bump_distance_m"], mobility);
registerGraph("shield_regen", ["time_s", "shield_pct"], ["shield_hp", "shield_regen_hp_s"], shieldRegen);
registerGraph("ewar", ["distance_m"], ["neut_gj_s", "web_pct", "ecm_strength", "damp_lock_range_pct", "td_optimal_pct", "gd_range_pct", "tp_sig_pct"], ewar);
registerGraph("remote_reps", ["distance_m", "time_s"], ["rps", "total"], remoteReps);
registerGraph("damage", ["distance_m", "time_s", "tgt_speed_mps", "tgt_sig_m"], ["dps", "volley", "damage"], damageGraph);
registerGraph("application_profile", ["distance_m"], ["dps", "volley"], applicationProfile);
registerGraph("capacitor", ["time_s", "cap_pct"], ["cap_gj", "cap_regen_gj_s"], capacitor);

export function graphs() {
  return Object.fromEntries(Object.entries(registry).map(([k, v]) => [k, { axes: v.axes, y: v.ys }]));
}

const clean = (v: number | null) => (v === null || !Number.isFinite(v) ? null : v);

const TARGET_GRAPHS = new Set(["damage", "application_profile", "ewar", "remote_reps"]);
/** graphs that read `target` (the engine only builds a target fit for these) */
export function usesTarget(graph: string): boolean {
  return TARGET_GRAPHS.has(graph);
}

const bad = (msg: string, path: string) => new GraphError("BAD_REQUEST", msg, path);
const isObj = (v: unknown) => v !== null && typeof v === "object" && !Array.isArray(v);

/** Contract 0.2 "Validation and error codes": throws the first failing rule's GraphError. */
export function validate(req: any): void {
  if (!isObj(req)) throw bad("request must be an object", "");
  if (typeof req.graph !== "string") throw bad("graph missing or not a string", "graph");
  if (!isObj(req.fit)) throw bad("fit missing or not an object", "fit");
  if (!isObj(req.x)) throw bad("x missing", "x");
  if (!Array.isArray(req.x.values)) throw bad("x.values missing or not an array", "x.values");
  req.x.values.forEach((v: unknown, i: number) => {
    if (typeof v !== "number" || !Number.isFinite(v)) throw bad("x value must be a finite number", `x.values[${i}]`);
  });
  if (!Array.isArray(req.y) || req.y.length === 0) throw bad("y missing, not an array or empty", "y");
  const g = registry[req.graph];
  if (!g) throw new GraphError("UNKNOWN_GRAPH", `unknown graph ${req.graph}`, "graph");
  if (!g.axes.includes(req.x.axis)) throw new GraphError("BAD_AXIS", `graph ${req.graph} has no x axis ${req.x.axis}`, "x.axis");
  req.y.forEach((y: unknown, i: number) => {
    if (typeof y !== "string" || !g.ys.includes(y)) throw new GraphError("BAD_AXIS", `graph ${req.graph} has no series ${String(y)}`, `y[${i}]`);
    if (g.pairs && !g.pairs(req.x.axis, y)) throw new GraphError("BAD_AXIS", `graph ${req.graph}: series ${y} is not defined for x axis ${req.x.axis}`, `y[${i}]`);
  });
  const t = req.target;
  if (usesTarget(req.graph) && isObj(t) && t.resist_mode !== undefined && t.resist_mode !== null &&
      !["auto", "shield", "armor", "hull", "weighted_average"].includes(t.resist_mode))
    throw bad(`unknown resist_mode ${t.resist_mode}`, "target.resist_mode");
  const md = req.settings?.mobile_drone_mode;
  if (md !== undefined && md !== null && !["auto", "follow_attacker", "follow_target"].includes(md)) throw bad(`unknown mobile_drone_mode ${md}`, "settings.mobile_drone_mode");
  const aq = req.params?.ammo_quality;
  if (req.graph === "application_profile" && aq !== undefined && aq !== null && !["t1", "navy", "all"].includes(aq)) throw bad(`unknown ammo_quality ${aq}`, "params.ammo_quality");
}

export function evaluate(req: GraphRequest, prim: Primitives): GraphResult {
  validate(req);
  const g = registry[req.graph];
  if (req.x.values.length === 0) {
    const series: Record<string, (number | null)[]> = {};
    for (const y of req.y) {
      series[y] = [];
      if (req.graph === "application_profile") series[`${y}_charge_type_id`] = [];
    }
    return { graph: req.graph, x_axis: req.x.axis, x: [], series };
  }
  const series = g.fn(req, prim);
  const out: Record<string, (number | null)[]> = {};
  for (const [k, v] of Object.entries(series)) out[k] = v.map((x) => (k.endsWith("_charge_type_id") ? x : clean(x)));
  return { graph: req.graph, x_axis: req.x.axis, x: req.x.values, series: out };
}
