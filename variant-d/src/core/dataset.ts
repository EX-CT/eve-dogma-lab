/**
 * Engine dataset (exct-eve-dataset format v1, produced by EX-CT/eve-sde-pipeline).
 * Immutable after load. Per-type attribute maps are materialised lazily and memoised (pure derivation).
 */

export const enum Func { Item = 0, Location = 1, LocationGroup = 2, LocationRequiredSkill = 3, OwnerRequiredSkill = 4, EffectStopper = 5 }
export const enum Domain { Item = 0, Ship = 1, Char = 2, Other = 3, Structure = 4, TargetId = 5, Target = 6, None = 7 }

export interface AttrInfo {
  id: number; name: string; default: number; stackable: boolean; highIsGood: boolean;
  minAttr: number | null; maxAttr: number | null; unit: number | null; display: string | null;
}

export interface Modifier {
  func: Func; domain: Domain; modified: number; modifying: number; op: number;
  /** group id (LocationGroup) or skill type id (...RequiredSkill); 0 = owning skill (EXCT patch convention) */
  extra: number;
}

export interface EffectInfo {
  id: number; name: string; category: number;
  durationAttr: number | null; dischargeAttr: number | null; rangeAttr: number | null; falloffAttr: number | null;
  trackingAttr: number | null; resistanceAttr: number | null; fittingUsageChanceAttr: number | null;
  isOffensive: boolean; isAssistance: boolean; mods: Modifier[];
  /** memo: resolved local special handler (undefined = not resolved yet, null = none) */
  special?: unknown;
  /** memo: every modifier has domain Item */
  itemOnly: boolean;
}

export interface TypeInfo {
  id: number; name: string; group: number; category: number; published: boolean;
  mass: number; volume: number; capacity: number; radius: number;
  marketGroup: number | null; metaGroup: number | null; metaLevel: number | null; variationParent: number | null;
  /** raw attribute object from the dataset (string keys) */
  rawAttrs: Record<string, number>;
  /** [effectId, isDefault] */
  effects: [number, number][];
}

/** Read-only id -> TypeInfo store (a Map for the JSON loader, a lazily materialising table for the cache). */
export interface TypeStore {
  get(id: number): TypeInfo | undefined;
  has(id: number): boolean;
  readonly size: number;
  /** ascending type id order */
  [Symbol.iterator](): IterableIterator<[number, TypeInfo]>;
  /** optional fast path: ids of a group in ascending order without materialising other types */
  idsInGroup?(group: number): number[];
  /** optional fast path: lower-cased name -> id (published preferred, then lowest id) without materialising types */
  nameIndex?(): Map<string, number>;
}

export interface GroupInfo { name: string; category: number }
export interface DbuffInfo {
  name: string | null; aggregate: string | null; op: number;
  item: number[]; location: number[]; location_group: [number, number][]; location_skill: [number, number][];
}
export interface MutaInfo { attrs: Record<string, [number, number]>; mapping: { inputs: number[]; output: number }[] }

/** Type-level fields injected as attributes: mass(4), capacity(38), volume(161), radius(162). */
export const TYPE_FIELD_ATTRS = [4, 38, 161, 162] as const;

export interface EffectStore {
  get(id: number): EffectInfo | undefined;
  readonly size: number;
}

/** dataset effect record -> EffectInfo */
export function effectInfo(id: number, e: any): EffectInfo {
  const mods = (e.mods ?? []).map((m: number[]) => ({
    func: (m[0] >= 0 && m[0] <= 4 ? m[0] : 5) as Func,
    domain: (m[1] >= 0 && m[1] <= 6 ? m[1] : 7) as Domain,
    modified: m[2], modifying: m[3], op: m[4], extra: m[5],
  }));
  return {
    id, name: e.name, category: e.category ?? 0,
    durationAttr: e.duration_attr ?? null, dischargeAttr: e.discharge_attr ?? null, rangeAttr: e.range_attr ?? null,
    falloffAttr: e.falloff_attr ?? null, trackingAttr: e.tracking_attr ?? null, resistanceAttr: e.resistance_attr ?? null,
    fittingUsageChanceAttr: e.fitting_usage_chance_attr ?? null,
    isOffensive: !!e.is_offensive, isAssistance: !!e.is_assistance,
    mods,
    itemOnly: mods.every((m: { domain: Domain }) => m.domain === Domain.Item),
  };
}

