// Shared numeric helpers (public formulas; no Pyfa code).
import type { StackInputs } from "./types.js";

export const AU = 149597870700;

/** Round away float noise the way the reference does for comparisons (9 decimals). */
export function floatUnerr(v: number): number {
  return Math.round(v * 1e9) / 1e9;
}

/** Turret-style range factor: 1 inside optimal, 0.5^((d-opt)/falloff)^2 beyond; restricted = 0 past opt + 3 falloff. */
export function rangeFactor(optimal: number, falloff: number, distance: number | null, restricted = true): number {
  if (distance === null) return 1;
  if (distance <= optimal) return 1;
  if (restricted && distance > optimal + 3 * falloff) return 0;
  if (falloff <= 0) return 0;
  return Math.pow(0.5, Math.pow((distance - optimal) / falloff, 2));
}

export const PENALTY = (k: number) => Math.exp(-(k * k) / 7.1289);

/** Stacking-penalised product of multipliers: positive and negative chains, strongest first. */
export function stackMultiply(mults: number[]): number {
  const pos = mults.filter((m) => m > 1).sort((a, b) => Math.abs(b - 1) - Math.abs(a - 1));
  const neg = mults.filter((m) => m < 1).sort((a, b) => Math.abs(b - 1) - Math.abs(a - 1));
  let v = 1;
  for (const l of [pos, neg]) l.forEach((m, k) => (v *= 1 + (m - 1) * PENALTY(k)));
  return v;
}

/**
 * Re-fold an attribute from its stacking inputs with extra stacking-penalised multipliers (op 6 chain):
 * CCP operator order, penalised chains per operator, same as the engine's fold.
 */
export function foldExtended(s: StackInputs, extraMults: number[]): number {
  let val = s.base;
  for (let op = -1; op <= 7; op++) {
    const ms = s.mods.filter((m) => m.op === op);
    const extra = op === 6 ? extraMults : [];
    if (!ms.length && !extra.length) continue;
    let assign: number | null = null;
    const pos: number[] = [];
    const neg: number[] = [];
    for (const m of ms) {
      const v = m.value;
      if (op === -1 || op === 7) {
        assign = assign === null ? v : s.high_is_good ? Math.max(assign, v) : Math.min(assign, v);
      } else if (op === 2) val += v;
      else if (op === 3) val -= v;
      else {
        let x = 1;
        if (op === 0 || op === 4) x = v;
        else if (op === 1 || op === 5) x = v === 0 ? 1 : 1 / v;
        else if (op === 6) x = 1 + v / 100;
        if (m.penalized) {
          if (x > 1) pos.push(x);
          else if (x < 1) neg.push(x);
        } else val *= x;
      }
    }
    for (const x of extra) {
      if (x > 1) pos.push(x);
      else if (x < 1) neg.push(x);
    }
    if (assign !== null) val = assign;
    for (const l of [pos, neg]) {
      // stable: strongest first
      const idx = l.map((v, i) => [v, i] as [number, number]).sort((a, b) => Math.abs(b[0] - 1) - Math.abs(a[0] - 1) || a[1] - b[1]);
      idx.forEach(([m], k) => (val *= 1 + (m - 1) * PENALTY(k)));
    }
  }
  if (s.min !== undefined) val = Math.max(val, s.min);
  if (s.max !== undefined) val = Math.min(val, s.max);
  return val;
}

export function finite(v: number | null | undefined): number | null {
  return v === null || v === undefined || !Number.isFinite(v) ? null : v;
}
