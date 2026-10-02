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

export interface GroupInfo { name: string; category: number }
export interface DbuffInfo {
  name: string | null; aggregate: string | null; op: number;
  item: number[]; location: number[]; location_group: [number, number][]; location_skill: [number, number][];
}
export interface MutaInfo { attrs: Record<string, [number, number]>; mapping: { inputs: number[]; output: number }[] }

/** Type-level fields injected as attributes: mass(4), capacity(38), volume(161), radius(162). */
export const TYPE_FIELD_ATTRS = [4, 38, 161, 162] as const;

export class Dataset {
  build = 0;
  releaseDate: string | null = null;
  sha256 = '';
  types = new Map<number, TypeInfo>();
  groups = new Map<number, GroupInfo>();
  attrs = new Map<number, AttrInfo>();
  effects = new Map<number, EffectInfo>();
  dbuffs = new Map<number, DbuffInfo>();
  mutaplasmids = new Map<number, MutaInfo>();
  /** all skill type ids (category 16), sorted */
  skills: number[] = [];
  /** published skills, sorted */
  publishedSkills: number[] = [];
  private attrByName = new Map<string, number>();
  private effectByName = new Map<string, number>();
  private typeByNameMap = new Map<string, number>();
  private typeAttrCache = new Map<number, Map<number, number>>();
  private reqSkillCache = new Map<number, number[]>();
  private modeCache = new Map<number, number | null>();

  /** T3D default mode: lowest type id in group 1306 whose name starts with the ship name (memoised) */
  defaultMode(shipId: number): number | null {
    let m = this.modeCache.get(shipId);
    if (m !== undefined) return m;
    const shipName = this.types.get(shipId)!.name.toLowerCase();
    m = null;
    for (const [id, t] of this.types) if (t.group === 1306 && t.name.toLowerCase().startsWith(shipName) && (m === null || id < m)) m = id;
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
    for (const k in raw.effects) {
      const e = raw.effects[k];
      const id = +k;
      ds.effectByName.set(e.name, id);
      ds.effects.set(id, {
        id, name: e.name, category: e.category ?? 0,
        durationAttr: e.duration_attr ?? null, dischargeAttr: e.discharge_attr ?? null, rangeAttr: e.range_attr ?? null,
        falloffAttr: e.falloff_attr ?? null, trackingAttr: e.tracking_attr ?? null, resistanceAttr: e.resistance_attr ?? null,
        fittingUsageChanceAttr: e.fitting_usage_chance_attr ?? null,
        isOffensive: !!e.is_offensive, isAssistance: !!e.is_assistance,
        mods: (e.mods ?? []).map((m: number[]) => ({
          func: (m[0] >= 0 && m[0] <= 4 ? m[0] : 5) as Func,
          domain: (m[1] >= 0 && m[1] <= 6 ? m[1] : 7) as Domain,
          modified: m[2], modifying: m[3], op: m[4], extra: m[5],
        })),
        itemOnly: false,
      });
      const ei = ds.effects.get(id)!;
      ei.itemOnly = ei.mods.every((m) => m.domain === Domain.Item);
    }
    for (const k in raw.groups) ds.groups.set(+k, { name: raw.groups[k].name ?? '', category: raw.groups[k].category });
    for (const k in raw.dbuffs ?? {}) ds.dbuffs.set(+k, raw.dbuffs[k]);
    for (const k in raw.mutaplasmids ?? {}) ds.mutaplasmids.set(+k, raw.mutaplasmids[k]);
  }

  /** register one type (raw dataset shape, or a cache record exposing the same fields); ids ascending */
  addType(id: number, t: any): void {
    const name: string = t.name ?? '';
    const key = name.toLowerCase();
    const prev = this.typeByNameMap.get(key);
    // prefer published types; among equals keep the lowest id (deterministic)
    if (prev === undefined || (t.published && !this.types.get(prev)!.published)) this.typeByNameMap.set(key, id);
    if (t.category === 16) this.skills.push(id);
    if (t instanceof Object && 'rawAttrs' in t) {
      this.types.set(id, t as TypeInfo);
      return;
    }
    // reuse the parsed raw object in place (no per-type allocation)
    t.id = id;
    t.name = name;
    t.published = !!t.published;
    t.mass ??= 0; t.volume ??= 0; t.capacity ??= 0; t.radius ??= 0;
    t.marketGroup = t.market_group ?? null; t.metaGroup = t.meta_group ?? null; t.metaLevel = t.meta_level ?? null;
    t.variationParent = t.variation_parent ?? null;
    t.rawAttrs = t.attrs ?? {};
    t.effects ??= [];
    this.types.set(id, t as TypeInfo);
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
  typeByName(name: string): number | undefined { return this.typeByNameMap.get(name.trim().toLowerCase()); }
  attrDefault(id: number): number { return this.attrs.get(id)?.default ?? 0; }

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

/** requiredSkill1..6 */
export const REQ_SKILL_ATTRS = [182, 183, 184, 1285, 1289, 1290] as const;