export class Dataset {
  build = 0;
  releaseDate: string | null = null;
  sha256 = '';
  types: TypeStore = new Map<number, TypeInfo>();
  groups = new Map<number, GroupInfo>();
  /** category id -> name */
  categories = new Map<number, string>();
  attrs = new Map<number, AttrInfo>();
  effects: EffectStore = new Map<number, EffectInfo>();
  dbuffs = new Map<number, DbuffInfo>();
  /** mutaplasmids, materialised on first use */
  mutaSource: () => Record<string, MutaInfo> = () => ({});
  private mutaMap: Map<number, MutaInfo> | null = null;
  get mutaplasmids(): Map<number, MutaInfo> {
    if (this.mutaMap === null) {
      const raw = this.mutaSource();
      this.mutaMap = new Map();
      for (const k in raw) this.mutaMap.set(+k, raw[k]);
    }
    return this.mutaMap;
  }
  /** register effect names of a lazily decoded effect table (cache loader) */
  setEffectNames(ids: ArrayLike<number>, names: string[]): void {
    for (let i = 0; i < ids.length; i++) this.effectByName.set(names[i], ids[i]);
  }
  /** all skill type ids (category 16), sorted */
  skills: number[] = [];
  /** published skills, sorted */
  publishedSkills: number[] = [];
  private attrByName = new Map<string, number>();
  private effectByName = new Map<string, number>();
  private typeByNameMap: Map<string, number> | null = null;
  private typeAttrCache = new Map<number, Map<number, number>>();
  private reqSkillCache = new Map<number, number[]>();
  private modeCache = new Map<number, number | null>();

  /** T3D default mode: lowest type id in group 1306 whose name starts with the ship name (memoised) */
  /** type ids of a group, ascending (memoised) */
  groupIds(group: number): number[] {
    let l = this.groupIdCache.get(group);
    if (l !== undefined) return l;
    if (this.types.idsInGroup) l = this.types.idsInGroup(group);
    else { l = []; for (const [id, t] of this.types) if (t.group === group) l.push(id); }
    this.groupIdCache.set(group, l);
    return l;
  }
  private groupIdCache = new Map<number, number[]>();

  defaultMode(shipId: number): number | null {
    let m = this.modeCache.get(shipId);
    if (m !== undefined) return m;
    const shipName = this.types.get(shipId)!.name.toLowerCase();
    m = null;
    for (const id of this.groupIds(1306)) if (this.types.get(id)!.name.toLowerCase().startsWith(shipName) && (m === null || id < m)) m = id;
    this.modeCache.set(shipId, m);
    return m;
  }

  /** Build from the parsed dataset JSON. `sha256` is the hex digest of the (uncompressed) JSON bytes. */
  static fromJson(raw: any, sha256 = ''): Dataset {
    if (raw.format !== 'exct-eve-dataset' || raw.format_version !== 1) {
      throw new Error(`unsupported dataset format ${raw.format} v${raw.format_version}`);
    }
    const ds = new Dataset();
    ds.sha256 = sha256;
    ds.initCommon(raw);
    const ids = Object.keys(raw.types).map(Number).sort((a, b) => a - b);
    for (const id of ids) ds.addType(id, raw.types[id]);
    ds.finishTypes();
    const zh = raw.names?.zh ?? {};
    ds.zhSource = () => zh;
    return ds;
  }

  /** attributes, effects, groups, dbuffs, mutaplasmids, sde info (shared by JSON and cache loaders) */
  initCommon(raw: any): void {
    const ds = this;
    ds.build = raw.sde.build;
    ds.releaseDate = raw.sde.release_date ?? null;
    for (const k in raw.attributes) {
      const a = raw.attributes[k];
      const id = +k;
      ds.attrByName.set(a.name, id);
      ds.attrs.set(id, {
        id, name: a.name, default: a.default ?? 0, stackable: a.stackable ?? true, highIsGood: a.high_is_good ?? true,
        minAttr: a.min_attr ?? null, maxAttr: a.max_attr ?? null, unit: a.unit ?? null, display: a.display ?? null,
      });
    }
    let maxA = 0;
    for (const id of ds.attrs.keys()) if (id > maxA) maxA = id;
    ds.defArr = new Float64Array(maxA + 1);
    for (const [id, a] of ds.attrs) ds.defArr[id] = a.default;
    if (raw.effects) {
      const em = ds.effects as Map<number, EffectInfo>;
      for (const k in raw.effects) {
        const id = +k;
        ds.effectByName.set(raw.effects[k].name, id);
        em.set(id, effectInfo(id, raw.effects[k]));
      }
    }
    for (const k in raw.groups) ds.groups.set(+k, { name: raw.groups[k].name ?? '', category: raw.groups[k].category });
    for (const k in raw.categories ?? {}) ds.categories.set(+k, raw.categories[k].name ?? '');
    for (const k in raw.dbuffs ?? {}) ds.dbuffs.set(+k, raw.dbuffs[k]);
    if (raw.mutaplasmids) ds.mutaSource = () => raw.mutaplasmids;
  }

