/**
 * Variant-D dataset cache ("VDC1"): the same dataset, re-laid-out for fast cold start.
 * - header JSON: sde info, sha256, attributes, effects, groups, dbuffs, mutaplasmids, compact type headers
 * - body: concatenated per-type JSON `[attrs, effects]` and the zh-name table, decoded lazily on first access.
 * Built from the official dataset only (no other data); pure function of it.
 */
import { Dataset, TypeInfo } from './dataset.js';

const MAGIC = 'VDC1';

export function buildCache(raw: any, sha256: string): string {
  const bodyParts: string[] = [];
  let off = 0;
  const add = (s: string) => { const o = off; bodyParts.push(s); off += s.length; return [o, s.length]; };
  const types: unknown[] = [];
  const ids = Object.keys(raw.types).map(Number).sort((a, b) => a - b);
  for (const id of ids) {
    const t = raw.types[id];
    const [o, l] = add(JSON.stringify([t.attrs ?? {}, t.effects ?? []]));
    types.push([id, t.name ?? '', t.group, t.category, t.published ? 1 : 0, t.mass ?? 0, t.volume ?? 0, t.capacity ?? 0, t.radius ?? 0,
      t.market_group ?? null, t.meta_group ?? null, t.meta_level ?? null, t.variation_parent ?? null, o, l]);
  }
  const zh = add(JSON.stringify(raw.names?.zh ?? {}));
  const header = JSON.stringify({
    format: raw.format, format_version: raw.format_version, sde: raw.sde, sha256,
    attributes: raw.attributes, effects: raw.effects, groups: raw.groups, dbuffs: raw.dbuffs ?? {}, mutaplasmids: raw.mutaplasmids ?? {},
    types, zh,
  });
  return `${MAGIC}\n${header.length}\n${header}${bodyParts.join('')}`;
}

class LazyType implements TypeInfo {
  private body: [Record<string, number>, [number, number][]] | null = null;
  constructor(
    private src: string, private off: number, private len: number,
    public id: number, public name: string, public group: number, public category: number, public published: boolean,
    public mass: number, public volume: number, public capacity: number, public radius: number,
    public marketGroup: number | null, public metaGroup: number | null, public metaLevel: number | null, public variationParent: number | null,
  ) {}
  private decode() {
    if (this.body === null) this.body = JSON.parse(this.src.slice(this.off, this.off + this.len));
    return this.body!;
  }
  get rawAttrs() { return this.decode()[0]; }
  get effects() { return this.decode()[1]; }
}

export function isCache(text: string): boolean {
  return text.startsWith(MAGIC + '\n');
}

export function datasetFromCache(text: string): Dataset {
  if (!isCache(text)) throw new Error('not a VDC1 cache');
  const nl = text.indexOf('\n', MAGIC.length + 1);
  const hlen = Number(text.slice(MAGIC.length + 1, nl));
  const h = JSON.parse(text.slice(nl + 1, nl + 1 + hlen));
  if (h.format !== 'exct-eve-dataset' || h.format_version !== 1) throw new Error(`unsupported dataset format ${h.format} v${h.format_version}`);
  const body = nl + 1 + hlen;
  const ds = new Dataset();
  ds.sha256 = h.sha256;
  ds.initCommon(h);
  for (const r of h.types as any[]) {
    ds.addType(r[0], new LazyType(text, body + r[13], r[14], r[0], r[1], r[2], r[3], r[4] === 1, r[5], r[6], r[7], r[8], r[9], r[10], r[11], r[12]));
  }
  ds.finishTypes();
  const [zo, zl] = h.zh;
  ds.zhSource = () => JSON.parse(text.slice(body + zo, body + zo + zl));
  return ds;
}
