/**
 * @exct/eve-dogma-ts (Variant D) public API. Pure: no I/O, clocks or global mutable state.
 *   const ds = Dataset.fromJson(parsedJson, sha256);  const stats = calc(ds, request);
 */
import { Dataset } from './core/dataset.js';
import { EngineError, Fit, inferSlot } from './core/fit.js';
import { FitRequest, normalize, RequestError } from './core/request.js';
import { computeStats, ENGINE } from './stats/index.js';
import { exportEft, parseEft } from './formats/eft.js';

export { Dataset, ENGINE, parseEft, exportEft, inferSlot };
export type { FitRequest };

const err = (code: string, message: string, path = '') => ({ error: { code, message, path } });

/** FitRequest object -> FitStats object (or {error}) */
export function calc(ds: Dataset, request: unknown): any {
  let req;
  try {
    req = normalize(request);
  } catch (e) {
    if (e instanceof RequestError) return err('BAD_REQUEST', e.message, e.path);
    throw e;
  }
  try {
    return computeStats(Fit.build(ds, req), req);
  } catch (e) {
    if (e instanceof EngineError) return err(e.code, e.message, e.path);
    throw e;
  }
}

/** JSON string in, JSON string out (contract `calc_json`). */
export function calcJson(ds: Dataset, requestJson: string): string {
  let v: unknown;
  try {
    v = JSON.parse(requestJson);
  } catch (e) {
    return JSON.stringify(err('BAD_JSON', (e as Error).message));
  }
  return JSON.stringify(calc(ds, v));
}

const KIND_BY_CAT: Record<number, string> = { 6: 'ship', 7: 'module', 8: 'charge', 18: 'drone', 87: 'fighter', 20: 'implant', 32: 'subsystem', 16: 'skill' };

/** kind of a type for search (contract 1.4.1 interim spec); null = not searchable */
export function typeKind(ds: Dataset, id: number): string | null {
  const t = ds.types.get(id);
  if (!t) return null;
  const k = KIND_BY_CAT[t.category];
  if (k === 'implant' && (ds.groups.get(t.group)?.name ?? '').includes('Booster')) return 'booster';
  return k ?? null;
}

/**
 * search (contract 1.4.1 interim): published ship/module/charge/drone/fighter/implant/booster/subsystem/skill types,
 * case-insensitive on English or Chinese name; exact > prefix > substring, ties by typeID ascending.
 */
export function search(ds: Dataset, q: string, limit = 20, kinds?: string[] | null): object[] {
  const ql = q.trim().toLowerCase();
  if (!ql) return [];
  const want = kinds && kinds.length ? new Set(kinds.map((k) => String(k).toLowerCase())) : null;
  const hits: [number, number, string][] = [];
  const rank = (name: string | undefined) => {
    if (!name) return 3;
    const l = name.toLowerCase();
    return l === ql ? 0 : l.startsWith(ql) ? 1 : l.includes(ql) ? 2 : 3;
  };
  for (const [id, t] of ds.types) {
    if (!t.published) continue;
    const kind = typeKind(ds, id);
    if (kind === null || (want && !want.has(kind))) continue;
    const r = Math.min(rank(t.name), rank(ds.namesZh.get(id)));
    if (r < 3) hits.push([r, id, kind]);
  }
  hits.sort((x, y) => x[0] - y[0] || x[1] - y[1]);
  const MATCH = ['exact', 'prefix', 'substring'];
  return hits.slice(0, Math.max(0, limit)).map(([r, id, kind]) => {
    const t = ds.types.get(id)!;
    return {
      type_id: id, name: t.name, name_zh: ds.namesZh.get(id) ?? null, group: ds.groups.get(t.group)?.name ?? null,
      category_id: t.category, kind, slot: inferSlot(t.effects), meta_level: t.metaLevel, match: MATCH[r],
    };
  });
}

export function typeInfo(ds: Dataset, key: string): object {
  const id = /^\d+$/.test(key.trim()) ? Number(key) : ds.typeByName(key);
  const t = id !== undefined ? ds.types.get(id) : undefined;
  if (!t) return { error: { code: 'UNKNOWN_TYPE', message: key } };
  const attributes: Record<string, number> = {};
  for (const k of Object.keys(t.rawAttrs).map(Number).sort((a, b) => a - b)) attributes[ds.attrs.get(k)?.name ?? String(k)] = t.rawAttrs[k];
  return {
    type_id: t.id, name: t.name, name_zh: ds.namesZh.get(t.id) ?? null, group: ds.groups.get(t.group)?.name ?? null, group_id: t.group,
    category_id: t.category, published: t.published, mass: t.mass, volume: t.volume, capacity: t.capacity, slot: inferSlot(t.effects),
    attributes, effects: t.effects.map(([e, d]) => ({ id: e, name: ds.effects.get(e)?.name ?? null, default: d !== 0 })),
  };
}

export function meta(ds: Dataset): object {
  return {
    engine: ENGINE, schema_version: 1, sde_build: ds.build, sde_release_date: ds.releaseDate, dataset_sha256: ds.sha256,
    types: ds.types.size, attributes: ds.attrs.size, effects: ds.effects.size,
  };
}

/** Pyfa-exact EFT export (slots padded to the ship's modified slot counts, like Pyfa's fill()) */
export function eftExport(ds: Dataset, fit: FitRequest, name: string): string {
  const r = calc(ds, fit);
  const sl = r?.resources?.slots;
  const totals = sl ? Object.fromEntries(Object.entries(sl).map(([k, v]: [string, any]) => [k, v?.total ?? 0])) : undefined;
  return exportEft(ds, fit, name, totals);
}

/** JSONL RPC dispatcher (serve-stdio): {"id","method","params"} -> {"id","result"} */
export function rpc(ds: Dataset, line: string): object {
  let v: any;
  try {
    v = JSON.parse(line);
  } catch (e) {
    return { id: null, error: { code: 'BAD_JSON', message: (e as Error).message } };
  }
  const id = v?.id ?? null;
  const p = v?.params ?? null;
  let result: unknown;
  switch (v?.method ?? 'calc') {
    case 'calc': result = calc(ds, p); break;
    case 'eft_parse':
      try { result = parseEft(ds, p?.text ?? ''); } catch (e) { result = { error: { code: 'EFT_PARSE', message: (e as Error).message } }; }
      break;
    case 'eft_export':
      result = p?.fit?.ship ? { text: eftExport(ds, p.fit, p.name ?? 'EXCT fit') } : { error: { code: 'BAD_REQUEST', message: 'missing fit' } };
      break;
    case 'search': result = search(ds, p?.query ?? '', p?.limit ?? 20, p?.kinds ?? null); break;
    case 'type': result = typeInfo(ds, String(p?.id ?? '')); break;
    case 'meta': result = meta(ds); break;
    default: result = { error: { code: 'UNKNOWN_METHOD', message: String(v.method) } };
  }
  return { id, result };
}
