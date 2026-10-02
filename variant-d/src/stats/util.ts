/** Small pure helpers shared by stats sections (Pyfa-equivalent formulas). */
import { roundHalfAway } from '../core/operators.js';
import type { Resists, Spool } from '../core/request.js';

export function rangeFactor(optimal: number, falloff: number, distance: number | null, restricted: boolean): number {
  if (distance === null || distance === undefined) return 1;
  if (falloff > 0) {
    if (restricted && distance > optimal + 3 * falloff) return 0;
    return Math.pow(0.5, Math.pow(Math.max(distance - optimal, 0) / falloff, 2));
  }
  return distance <= optimal ? 1 : 0;
}

export function lockTime(scanRes: number, sig: number): number | null {
  if (scanRes <= 0 || sig <= 0) return null;
  const a = Math.asinh(sig);
  return Math.min(40000 / scanRes / (a * a), 1800);
}

export const floatUnerr = (v: number): number => roundHalfAway(v * 1e9) / 1e9;

/** Pyfa eos/utils/spoolSupport.calculateSpoolup -> [value, cycles, time] */
export function spoolup(max: number, step: number, cycleS: number, spool: Spool): [number, number, number] {
  if (max === 0 || step === 0) return [0, 0, 0];
  let cycles: number;
  switch (spool.type) {
    case 'spool_scale': cycles = Math.ceil(floatUnerr((max * spool.amount) / step)); break;
    case 'cycle_scale': cycles = roundHalfAway(spool.amount * Math.ceil(floatUnerr(max / step))); break;
    case 'time': cycles = Math.min(Math.floor(floatUnerr(spool.amount / cycleS)), Math.ceil(floatUnerr(max / step))); break;
    default: cycles = Math.min(Math.floor(spool.amount), Math.ceil(floatUnerr(max / step)));
  }
  return [Math.min(cycles * step, max), cycles, cycles * cycleS];
}

export class Dmg {
  constructor(public em = 0, public th = 0, public ki = 0, public ex = 0) {}
  total(): number { return this.em + this.th + this.ki + this.ex; }
  scale(k: number): Dmg { return new Dmg(this.em * k, this.th * k, this.ki * k, this.ex * k); }
  add(o: Dmg): void { this.em += o.em; this.th += o.th; this.ki += o.ki; this.ex += o.ex; }
  clone(): Dmg { return new Dmg(this.em, this.th, this.ki, this.ex); }
  vs(r: Resists): number {
    return this.em * (1 - r.em) + this.th * (1 - r.thermal) + this.ki * (1 - r.kinetic) + this.ex * (1 - r.explosive);
  }
  json() { return { em: this.em, thermal: this.th, kinetic: this.ki, explosive: this.ex, total: this.total() }; }
}

const round6 = (v: number): number => (Number.isFinite(v) ? roundHalfAway(v * 1e6) / 1e6 : v);

/** Recursively round floats to 1e-6 and sort object keys (stable, byte-identical output). */
export function tidy(v: unknown): unknown {
  if (typeof v === 'number') return Number.isInteger(v) ? v : round6(v);
  if (Array.isArray(v)) return v.map(tidy);
  if (v !== null && typeof v === 'object') {
    const o = v as Record<string, unknown>;
    const out: Record<string, unknown> = {};
    for (const k of Object.keys(o).sort()) if (o[k] !== undefined) out[k] = tidy(o[k]);
    return out;
  }
  return v;
}
