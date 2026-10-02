/**
 * Variant-D dataset cache ("VDC4"): the same dataset, re-laid-out for fast cold start. Platform-neutral (bytes in).
 *   "VDC4\n" <header byte length> "\n" <header JSON> <body bytes>
 * - header: sde info, sha256, attributes, groups, categories, dbuffs, an effect index and the types as
 *   *columns* (ids, group, category, published, mass, ... and byte offset/length of each type's body slice)
 * - body: per-type JSON `[attrs, effects]` slices, the type-name array, per-effect JSON slices (decoded on first
 *   lookup), the mutaplasmid table and the zh-name table.
 * Nothing per type is allocated at load: TypeInfo objects are created on first access (TypeTable) and their
 * attrs/effects decoded on first use (memoised, so the effects array keeps its identity for the plan WeakMap).
 */
import { Dataset, EffectInfo, EffectStore, TypeInfo, TypeStore, effectInfo } from './dataset.js';

const MAGIC = 'VDC4';
const enc = new TextEncoder();

export function buildCache(raw: any, sha256: string): Uint8Array {
  const parts: Uint8Array[] = [];
  let off = 0;
  const add = (s: string): [number, number] => { const b = enc.encode(s); const o = off; parts.push(b); off += b.length; return [o, b.length]; };
  const ids = Object.keys(raw.types).map(Number).sort((a, b) => a - b);
  const col = (f: (t: any) => unknown) => ids.map((id) => f(raw.types[id]));
  const offs: number[] = [], lens: number[] = [];
  for (const id of ids) {
    const t = raw.types[id];
    const [o, l] = add(JSON.stringify([t.attrs ?? {}, t.effects ?? []]));
    offs.push(o); lens.push(l);
  }
  const names = add(JSON.stringify(col((t) => t.name ?? '')));
  // effects: one JSON slice each, decoded on first lookup; mutaplasmids: one slice, decoded on first use
  const eids = Object.keys(raw.effects).map(Number).sort((a, b) => a - b);
  const eoff: number[] = [], elen: number[] = [];
  for (const id of eids) { const [o, l] = add(JSON.stringify(raw.effects[id])); eoff.push(o); elen.push(l); }
  const muta = add(JSON.stringify(raw.mutaplasmids ?? {}));
  const zh = add(JSON.stringify(raw.names?.zh ?? {}));
  const header = enc.encode(JSON.stringify({
    format: raw.format, format_version: raw.format_version, sde: raw.sde, sha256,
    attributes: raw.attributes, groups: raw.groups, categories: raw.categories ?? {},
    dbuffs: raw.dbuffs ?? {}, muta,
    effect_index: { id: eids, name: eids.map((id) => raw.effects[id].name), off: eoff, len: elen },
    types: {
      id: ids, group: col((t) => t.group), category: col((t) => t.category), published: col((t) => (t.published ? 1 : 0)),
      mass: col((t) => t.mass ?? 0), volume: col((t) => t.volume ?? 0), capacity: col((t) => t.capacity ?? 0), radius: col((t) => t.radius ?? 0),
      market_group: col((t) => t.market_group ?? null), meta_group: col((t) => t.meta_group ?? null),
      meta_level: col((t) => t.meta_level ?? null), variation_parent: col((t) => t.variation_parent ?? null),
      off: offs, len: lens,
    },
    names, zh,
  }));
  const pre = enc.encode(`${MAGIC}\n${header.length}\n`);
  const out = new Uint8Array(pre.length + header.length + off);
  out.set(pre, 0);
  out.set(header, pre.length);
  let p = pre.length + header.length;
  for (const b of parts) { out.set(b, p); p += b.length; }
  return out;
}

interface Cols {
  id: number[]; group: number[]; category: number[]; published: number[]; mass: number[]; volume: number[]; capacity: number[]; radius: number[];
  market_group: (number | null)[]; meta_group: (number | null)[]; meta_level: (number | null)[]; variation_parent: (number | null)[];
  off: number[]; len: number[];
}

class LazyType implements TypeInfo {
  private body: [Record<string, number>, [number, number][]] | null = null;
  constructor(private tab: TypeTable, private row: number, public id: number) {}
  get name(): string { return this.tab.name(this.row); }
  get group() { return this.tab.c.group[this.row]; }
  get category() { return this.tab.c.category[this.row]; }
  get published() { return this.tab.c.published[this.row] === 1; }
  get mass() { return this.tab.c.mass[this.row]; }
  get volume() { return this.tab.c.volume[this.row]; }
  get capacity() { return this.tab.c.capacity[this.row]; }
  get radius() { return this.tab.c.radius[this.row]; }
  get marketGroup() { return this.tab.c.market_group[this.row]; }
  get metaGroup() { return this.tab.c.meta_group[this.row]; }
  get metaLevel() { return this.tab.c.meta_level[this.row]; }
  get variationParent() { return this.tab.c.variation_parent[this.row]; }
  private decode() {
    if (this.body === null) this.body = JSON.parse(this.tab.slice(this.tab.c.off[this.row], this.tab.c.len[this.row]));
    return this.body!;
  }
  get rawAttrs() { return this.decode()[0]; }
  get effects() { return this.decode()[1]; }
}