  /** register one type (raw dataset shape, or a cache record exposing the same fields); ids ascending */
  addType(id: number, t: any): void {
    const name: string = t.name ?? '';
    if (t.category === 16) this.skills.push(id);
    // reuse the parsed raw object in place (no per-type allocation)
    t.id = id;
    t.name = name;
    t.published = !!t.published;
    t.mass ??= 0; t.volume ??= 0; t.capacity ??= 0; t.radius ??= 0;
    t.marketGroup = t.market_group ?? null; t.metaGroup = t.meta_group ?? null; t.metaLevel = t.meta_level ?? null;
    t.variationParent = t.variation_parent ?? null;
    t.rawAttrs = t.attrs ?? {};
    t.effects ??= [];
    (this.types as Map<number, TypeInfo>).set(id, t as TypeInfo);
  }

  finishTypes(): void {
    this.publishedSkills = this.skills.filter((s) => this.types.get(s)!.published);
  }

  /** zh names, materialised on first use */
  zhSource: () => Record<string, string> = () => ({});
  private zhMap: Map<number, string> | null = null;
  get namesZh(): Map<number, string> {
    if (this.zhMap === null) {
      const zh = this.zhSource();
      this.zhMap = new Map();
      for (const k in zh) this.zhMap.set(+k, zh[k]);
    }
    return this.zhMap;
  }

  attrId(name: string): number { return this.attrByName.get(name) ?? 0; }
  effectId(name: string): number { return this.effectByName.get(name) ?? 0; }
  typeByName(name: string): number | undefined {
    if (this.typeByNameMap === null) {
      // built on first use; prefer published types, among equals keep the lowest id (deterministic)
      if (this.types.nameIndex) { this.typeByNameMap = this.types.nameIndex(); return this.typeByNameMap.get(name.trim().toLowerCase()); }
      const m = new Map<string, number>();
      for (const [id, t] of this.types) {
        const key = t.name.toLowerCase();
        const prev = m.get(key);
        if (prev === undefined || (t.published && !this.types.get(prev)!.published)) m.set(key, id);
      }
      this.typeByNameMap = m;
    }
    return this.typeByNameMap.get(name.trim().toLowerCase());
  }
  /** attribute default values indexed by id (dense; built in initCommon) */
  private defArr = new Float64Array(0);
  attrDefault(id: number): number { return id < this.defArr.length ? this.defArr[id] : 0; }

  /** Base attribute map of a type incl. the authoritative type-level fields (memoised). */
  typeAttrs(typeId: number): Map<number, number> {
    let m = this.typeAttrCache.get(typeId);
    if (m) return m;
    const t = this.types.get(typeId)!;
    m = new Map();
    for (const k in t.rawAttrs) m.set(+k, t.rawAttrs[k]);
    const fields = [t.mass, t.capacity, t.volume, t.radius];
    TYPE_FIELD_ATTRS.forEach((a, i) => { if (fields[i] !== 0 || !m!.has(a)) m!.set(a, fields[i]); });
    this.typeAttrCache.set(typeId, m);
    return m;
  }

  typeAttr(typeId: number, attr: number): number | undefined { return this.typeAttrs(typeId).get(attr); }

  /** requiredSkill1..6 of a type (non-zero), memoised */
  /** type + memoised attribute map + required skills in one lookup (item creation); undefined = unknown type */
  proto(typeId: number): TypeProto | undefined {
    let p = this.protoCache.get(typeId);
    if (p !== undefined) return p;
    const t = this.types.get(typeId);
    if (!t) return undefined;
    p = { t, tattrs: this.typeAttrs(typeId), reqSkills: this.requiredSkills(typeId) };
    this.protoCache.set(typeId, p);
    return p;
  }
  private protoCache = new Map<number, TypeProto>();

  requiredSkills(typeId: number): number[] {
    let r = this.reqSkillCache.get(typeId);
    if (r) return r;
    const a = this.typeAttrs(typeId);
    r = [];
    for (const id of REQ_SKILL_ATTRS) { const v = a.get(id); if (v !== undefined && v !== 0) r.push(v | 0); }
    this.reqSkillCache.set(typeId, r);
    return r;
  }
}

export interface TypeProto { t: TypeInfo; tattrs: Map<number, number>; reqSkills: number[] }

/** requiredSkill1..6 */
export const REQ_SKILL_ATTRS = [182, 183, 184, 1285, 1289, 1290] as const;
