// application_profile graph: the damage model evaluated once per charge the dominant weapon group can load
// (variants exported by the engine), keeping the best value per point. Written from the contract.
import { damageGraph } from "./damage.js";
import type { GraphRequest, Primitives } from "./types.js";
import { GraphError } from "./types.js";

interface Variant { type_id: number; name: string; meta_group?: number; meta_level?: number; source: { items: any[]; offense: any } }

/** quality tiers: t1 = Tech I charges; navy = Tech I + faction; all = everything the module can load */
function inTier(v: Variant, tier: string): boolean {
  const mg = v.meta_group ?? 1;
  if (tier === "all") return true;
  if (tier === "t1") return mg === 1;
  // navy: everything except pirate-faction charges (empire navy faction charges stay)
  if (tier === "navy") return mg !== 4 || /\b(Navy|Republic Fleet)\b/.test(v.name);
  throw new GraphError("BAD_PARAM", `unknown ammo_quality ${tier}`, "params.ammo_quality");
}

/**
 * Spacing of the distance grid on which the target's (webbed / painted) speed and signature are sampled:
 * the profile's reach X rounded up to 25 km, split into 250 steps. X = longest turret optimal + 2 x falloff over
 * every loadable charge, or the longest missile flight (velocity x flight time) over the charges of the tier.
 */
function targetGrid(ch: { modules: number[]; variants: Variant[] } | undefined, tier: string): number {
  if (!ch) return 0;
  const only = new Set(ch.modules);
  let x = 0;
  for (const v of ch.variants) {
    const it = v.source.items.find((i: any) => i.kind === "module" && only.has(i.index));
    if (!it) continue;
    const a = it.attrs;
    if ((a.maxRange ?? 0) > 0) x = Math.max(x, a.maxRange + 2 * (a.falloff ?? 0));
    else if (inTier(v, tier)) {
      const c = it.charge?.attrs ?? {};
      x = Math.max(x, ((c.maxVelocity ?? 0) * (c.explosionDelay ?? 0)) / 1000);
    }
  }
  return x > 0 ? (Math.ceil(x / 25000) * 25000) / 250 : 0;
}

export function applicationProfile(req: GraphRequest, p: Primitives): Record<string, (number | null)[]> {
  if (req.x.axis !== "distance_m") throw new GraphError("BAD_AXIS", `application_profile has no x axis ${req.x.axis}`, "x.axis");
  for (const y of req.y) if (y !== "dps" && y !== "volley") throw new GraphError("BAD_AXIS", `application_profile has no series ${y}`, "y");
  const tier = req.params?.ammo_quality ?? "all";
  const ch = (p as any).charges as { modules: number[]; variants: Variant[] } | undefined;
  const only = new Set<number>(ch?.modules ?? []);
  const n = req.x.values.length;
  const out: Record<string, (number | null)[]> = {};
  for (const y of req.y) {
    out[y] = req.x.values.map((x) => (x < 0 ? null : 0));
    out[`${y}_charge_type_id`] = req.x.values.map(() => null);
  }
  const dreq: GraphRequest = { ...req, graph: "damage", y: req.y, settings: { ...(req.settings ?? {}), _targetGrid: targetGrid(ch, tier) } as any };
  for (const v of ch?.variants ?? []) {
    if (!inTier(v, tier)) continue;
    // the variant carries only what the charge changes: the dominant modules and the offense stats
    const items = p.source.items.map((it) => (it.kind === "module" && only.has(it.index as number) ? v.source.items.find((x) => x.index === it.index) ?? it : it));
    const vp: Primitives = { ...p, source: { ...p.source, items, stats: { ...p.source.stats, offense: v.source.offense } } };
    const r = damageGraph(dreq, vp, only);
    for (const y of req.y) {
      for (let i = 0; i < n; i++) {
        const val = r[y][i];
        if (val === null) continue;
        if (out[`${y}_charge_type_id`][i] === null || val > (out[y][i] as number)) {
          out[y][i] = val;
          out[`${y}_charge_type_id`][i] = v.type_id;
        }
      }
    }
  }
  return out;
}
