// Closed-form graphs: lock_time, warp_time, mobility, shield_regen, capacitor (regen-only part).
// Formulas from public sources (EVE University wiki: warp, lock time, alignment; CCP capacitor/shield recharge).
import { AU, finite } from "./math.js";
import type { FitPrim, GraphRequest, Primitives } from "./types.js";
import { GraphError } from "./types.js";

type Series = Record<string, (number | null)[]>;
const nav = (p: FitPrim) => p.stats.navigation;

export function lockTime(req: GraphRequest, p: Primitives): Series {
  const scanRes = p.source.ship.attrs.scanResolution ?? 0;
  const f = (sig: number): number | null => {
    if (!(sig >= 1)) return null;
    if (scanRes <= 0) return (p.source.ship.attrs.scanSpeed ?? 0) / 1000;
    return Math.min(40000 / scanRes / Math.asinh(sig) ** 2, 1800);
  };
  return { time_s: req.x.values.map(f) };
}

/** Time in warp (EVE University model): exponential acceleration (k = warp speed AU/s) and deceleration
 *  (k = min(warp/3, 2)) phases, cruise in between, dropping out at min(subwarp/2, 100) m/s. */
export function warpTimeAt(distance: number, warpAuS: number, subwarp: number): number {
  if (distance <= 0) return 0;
  const kA = warpAuS;
  const kD = Math.min(warpAuS / 3, 2);
  const vWarp = warpAuS * AU;
  const vDrop = Math.min(subwarp / 2, 100);
  const dAccel = AU; // = vWarp / kA
  const dDecel = vWarp / kD;
  if (distance > dAccel + dDecel) {
    const tA = Math.log(vWarp / kA) / kA;
    const tD = Math.log(vWarp / vDrop) / kD;
    return tA + (distance - dAccel - dDecel) / vWarp + tD;
  }
  const vMax = (distance * kA * kD) / (kA + kD);
  const tA = Math.log(vMax / kA) / kA;
  const tD = Math.log(vMax / vDrop) / kD;
  return tA + tD;
}

export function warpTime(req: GraphRequest, p: Primitives): Series {
  const n = nav(p.source);
  const maxD = n.max_warp_distance_au * AU;
  const sub = p.subwarp_speed ?? n.max_velocity;
  return { time_s: req.x.values.map((d) => (d < 0 || d > maxD + 1e-6 ? null : finite(warpTimeAt(d, n.warp_speed_au_s, sub)))) };
}

export function mobility(req: GraphRequest, p: Primitives): Series {
  const n = nav(p.source);
  const v = n.max_velocity;
  const mass = n.mass;
  const k = (n.agility * mass) / 1e6; // time constant, s
  const prm = req.params ?? {};
  const tMass = (prm.tgt_mass_kg ?? 1.3e9) / 1e6;
  const tInertia = prm.tgt_inertia ?? 0.015;
  const out: Series = {};
  const speed = (t: number) => (k > 0 ? v * (1 - Math.exp(-t / k)) : v);
  for (const y of req.y) {
    out[y] = req.x.values.map((t) => {
      if (t < 0) return null;
      const s = speed(t);
      switch (y) {
        case "speed_mps":
          return s;
        case "distance_m":
          return k > 0 ? v * (t + k * (Math.exp(-t / k) - 1)) : v * t;
        case "momentum_kg_mps":
          return s * mass;
        case "bump_speed_mps": {
          const ms = mass / 1e6;
          return (2 * s * ms) / (ms + tMass);
        }
        case "bump_distance_m": {
          const ms = mass / 1e6;
          return ((2 * s * ms) / (ms + tMass)) * tMass * tInertia;
        }
      }
      throw new GraphError("BAD_AXIS", `mobility has no series ${y}`, "y");
    });
  }
  return out;
}

/** Recharge curve shared by capacitor and shield: level after t seconds starting at c0, and regen rate. */
export function regenLevel(C: number, tauS: number, c0: number, t: number): number {
  if (C <= 0) return 0;
  return C * (1 + Math.exp((-5 * t) / tauS) * (Math.sqrt(c0 / C) - 1)) ** 2;
}
export function regenRate(C: number, tauS: number, c: number): number {
  if (C <= 0 || tauS <= 0) return 0;
  const r = c / C;
  return ((10 * C) / tauS) * (Math.sqrt(r) - r);
}

function effectivifyShield(p: FitPrim, dp: any): (v: number) => number {
  const a = p.ship.attrs;
  const r = [a.shieldEmDamageResonance, a.shieldThermalDamageResonance, a.shieldKineticDamageResonance, a.shieldExplosiveDamageResonance];
  const pat = dp ? [dp.em, dp.thermal, dp.kinetic, dp.explosive] : [25, 25, 25, 25];
  const tot = pat.reduce((s, x) => s + x, 0) || 1;
  const div = pat.reduce((s, x, i) => s + x * r[i], 0) / tot;
  return (v) => (div === 0 ? v : v / div);
}

export function shieldRegen(req: GraphRequest, p: Primitives): Series {
  const a = p.source.ship.attrs;
  const C = a.shieldCapacity ?? 0;
  const tau = (a.shieldRechargeRate ?? 0) / 1000;
  const prm = req.params ?? {};
  const eff = prm.effective ? effectivifyShield(p.source, req.fit?.damage_pattern) : (v: number) => v;
  const c0 = ((prm.shield_start_pct ?? 0) / 100) * C;
  const out: Series = {};
  const level = (x: number): number | null => {
    if (req.x.axis === "time_s") return x < 0 ? null : regenLevel(C, tau, c0, x);
    if (req.x.axis === "shield_pct") return x < 0 || x > 100 ? null : (x / 100) * C;
    throw new GraphError("BAD_AXIS", `shield_regen has no x axis ${req.x.axis}`, "x.axis");
  };
  for (const y of req.y) {
    out[y] = req.x.values.map((x) => {
      const c = level(x);
      if (c === null) return null;
      if (y === "shield_hp") return eff(c);
      if (y === "shield_regen_hp_s") return eff(regenRate(C, tau, c));
      throw new GraphError("BAD_AXIS", `shield_regen has no series ${y}`, "y");
    });
  }
  return out;
}