class TypeTable implements TypeStore {
  private objs: (LazyType | undefined)[];
  rowOf = new Map<number, number>();
  private names: string[] | null = null;
  private dec = new TextDecoder();
  constructor(readonly c: Cols, public bytes: Uint8Array | null, private body: number, private namesAt: [number, number]) {
    this.objs = new Array(c.id.length);
    this.indexRows();
  }
  /** id -> row map (rebuilt after a snapshot instead of being stored in it) */
  indexRows(): void {
    const id = this.c.id, m = new Map<number, number>();
    for (let r = 0; r < id.length; r++) m.set(id[r], r);
    this.rowOf = m;
  }
  slice(off: number, len: number): string { return this.dec.decode(this.bytes!.subarray(this.body + off, this.body + off + len)); }
  name(row: number): string {
    if (this.names === null) this.names = JSON.parse(this.slice(this.namesAt[0], this.namesAt[1]));
    return this.names![row];
  }
  get size() { return this.c.id.length; }
  private at(r: number): LazyType {
    return (this.objs[r] ??= new LazyType(this, r, this.c.id[r]));
  }
  get(id: number): TypeInfo | undefined { const r = this.rowOf.get(id); return r === undefined ? undefined : this.at(r); }
  has(id: number): boolean { return this.rowOf.has(id); }
  nameIndex(): Map<string, number> {
    const m = new Map<string, number>();
    const id = this.c.id, pub = this.c.published, rowOf = this.rowOf;
    for (let r = 0; r < id.length; r++) {
      const key = this.name(r).toLowerCase();
      const prev = m.get(key);
      if (prev === undefined || (pub[r] && !pub[rowOf.get(prev)!])) m.set(key, id[r]);
    }
    return m;
  }
  idsInGroup(group: number): number[] {
    const out: number[] = [];
    const g = this.c.group, id = this.c.id;
    for (let r = 0; r < g.length; r++) if (g[r] === group) out.push(id[r]);
    return out;
  }
  *[Symbol.iterator](): IterableIterator<[number, TypeInfo]> {
    for (let r = 0; r < this.c.id.length; r++) yield [this.c.id[r], this.at(r)];
  }
}

/** effect table decoded per effect on first lookup */
class LazyEffects implements EffectStore {
  private rowOf = new Map<number, number>();
  private objs: (EffectInfo | undefined)[];
  constructor(private tab: TypeTable, ids: number[], private off: number[], private len: number[]) {
    for (let r = 0; r < ids.length; r++) this.rowOf.set(ids[r], r);
    this.objs = new Array(ids.length);
  }
  get size() { return this.objs.length; }
  get(id: number): EffectInfo | undefined {
    const r = this.rowOf.get(id);
    if (r === undefined) return undefined;
    return (this.objs[r] ??= effectInfo(id, JSON.parse(this.tab.slice(this.off[r], this.len[r]))));
  }
}

const MAGIC_BYTES = enc.encode(MAGIC + '\n');
export function isCache(bytes: Uint8Array): boolean {
  if (bytes.length < MAGIC_BYTES.length) return false;
  for (let i = 0; i < MAGIC_BYTES.length; i++) if (bytes[i] !== MAGIC_BYTES[i]) return false;
  return true;
}

export function datasetFromCache(bytes: Uint8Array): Dataset {
  if (!isCache(bytes)) throw new Error(`not a ${MAGIC} cache`);
  const dec = new TextDecoder();
  const nl = bytes.indexOf(10, MAGIC_BYTES.length);
  const hlen = Number(dec.decode(bytes.subarray(MAGIC_BYTES.length, nl)));
  const h = JSON.parse(dec.decode(bytes.subarray(nl + 1, nl + 1 + hlen)));
  if (h.format !== 'exct-eve-dataset' || h.format_version !== 1) throw new Error(`unsupported dataset format ${h.format} v${h.format_version}`);
  const body = nl + 1 + hlen;
  const ds = new Dataset();
  ds.sha256 = h.sha256;
  ds.initCommon(h);
  const c: Cols = h.types;
  const tab = new TypeTable(c, bytes, body, h.names);
  ds.types = tab;
  for (let r = 0; r < c.id.length; r++) if (c.category[r] === 16) ds.skills.push(c.id[r]);
  ds.finishTypes();
  const ei = h.effect_index;
  ds.effects = new LazyEffects(tab, ei.id, ei.off, ei.len);
  ds.setEffectNames(ei.id, ei.name);
  // closures capture only offsets: capturing `h` would keep the whole parsed header (raw attributes etc.) alive
  const [mo, ml] = h.muta;
  ds.mutaSource = () => JSON.parse(tab.slice(mo, ml));
  const [zo, zl] = h.zh;
  ds.zhSource = () => JSON.parse(tab.slice(zo, zl));
  return ds;
}

/**
 * Startup snapshots: drop / re-attach the body bytes of a cache-loaded dataset (the snapshot keeps the decoded
 * header; the body is read from the same cache file at run time). attach checks the file still is that cache.
 */
export function detachCacheBytes(ds: Dataset): { size: number; head: number[] } | null {
  const tab = ds.types;
  if (!(tab instanceof TypeTable) || tab.bytes === null) return null;
  const b = tab.bytes;
  tab.bytes = null;
  tab.rowOf = new Map();
  // numeric columns without nulls as typed arrays: same values, fewer bytes in the snapshot
  const c = tab.c as unknown as Record<string, ArrayLike<number | null>>;
  for (const k of Object.keys(c)) {
    const col = c[k];
    if (!Array.isArray(col) || col.some((v) => typeof v !== 'number')) continue;
    c[k] = (col as number[]).every((v) => (v | 0) === v && !Object.is(v, -0)) ? Int32Array.from(col as number[]) : Float64Array.from(col as number[]);
  }
  return { size: b.length, head: Array.from(b.subarray(0, Math.min(b.length, 4096))) };
}
export function attachCacheBytes(ds: Dataset, bytes: Uint8Array, sig: { size: number; head: number[] }): boolean {
  const tab = ds.types;
  if (!(tab instanceof TypeTable) || bytes.length !== sig.size) return false;
  for (let i = 0; i < sig.head.length; i++) if (bytes[i] !== sig.head[i]) return false;
  tab.bytes = bytes;
  tab.indexRows();
  return true;
}
