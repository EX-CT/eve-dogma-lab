/**
 * Dogma operator semantics as data. Order of application = order of this table.
 * kind: 'assign' (high-is-good picks max, else min), 'add' (value or negated), 'mul' (transformed, maybe penalised).
 */
export type OpKind = 'assign' | 'add' | 'mul';
export interface OpDef { code: number; name: string; kind: OpKind; transform: (v: number) => number }

export const OPERATORS: readonly OpDef[] = [
  { code: -1, name: 'PreAssign', kind: 'assign', transform: (v) => v },
  { code: 0, name: 'PreMul', kind: 'mul', transform: (v) => v },
  { code: 1, name: 'PreDiv', kind: 'mul', transform: (v) => (v === 0 ? 1 : 1 / v) },
  { code: 2, name: 'ModAdd', kind: 'add', transform: (v) => v },
  { code: 3, name: 'ModSub', kind: 'add', transform: (v) => -v },
  { code: 4, name: 'PostMul', kind: 'mul', transform: (v) => v },
  { code: 5, name: 'PostDiv', kind: 'mul', transform: (v) => (v === 0 ? 1 : 1 / v) },
  { code: 6, name: 'PostPercent', kind: 'mul', transform: (v) => 1 + v / 100 },
  { code: 7, name: 'PostAssign', kind: 'assign', transform: (v) => v },
];

/** operator code -> position in OPERATORS (codes -1..7) */
export const OP_SLOT = (code: number): number => code + 1;
export const N_OPS = OPERATORS.length;

/** Stacking penalty weight for the i-th strongest modifier: exp(-i²/7.1289) */
export const PENALTY: Float64Array = (() => {
  const a = new Float64Array(64);
  for (let i = 0; i < 64; i++) a[i] = Math.exp(-(i * i) / 7.1289);
  return a;
})();

/** Source categories exempt from stacking penalties: Ship, Charge, Skill, Implant, Subsystem, Structure. */
export const EXEMPT_CATEGORIES = new Set([6, 8, 16, 20, 32, 65]);

/** Rust-compatible rounding (half away from zero). */
export const roundHalfAway = (x: number): number => (x < 0 ? -Math.round(-x) : Math.round(x));
