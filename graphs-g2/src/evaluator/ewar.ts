// ewar graph: neut / web / ECM / damp / TD / GD / TP strength vs distance (behaviour per CONTRACT-GRAPHS.md).
import { rangeFactor, stackMultiply } from "./math.js";
import type { GraphRequest, ItemPrim, Primitives } from "./types.js";

type Kind = "neut" | "nos" | "web" | "ecm" | "damp" | "td" | "gd" | "tp";

interface Src {
  item: ItemPrim;
  kinds: Set<Kind>;
  range: number;
  falloff: number;
  mobile: boolean; // drones / fighters: no range factor, need drone control range
  burst: boolean;
  cycleS: number;
}

function classify(it: ItemPrim): { kinds: Set<Kind>; effect: string | null; burst: boolean } {
  const kinds = new Set<Kind>();
  let effect: string | null = null;
  let burst = false;
  const a = it.attrs;
  for (const e of it.effects) {
    let k: Kind | null = null;
    if (/^doomsdayAOE/.test(e)) burst = true;
    if (/Nosferatu/i.test(e)) k = "nos";
    else if (/Neutraliz/i.test(e)) k = "neut";
    else if (/Webifier|AOEWeb/i.test(e) || e === "fighterAbilityStasisWebifier") k = "web";
    else if (/ECM/.test(e)) k = "ecm";
    else if (/SensorDamp|AOEDamp/i.test(e)) k = "damp";
    else if (/TrackingDisrupt|WeaponDisrupt|AOETrack/i.test(e)) k = "td";
    else if (/TargetPaint|AOEPaint/i.test(e)) k = "tp";
    if (k) {
      effect ??= e;
      if (k === "td") {
        if (a.maxRangeBonus !== undefined || a.falloffBonus !== undefined || a.trackingSpeedBonus !== undefined) kinds.add("td");
        if (a.missileVelocityBonus !== undefined || a.explosionDelayBonus !== undefined) kinds.add("gd");
      } else kinds.add(k);
    }
  }
  return { kinds, effect, burst };
}

export function ewarSources(p: Primitives): Src[] {
  const out: Src[] = [];
  for (const it of p.source.items) {
    if (it.kind === "module" && it.state !== "active" && it.state !== "overheated") continue;
    if (it.kind !== "module" && !(it.active ?? 0)) continue;
    const { kinds, effect, burst } = classify(it);
    if (!kinds.size) continue;
    const er = effect ? it.effect_ranges?.[effect] : undefined;
    let range = er?.range ?? it.attrs.maxRange ?? 0;
    const falloff = er?.falloff ?? it.attrs.falloffEffectiveness ?? 0;
    if (burst) range = (it.attrs.maxRange ?? range) + (it.attrs.doomsdayAOERange ?? 0);
    const cycleS = (it.cycle?.avg_ms ?? (it.attrs.duration ?? it.attrs.speed ?? 0)) / 1000;
    out.push({ item: it, kinds, range, falloff, mobile: it.kind !== "module", burst, cycleS });
  }
  return out;
}

export function ewar(req: GraphRequest, p: Primitives): Record<string, (number | null)[]> {
  const resist = req.params?.resist ?? 0;
  const settings = req.settings ?? {};
  const ignoreDcr = settings.ignore_drone_control_range ?? false;
  const dcr = p.source.stats.drones?.control_range_m ?? Infinity;
  const srcs = ewarSources(p);
  const out: Record<string, (number | null)[]> = {};
  const factor = (s: Src, d: number) => {
    if (s.mobile) return ignoreDcr || d <= dcr ? 1 : 0;
    return rangeFactor(s.range, s.falloff, d, true);
  };
  const count = (s: Src) => (s.mobile ? s.item.active ?? 1 : 1);
  const mults = (kind: Kind, attr: string, d: number) => {
    const l: number[] = [];
    for (const s of srcs) {
      if (!s.kinds.has(kind)) continue;
      const v = s.item.attrs[attr];
      if (v === undefined) continue;
      const rf = factor(s, d);
      for (let k = 0; k < count(s); k++) l.push(1 + (v * (1 - resist) * rf) / 100);
    }
    return stackMultiply(l);
  };
  for (const y of req.y) {
    out[y] = req.x.values.map((d) => {
      if (d < 0) return null;
      switch (y) {
        case "neut_gj_s": {
          let sum = 0;
          for (const s of srcs) {
            if (!s.kinds.has("neut") && !(s.kinds.has("nos") && req.params?.nos_override)) continue;
            const amt = s.item.attrs.energyNeutralizerAmount ?? s.item.attrs.powerTransferAmount ?? 0;
            const cyc = s.mobile ? (s.item.attrs.energyNeutralizerDuration ?? s.item.attrs.duration ?? 0) / 1000 : s.cycleS;
            if (cyc > 0) sum += (amt / cyc) * (1 - resist) * factor(s, d) * count(s);
          }
          return sum;
        }
        case "web_pct":
          return (1 - mults("web", "speedFactor", d)) * 100;
        case "ecm_strength": {
          let sum = 0;
          for (const s of srcs) {
            if (!s.kinds.has("ecm")) continue;
            const a = s.item.attrs;
            const names = s.item.kind === "fighter" ? ["fighterAbilityECMStrengthGravimetric", "fighterAbilityECMStrengthLadar", "fighterAbilityECMStrengthMagnetometric", "fighterAbilityECMStrengthRadar"] : ["scanGravimetricStrengthBonus", "scanLadarStrengthBonus", "scanMagnetometricStrengthBonus", "scanRadarStrengthBonus"];
            const st = Math.max(...names.map((n) => a[n] ?? 0));
            sum += st * (1 - resist) * factor(s, d) * count(s);
          }
          return sum;
        }
        case "damp_lock_range_pct":
          return (1 - mults("damp", "maxTargetRangeBonus", d)) * 100;
        case "td_optimal_pct":
          return (1 - mults("td", "maxRangeBonus", d)) * 100;
        case "gd_range_pct":
          return (1 - mults("gd", "missileVelocityBonus", d) * mults("gd", "explosionDelayBonus", d)) * 100;
        case "tp_sig_pct":
          return (mults("tp", "signatureRadiusBonus", d) - 1) * 100;
      }
      return null;
    });
  }
  return out;
}
