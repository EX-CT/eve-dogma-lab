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

/**
 * Python round(x, n) for n >= 0: correctly rounded on the exact binary value, ties to even. A tie needs
 * x * 2^(n+1) to be an integer; toFixed (exact, ties away from zero) does the rest.
 */
const POW10 = [1, 10, 100, 1000, 1e4, 1e5, 1e6];
export function pyRound(x: number, n: number): number {
  if (!Number.isFinite(x)) return x;
  if (n < 0) { const p = Math.pow(10, -n); return roundHalfAway(x / p) * p; }
  if (n > 99) return x;
  if (n <= 6) {
    // fast path away from ties: the nearest k / 10^n is unambiguous, and k / 10^n is the correctly rounded double
    const p = POW10[n], y = x * p, f = Math.abs(y - Math.trunc(y));
    if (Math.abs(f - 0.5) > 1e-6 && Math.abs(y) < 2 ** 52) return roundHalfAway(y) / p;
  }
  // an exact tie has at most n + 1 binary fraction digits (x * 2^(n+1) integral, exact), so toFixed(n + 1) is exact
  const t = Number.isInteger(x * Math.pow(2, n + 1)) ? x.toFixed(n + 1) : '';
  if (t.endsWith('5')) {
    const toZero = t.slice(0, -1);
    const last = toZero.charCodeAt(toZero.length - 1) === 46 ? toZero.charCodeAt(toZero.length - 2) : toZero.charCodeAt(toZero.length - 1);
    if ((last - 48) % 2 === 0) return Number(toZero);
  }
  return Number(x.toFixed(n));
}

/** Pyfa eos.utils.float.floatUnerr: round(value, 7 - ceil(log10(|value|))) */
export function pyFloatUnerr(v: number): number {
  if (v === 0 || v === Infinity) return v;
  return pyRound(v, Math.trunc(7 - Math.ceil(Math.log10(Math.abs(v)))));
}
