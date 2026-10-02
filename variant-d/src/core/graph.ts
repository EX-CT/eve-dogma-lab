/**
 * AttrGraph: items + lazily materialised attribute cells with pull-based, memoised evaluation.
 *
 * - Base values: item.base (mutations/overrides/skill level/...) -> type attribute map -> attribute default.
 * - A Cell exists only for attributes that are targeted by at least one modifier.
 * - Cache validity is an epoch number: invalidate() is O(1).
 */
import { Dataset } from './dataset.js';
import { OPERATORS, N_OPS, OP_SLOT, PENALTY } from './operators.js';
import { SlotName, Spool, State } from './request.js';

export const enum Kind { Ship, Char, Skill, Module, Charge, Drone, Fighter, Implant, Booster, Mode, Beacon, Projected }
export const enum Loc { Ship, Char, Space, Nowhere }

/** Modifier source kinds */
export const enum SrcK { Attr = 0, Const = 1, Prop = 2, Projected = 3 }

/** One modifier on a cell. Single monomorphic shape for all source kinds (V8-friendly). */
export interface Mod {
  op: number;
  pen: boolean;
  k: SrcK;
  /** Attr/Projected: source item; Prop: module */
  item: number;
  /** Attr/Projected: source attribute; Prop: speedFactor */
  attr: number;
  /** Const: value; Projected: range factor */
  v: number;
  /** Prop: thrust attr; Projected: resist attr */
  a2: number;
  /** Prop: mass attr; Projected: target item */
  a3: number;
  /** Projected: multiplicative (scale (v-1)) */
  mul: boolean;
  /** item that caused this modifier (provenance) */
  src: number;
}

export interface Cell { base: number; mods: Mod[]; val: number; epoch: number; busy: boolean }

export interface Item {
  idx: number;
  typeId: number;
  group: number;
  category: number;
  kind: Kind;
  state: State;
  loc: Loc;
  owned: boolean;
  parent: number;
  charge: number;
  slot: SlotName | null;
  reqIndex: number | null;
  quantity: number;
  activeCount: number;
  /** base values that differ from the type (mutations, overrides, skill level, security modifier) */
  base: Map<number, number> | null;
  /** single inline base override (fast path: skill level etc.); -1 = unused */
  ovA: number;
  ovV: number;
  /** type attribute map (shared, immutable) */
  tattrs: Map<number, number>;
  cells: Map<number, Cell> | null;
  reqSkills: number[];
  effects: [number, number][];
  fighterAbilities: number[] | null;
  boosterSideEffects: number[];
  spool: Spool | null;
  distance: number | null;
}

/** attributes whose value is post-processed even without modifiers */
interface AttrPost { min: number | null; max: number | null; round2: boolean; highIsGood: boolean }
/** per-dataset memo of AttrPost by attribute id (shared by all fits of a dataset) */
interface PostTable { post: (AttrPost | null)[]; known: Uint8Array }
const POST = new WeakMap<Dataset, PostTable>();

export class AttrGraph {
  items: Item[] = [];
  epoch = 1;
  private pt: PostTable;

  constructor(public ds: Dataset) {
    let pt = POST.get(ds);
    if (pt === undefined) POST.set(ds, (pt = { post: [], known: new Uint8Array(0) }));
    this.pt = pt;
  }

  // ------------------------------------------------------------------ base values
  base(i: number, a: number): number {
    const it = this.items[i];
    if (it.ovA === a) return it.ovV;
    if (it.base !== null) {
      const v = it.base.get(a);
      if (v !== undefined) return v;
    }
    const v = it.tattrs.get(a);
    return v !== undefined ? v : this.ds.attrDefault(a);
  }

  has(i: number, a: number): boolean {
    const it = this.items[i];
    return it.ovA === a || (it.base !== null && it.base.has(a)) || it.tattrs.has(a) || (it.cells !== null && it.cells.has(a));
  }

  setBase(i: number, a: number, v: number): void {
    const it = this.items[i];
    if (it.ovA === a || it.ovA === -1) {
      it.ovA = a;
      it.ovV = v;
    } else {
      if (it.base === null) it.base = new Map();
      it.base.set(a, v);
    }
    if (it.cells !== null) {
      const c = it.cells.get(a);
      if (c) c.base = v;
    }
  }

  // ------------------------------------------------------------------ modifiers
  cell(i: number, a: number): Cell {
    const it = this.items[i];
    if (it.cells === null) it.cells = new Map();
    let c = it.cells.get(a);
    if (c === undefined) {
      c = { base: this.base(i, a), mods: [], val: NaN, epoch: 0, busy: false }; // base() already falls back to the default
      it.cells.set(a, c);
    }
    return c;
  }

  addMod(target: number, attr: number, m: Mod): void {
    this.cell(target, attr).mods.push(m);
  }

