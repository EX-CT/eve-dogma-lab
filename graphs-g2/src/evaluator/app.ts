// application_profile graph: the damage model evaluated once per charge the dominant weapon group can load
// (variants exported by the engine), keeping the best value per point. Written from the contract.
import { missileRanges, prepareDamage } from "./damage.js";
import type { GraphRequest, Primitives } from "./types.js";
import { GraphError } from "./types.js";

interface Variant { type_id: number; name: string; meta_group?: number; meta_level?: number; source: { items: any[]; offense: any } }

/**
 * Quality tiers (contract 0.3 proposal, graphs/pending.md item 6; verified against the oracle):
 * t1 = meta group 1 or none; navy = t1 + Tech II (meta group 2) + faction (4) charges named with an empire navy
 * prefix (Imperial Navy / Republic Fleet / Caldari Navy / Federation Navy + space), or for "… XL" charges the prefixes
 * "Sansha " / "Arch Angel " / "Shadow " (so "Sanshas …" is not navy); all = everything. If no candidate has a meta group, the tier filter is skipped.
 */
function inTier(v: Variant, tier: string): boolean {
  const mg = v.meta_group;
  if (tier === "all") return true;
  const t1 = mg === undefined || mg === null || mg === 1;
  if (tier === "t1") return t1;
  if (tier === "navy") {
    if (t1 || mg === 2) return true;
    if (mg !== 4) return false;
    return / XL$/.test(v.name) ? /^(Sansha|Arch Angel|Shadow) /.test(v.name) : /^(Imperial Navy|Republic Fleet|Caldari Navy|Federation Navy) /.test(v.name);
  }
  throw new GraphError("BAD_REQUEST", `unknown ammo_quality ${tier}`, "params.ammo_quality");
}

/**
 * Spacing of the distance grid on which the target's (webbed / painted / scrammed) speed and signature are sampled
 * and linearly interpolated (Pyfa's projected cache, contract 0.3 proposal in the bench's graphs/pending.md):
 * step = max(100, ceil(R / 300 / 100) * 100) m. R = max over the dominant modules of
 *   turrets:   trunc(base optimal * longest weaponRangeMultiplier of the tier's charges + base falloff * 3.1)
 *              (base = the module's value with the loaded charge's multipliers divided out),
 *   launchers: the longest "higher" flight range (ceil of the flight time, minus ship radius) of the tier's charges.
 */
function targetGrid(p: Primitives, ch: { modules: number[]; variants: Variant[] } | undefined, tier: string): number {
  if (!ch) return 0;
  const only = new Set(ch.modules);
  let r = 0;
  let longest = 1;
  let missile = 0;
  for (const v of ch.variants) {
    if (!inTier(v, tier)) continue;
    const it = v.source.items.find((i: any) => i.kind === "module" && only.has(i.index));
    const c = it?.charge?.attrs ?? {};
    longest = Math.max(longest, c.weaponRangeMultiplier || 1);
    if ((c.maxVelocity ?? 0) > 0) missile = Math.max(missile, missileRanges(p.source, c, null).hi);
  }
  for (const it of p.source.items) {
    if (it.kind !== "module" || !only.has(it.index as number)) continue;
    const a = it.attrs;
    if ((a.maxRange ?? 0) > 0 && it.weapon_kind === "turret") {
      const c: any = it.charge?.attrs ?? {};
      const opt = a.maxRange / (c.weaponRangeMultiplier || 1);
      const fall = (a.falloff ?? 0) / (c.fallofMultiplier || 1);
      r = Math.max(r, Math.trunc(opt * longest + fall * 3.1));
    } else r = Math.max(r, missile);
  }
  if (r <= 0) return 100;
  const step = r / 300;
  return step <= 100 ? 100 : Math.ceil(step / 100) * 100;
}

const sampleStep = (r: number) => (!(r > 0) || r / 300 <= 100 ? 100 : Math.ceil(r / 300 / 100) * 100);

/**
 * Which charge a weapon type uses at each distance (observed Pyfa behaviour, see DESIGN.md): the best charge by
 * applied volley is found at 0, step, 2·step … up to the type's reach (step = sampleStep(reach)); when it changes
 * between two scan points the switch distance is bisected down to ≤ 10 m (integer midpoints) and the new charge
 * applies from the upper bound. Charges that are only best between two scan points are never picked; beyond the
 * reach the last charge stays. Turret reach = trunc(max charged optimal + max charged falloff · 3.1); launcher reach =
 * the longest charge flight range, and the scan ends (no damage from there on) once the best volley is < 0.01.
 * Strictly larger volley wins, so when every charge does 0 the first candidate is used (vi 0).
 */
