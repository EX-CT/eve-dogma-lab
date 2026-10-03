// ecm_burst graph (contract 0.2): source ECM-bursts every 30 s, the enemy re-locks after each burst.
// Behaviour per CONTRACT-GRAPHS.md "ecm_burst"; inputs are the source's stats-panel values and item primitives.
import { stackMultiply } from "./math.js";
import type { GraphRequest, Primitives } from "./types.js";

const DAMP_MODULE_EFFECTS = new Set(["remoteSensorDampFalloff", "structureModuleEffectRemoteSensorDampener", "doomsdayAOEDamp"]);
const DAMP_DRONE_EFFECT = "remoteSensorDampEntity";

/** scan-resolution multiplier the source's damps apply to the enemy (one stacking group, range ignored) */
export function dampMultScanRes(p: Primitives): number {
  const l: number[] = [];
  for (const it of p.source.items) {
    const bonus = it.attrs.scanResolutionBonus;
    if (bonus === undefined) continue;
    if (it.kind === "module") {
      if (it.state !== "active" && it.state !== "overheated") continue;
      if (!it.effects.some((e) => DAMP_MODULE_EFFECTS.has(e))) continue;
      l.push(1 + bonus / 100);
    } else if (it.kind === "drone") {
      if (!it.effects.includes(DAMP_DRONE_EFFECT)) continue;
      for (let k = 0; k < (it.active ?? 0); k++) l.push(1 + bonus / 100);
    }
  }
  return stackMultiply(l);
}

export function ecmBurst(req: GraphRequest, p: Primitives): Record<string, (number | null)[]> {
  const prm = req.params ?? {};
  const scanResP: number = prm.tgt_scan_res_mm ?? 700;
  const tgtDpsP: number = prm.tgt_dps ?? 200;
  const adj: number = prm.uptime_adj_s ?? 1;
  const limit = Math.trunc(prm.uptime_amount_limit ?? 3);
  const m = (prm.apply_damps ?? true) ? dampMultScanRes(p) : 1;
  const sig = p.source.ship.attrs.signatureRadius ?? 0;
  const lock = (sr: number) => Math.min(40000 / (sr * m) / Math.asinh(sig) ** 2, 1800);
  const off: any = p.source.stats.offense?.total ?? {};
  const weaponDps: number = off.weapon_dps ?? 0;
  const droneDps: number = (prm.apply_drones ?? true) ? (off.drone_dps ?? 0) + (off.fighter_dps ?? 0) : 0;
  const ehp: number = p.source.stats.defense?.ehp?.total ?? 0;
  const srcDamage = (sr: number, dps: number) => {
    const L = lock(sr);
    const up = Math.max(0, 30 - L - adj);
    const down = 30 - up;
    let rem = ehp;
    let dmg = 0;
    for (let i = 0; i < limit; i++) {
      const alive = down + Math.min(up, rem / dps);
      rem -= up * dps;
      dmg += alive * weaponDps + Math.max(0, alive - 3) * droneDps;
      if (rem <= 0) break;
    }
    return dmg;
  };
  const out: Record<string, (number | null)[]> = {};
  for (const y of req.y) {
    out[y] = req.x.values.map((x) => {
      let sr = scanResP;
      let dps = tgtDpsP;
      if (req.x.axis === "tgt_scan_res_mm") sr = x;
      else dps = x;
      if (!(sr >= 1)) return null;
      if (y === "tgt_lock_time_s") return lock(sr);
      if (y === "tgt_lock_uptime_s") return Math.max(0, 30 - lock(sr));
      if (!(dps > 0)) return null;
      return srcDamage(sr, dps);
    });
  }
  return out;
}