  invalidate(): void {
    this.epoch++;
  }

  // ------------------------------------------------------------------ evaluation
  private attrPost(a: number): AttrPost | null {
    const pt = this.pt;
    if (a < pt.known.length && pt.known[a]) return pt.post[a];
    if (a >= pt.known.length) {
      const n = new Uint8Array(Math.max(a + 1, pt.known.length * 2, 4096));
      n.set(pt.known);
      pt.known = n;
    }
    const info = this.ds.attrs.get(a);
    let p: AttrPost | null = null;
    if (info) {
      const round2 = info.name === 'cpu' || info.name === 'power' || info.name === 'cpuOutput' || info.name === 'powerOutput';
      p = { min: info.minAttr, max: info.maxAttr, round2, highIsGood: info.highIsGood };
    }
    pt.post[a] = p;
    pt.known[a] = 1;
    return p;
  }

  /** Modified value; missing attribute -> attribute default (no post-processing), like eve-dogma-rs. */
  get(i: number, a: number): number {
    const it = this.items[i];
    const c = it.cells !== null ? it.cells.get(a) : undefined;
    if (c !== undefined) return this.evalCell(i, a, c);
    if (!this.has(i, a)) return this.ds.attrDefault(a);
    const v = this.base(i, a);
    const p = this.attrPost(a);
    return p === null ? v : this.finish(i, v, p);
  }

  getOpt(i: number, a: number): number | null {
    return this.has(i, a) ? this.get(i, a) : null;
  }

  private finish(i: number, val: number, p: AttrPost): number {
    if (p.min !== null) val = Math.max(val, this.get(i, p.min));
    if (p.max !== null) val = Math.min(val, this.get(i, p.max));
    if (p.round2) val = Math.round(val * 100) / 100;
    return val;
  }

  private srcValue(m: Mod): number {
    switch (m.k) {
      case SrcK.Attr:
        return this.get(m.item, m.attr);
      case SrcK.Const:
        return m.v;
      case SrcK.Prop: {
        const mass = this.get(m.a3, 4);
        if (mass === 0) return 1;
        return 1 + (this.get(m.item, m.attr) / 100) * this.get(m.item, m.a2) / mass;
      }
      case SrcK.Projected: {
        let f = m.v;
        if (m.a2 !== 0) f *= this.get(m.a3, m.a2);
        const v = this.get(m.item, m.attr);
        return m.mul ? (v - 1) * f + 1 : v * f;
      }
    }
  }

  private evalCell(i: number, a: number, c: Cell): number {
    if (c.epoch === this.epoch) return c.val;
    if (c.busy) return c.base; // dogma cycle guard
    c.busy = true;
    const p = this.attrPost(a);
    let val = c.base;
    const mods = c.mods;
    const n = mods.length;
    if (n > 0) {
      // pull every source value once, remember which operator slots are present
      const vals = new Float64Array(n);
      let present = 0;
      for (let j = 0; j < n; j++) {
        const s = OP_SLOT(mods[j].op);
        if (s < 0 || s >= N_OPS) continue;
        present |= 1 << s;
        vals[j] = this.srcValue(mods[j]);
      }
      const hig = p === null ? true : p.highIsGood;
      for (let s = 0; s < N_OPS; s++) {
        if ((present & (1 << s)) === 0) continue;
        const def = OPERATORS[s];
        const code = def.code;
        if (def.kind === 'assign') {
          let x = NaN;
          for (let j = 0; j < n; j++) {
            if (mods[j].op !== code) continue;
            const v = vals[j];
            x = x !== x ? v : hig ? Math.max(x, v) : Math.min(x, v);
          }
          val = x;
        } else if (def.kind === 'add') {
          for (let j = 0; j < n; j++) if (mods[j].op === code) val += def.transform(vals[j]);
        } else {
          let pos: number[] | null = null;
          let neg: number[] | null = null;
          for (let j = 0; j < n; j++) {
            const m = mods[j];
            if (m.op !== code) continue;
            const mv = def.transform(vals[j]);
            if (m.pen) {
              if (mv > 1) (pos ??= []).push(mv);
              else if (mv < 1) (neg ??= []).push(mv);
            } else val *= mv;
          }
          if (pos !== null) val = applyPenalised(val, pos);
          if (neg !== null) val = applyPenalised(val, neg);
        }
      }
    }
    if (p !== null) val = this.finish(i, val, p);
    c.busy = false;
    c.val = val;
    c.epoch = this.epoch;
    return val;
  }
}

function applyPenalised(val: number, list: number[]): number {
  if (list.length > 1) list.sort((x, y) => Math.abs(y - 1) - Math.abs(x - 1));
  for (let i = 0; i < list.length; i++) {
    const w = i < PENALTY.length ? PENALTY[i] : 0;
    val *= 1 + (list[i] - 1) * w;
  }
  return val;
}