function chargeTransitions(p: Primitives, variants: Variant[], idxs: Set<number>, evalAt: (vi: number, xs: number[], y: string) => (number | null)[]): { at: number; vi: number }[] {
  const items = variants.map((v) => v.source.items.find((i: any) => i.kind === "module" && idxs.has(i.index)));
  const turret = items.some((it) => it?.weapon_kind === "turret");
  let reach = 0;
  if (turret) {
    let opt = 0;
    let fall = 0;
    for (const it of items) (opt = Math.max(opt, it?.attrs.maxRange ?? 0)), (fall = Math.max(fall, it?.attrs.falloff ?? 0));
    reach = Math.trunc(opt + fall * 3.1);
  } else {
    for (const it of items) if ((it?.charge?.attrs?.maxVelocity ?? 0) > 0) reach = Math.max(reach, missileRanges(p.source, it.charge.attrs, null).hi);
    reach = Math.trunc(reach);
  }
  const step = sampleStep(reach);
  const scan: number[] = [];
  for (let d = 0; d <= Math.max(reach, 0); d += step) scan.push(d);
  if (!scan.length) scan.push(0);
  const vols = variants.map((_, vi) => evalAt(vi, scan, "volley"));
  const pick = (get: (vi: number) => number) => {
    let bv = 0;
    let bi = -1;
    for (let vi = 0; vi < variants.length; vi++) {
      const v = get(vi);
      if (v > bv) (bv = v), (bi = vi);
    }
    return { bv, bi };
  };
  const at = (d: number) => pick((vi) => evalAt(vi, [d], "volley")[0] ?? 0);
  const first = pick((vi) => vols[vi][0] ?? 0);
  const out: { at: number; vi: number }[] = [{ at: 0, vi: Math.max(first.bi, 0) }];
  let cur = first.bi;
  for (let k = 1; k < scan.length; k++) {
    let { bv, bi } = pick((vi) => vols[vi][k] ?? 0);
    if (bi !== cur) {
      let lo = scan[k] - step;
      let hi = scan[k];
      while (hi - lo > 10) {
        const mid = Math.floor((lo + hi) / 2);
        if (at(mid).bi === cur) lo = mid;
        else hi = mid;
      }
      bv = at(hi).bv;
      out.push({ at: hi, vi: Math.max(bi, 0) });
      cur = bi;
    }
    if (!turret && bv < 0.01) {
      out.push({ at: scan[k], vi: -1 });
      break;
    }
  }
  return out;
}

export function applicationProfile(req: GraphRequest, p: Primitives): Record<string, (number | null)[]> {
  if (req.x.axis !== "distance_m") throw new GraphError("BAD_AXIS", `application_profile has no x axis ${req.x.axis}`, "x.axis");
  for (const y of req.y) if (y !== "dps" && y !== "volley") throw new GraphError("BAD_AXIS", `application_profile has no series ${y}`, "y");
  const ch0 = (p as any).charges as { variants: Variant[] } | undefined;
  const anyMeta = (ch0?.variants ?? []).some((v) => v.meta_group !== undefined && v.meta_group !== null);
  const tier = anyMeta ? req.params?.ammo_quality ?? "all" : "all";
  const ch = (p as any).charges as { modules: number[]; variants: Variant[] } | undefined;
  const only = new Set<number>(ch?.modules ?? []);
  const n = req.x.values.length;
  const out: Record<string, (number | null)[]> = {};
  for (const y of req.y) {
    out[y] = req.x.values.map((x) => (x < 0 ? null : 0));
    out[`${y}_charge_type_id`] = req.x.values.map(() => null);
  }
  const dreq: GraphRequest = { ...req, graph: "damage", y: req.y, settings: { ...(req.settings ?? {}), ignore_lock_range: true, _app: true, _targetGrid: targetGrid(p, ch, tier), _ptCache: new Map(), _ctx: {} } as any };
  const variants = (ch?.variants ?? []).filter((v) => inTier(v, tier));
  if (!variants.length) return out;
  // the variant carries only what the charge changes: the dominant modules and the offense stats
  const vps: Primitives[] = variants.map((v) => {
    const vIdx = new Map<number, any>(v.source.items.map((x: any) => [x.index, x]));
    const items = p.source.items.map((it) => (it.kind === "module" && only.has(it.index as number) ? vIdx.get(it.index as number) ?? it : it));
    return { ...p, source: { ...p.source, items, stats: { ...p.source.stats, offense: v.source.offense } } };
  });
  // one charge choice per module type (Pyfa caches per weapon type), see chargeTransitions
  const byType = new Map<number, Set<number>>();
  for (const it of p.source.items) {
    if (it.kind !== "module" || !only.has(it.index as number)) continue;
    if (!byType.has(it.type_id)) byType.set(it.type_id, new Set());
    byType.get(it.type_id)!.add(it.index as number);
  }
  const best: { val: number; id: number }[][] = req.y.map(() => req.x.values.map(() => ({ val: -1, id: 0 })));
  for (const idxs of byType.values()) {
    const ys = [...new Set([...req.y, "volley"])];
    const prep = vps.map((vp) => prepareDamage({ ...dreq, y: ys }, vp, idxs));
    const evalAt = (vi: number, xs: number[], y: string) => prep[vi](xs)[y];
    const trans = chargeTransitions(p, variants, idxs, evalAt);
    // only the chosen charge is evaluated at each x
    const chosen = req.x.values.map((x) => {
      if (x < 0) return -2;
      let k = 0;
      while (k + 1 < trans.length && trans[k + 1].at <= x) k++;
      return trans[k].vi;
    });
    const groups = new Map<number, number[]>();
    chosen.forEach((vi, i) => {
      if (vi < 0) return;
      if (!groups.has(vi)) groups.set(vi, []);
      groups.get(vi)!.push(i);
    });
    for (const [vi, is] of groups) {
      const r = prep[vi](is.map((i) => req.x.values[i]));
      is.forEach((i, j) => {
        req.y.forEach((y, yi) => {
          const val = r[y][j] ?? 0;
          out[y][i] = (out[y][i] as number) + val;
          if (val > best[yi][i].val) best[yi][i] = { val, id: variants[vi].type_id };
        });
      });
    }
  }
  req.y.forEach((y, yi) => {
    out[`${y}_charge_type_id`] = req.x.values.map((x, i) => (x < 0 || best[yi][i].val < 0 ? null : best[yi][i].id));
  });
  return out;
}
