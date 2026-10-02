// remote_reps graph: outgoing remote repairs vs distance / time (behaviour per CONTRACT-GRAPHS.md).
// Shield reps land at the start of a cycle, armor/hull reps at the end; ancillary reps reload after their
// clip (or, with anc_reload off, keep cycling without the paste bonus); spooling reps ramp up per cycle.
import { floatUnerr, rangeFactor } from "./math.js";
import type { GraphRequest, ItemPrim, Primitives } from "./types.js";

interface RrSrc {
  amount: number; // per cycle, fully boosted
  unboosted: number; // per cycle without charge bonus
  layer: "shield" | "armor" | "hull";
  cycleMs: number;
  avgReloadMs: number;
  reloadMs: number;
  shots: number; // 0 = no clip
  range: number;
  falloff: number;
  mobile: boolean;
  count: number;
  spoolMax: number;
  spoolStep: number;
}

function rrSources(p: Primitives): RrSrc[] {
  const out: RrSrc[] = [];
  for (const it of p.source.items) {
    const active = it.kind === "module" ? it.state === "active" || it.state === "overheated" : (it.active ?? 0) > 0;
    if (!active) continue;
    const eff = Object.entries(it.effect_ranges ?? {}).find(([n]) => /Remote(Armor|Shield|Hull)Repair|RemoteArmorMutadaptive|Remote.*Boost|RemoteRepair|ArmorRepairEntity|ShieldBoostingEntity|HullRepairingEntity|npcEntityRemote|RemoteAAR|remoteArmor|remoteShield|remoteHull/i.test(n) && !/Capacitor|Energy/i.test(n));
    if (!eff) continue;
    const a = it.attrs;
    let layer: RrSrc["layer"];
    let base: number;
    if ((a.shieldBonus ?? 0) > 0) (layer = "shield"), (base = a.shieldBonus);
    else if ((a.armorDamageAmount ?? 0) > 0) (layer = "armor"), (base = a.armorDamageAmount);
    else if ((a.structureDamageAmount ?? 0) > 0) (layer = "hull"), (base = a.structureDamageAmount);
    else continue;
    let boosted = base;
    if (it.charge && a.chargedArmorDamageMultiplier !== undefined && layer === "armor") boosted = base * a.chargedArmorDamageMultiplier;
    const mobile = it.kind !== "module";
    const cyc = it.cycle?.raw_ms ?? Math.max(a.duration ?? 0, a.speed ?? 0);
    out.push({
      amount: boosted, unboosted: base, layer, cycleMs: cyc, avgReloadMs: it.cycle?.avg_reload_ms ?? cyc, reloadMs: it.cycle?.reload_ms ?? 0,
      shots: it.charge ? it.cycle?.shots ?? 0 : 0, range: eff[1].range ?? 0, falloff: eff[1].falloff ?? 0, mobile, count: mobile ? it.active ?? 1 : 1,
      spoolMax: a.repairMultiplierBonusMax ?? 0, spoolStep: a.repairMultiplierBonusPerCycle ?? 0,
    });
  }
  return out;
}

/** Cycle schedule up to tS: list of [start s, amount]. */
function schedule(s: RrSrc, tS: number, ancReload: boolean): { start: number; end: number; amount: number }[] {
  const out: { start: number; end: number; amount: number }[] = [];
  const cyc = s.cycleMs / 1000;
  if (cyc <= 0) return out;
  let t = 0;
  let n = 0;
  let shot = 0;
  while (floatUnerr(t) <= floatUnerr(tS) && n < 100000) {
    let amt = s.amount;
    if (s.shots > 0 && !ancReload && n >= s.shots) amt = s.unboosted;
    if (s.spoolStep > 0) amt *= 1 + Math.min(s.spoolStep * n, s.spoolMax);
    out.push({ start: t, end: t + cyc, amount: amt });
    n++;
    t += cyc;
    if (s.shots > 0 && ancReload && ++shot >= s.shots) {
      shot = 0;
      t += s.reloadMs / 1000;
    }
  }
  return out;
}

export function remoteReps(req: GraphRequest, p: Primitives): Record<string, (number | null)[]> {
  const prm = req.params ?? {};
  const settings = req.settings ?? {};
  const ancReload = prm.anc_reload ?? true;
  const dcr = p.source.stats.drones?.control_range_m ?? Infinity;
  const ignoreDcr = settings.ignore_drone_control_range ?? false;
  const srcs = rrSources(p);
  const rf = (s: RrSrc, d: number | null) => (s.mobile ? (d === null || ignoreDcr || d <= dcr ? 1 : 0) : rangeFactor(s.range, s.falloff, d, true));
  const at = (d: number | null, t: number | null) => {
    let rps = 0;
    let total: number | null = null;
    for (const s of srcs) {
      const f = rf(s, d) * s.count;
      if (f === 0) continue;
      if (t === null) {
        const cyc = (s.shots > 0 && ancReload ? s.avgReloadMs : s.cycleMs) / 1000;
        const amt = s.amount * (1 + s.spoolMax);
        if (cyc > 0) rps += (amt / cyc) * f;
      } else {
        const sch = schedule(s, t, ancReload);
        const tu = floatUnerr(t);
        const cur = sch.find((c) => floatUnerr(c.start) <= tu && tu < floatUnerr(c.end));
        if (cur) rps += (cur.amount / (s.cycleMs / 1000)) * f;
        let tot = 0;
        for (const c of sch) if (floatUnerr(s.layer === "shield" ? c.start : c.end) <= tu) tot += c.amount;
        total = (total ?? 0) + tot * f;
      }
    }
    if (t !== null && total === null) total = 0;
    return { rps, total };
  };
  const out: Record<string, (number | null)[]> = {};
  for (const y of req.y) {
    out[y] = req.x.values.map((x) => {
      let d: number | null = prm.distance_m ?? null;
      let t: number | null = prm.time_s ?? null;
      if (req.x.axis === "distance_m") {
        if (x < 0) return null;
        d = x;
      } else {
        if (x < 0 || x > 2500) return null;
        t = x;
      }
      const r = at(d, t);
      if (y === "rps") return r.rps;
      if (y === "total") return t === null ? null : r.total;
      return null;
    });
  }
  return out;
}
