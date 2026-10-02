/** Fit: builds the item graph for one request and registers all modifiers (no evaluation here). */
import { Dataset, Domain, Func } from './dataset.js';
import { TargetIndex, resolveTargets } from './domains.js';
import { AttrGraph, Item, Kind, Loc, Mod, SrcK } from './graph.js';
import { EXEMPT_CATEGORIES, roundHalfAway } from './operators.js';
import { FitRequest, ModuleReq, Mutation, normalize, NormRequest, SlotName, State, STATE_NAMES } from './request.js';
import { SpecialHandler, localSpecial, projectedSpecial, ProjSpecial, projectedFeed, PROJECTED_DAMAGE_EFFECTS } from './specials.js';
import { rangeFactor } from '../stats/util.js';

export const ATTR_SKILL_LEVEL = 280;
const EFFECT_SKILL_EFFECT = 132;
/** em/explosive/kinetic/thermal DamageResonance (hull) */
const HULL_RESONANCES = [113, 111, 109, 110];
/** On structures pilot skills do not affect the structure except these effects. */
const STRUCTURE_SKILL_EFFECT_NAMES = [
  'targetingMaxTargetBonusModAddMaxLockedTargetsLocationChar',
  'skillStructureMissileDamageBonus',
  'skillStructureElectronicSystemsCapNeedBonus',
  'skillStructureEngineeringSystemsCapNeedBonus',
  'skillStructureDoomsdayDurationBonus',
];

export class EngineError extends Error {
  constructor(public code: string, message: string, public path: string) { super(message); }
}

export const stateOf = (s: string | null | undefined): State | null => {
  if (s === null || s === undefined) return null;
  const i = STATE_NAMES.indexOf(s as never);
  return i < 0 ? null : (i as State);
};

function stateOk(category: number, state: State): boolean {
  switch (category) {
    case 0: case 4: return state >= State.Online;
    case 1: return state >= State.Active;
    case 5: return state >= State.Overheated;
    case 7: return true;
    default: return false; // 2 target, 3 area, 6 dungeon: not local
  }
}

/** Slot from the type's slot effect (hiPower 12, medPower 13, loPower 11, rigSlot 2663, subSystem 3772, serviceSlot 6306). */
export function inferSlot(effects: [number, number][]): SlotName | null {
  for (const [e] of effects) {
    switch (e) {
      case 12: return 'high';
      case 13: return 'mid';
      case 11: return 'low';
      case 2663: return 'rig';
      case 3772: return 'subsystem';
      case 6306: return 'service';
    }
  }
  return null;
}

/** Inputs of a modifier source, without op/pen/src */
export interface SrcSpec { k: SrcK; item?: number; attr?: number; v?: number; a2?: number; a3?: number; mul?: boolean }

const NO_IDS: number[] = [];

export class Fit extends AttrGraph {
  ship = 0;
  char = 0;
  warnings: string[] = [];
  isStructure = false;
  index!: TargetIndex;
  skillDefault = 0;
  skillCustom = new Map<number, number>();
  /** trained level of a skill as given by the request (0..5) */
  skillLevel(s: number): number { return this.skillCustom.get(s) ?? (this.ds.types.get(s)?.published && this.ds.types.get(s)?.category === 16 ? this.skillDefault : 0); }
  /** incoming remote reps / cap transfers / neuts from projected items (evaluated in stats) */
  projSpecial: ProjSpecial[] = [];

  constructor(ds: Dataset) { super(ds); }

  // ---------------------------------------------------------------- items
  newItem(typeId: number, kind: Kind, loc: Loc, path: string): number {
    const t = this.ds.types.get(typeId);
    if (!t) throw new EngineError('UNKNOWN_TYPE', `unknown type_id ${typeId}`, path);
    const it: Item = {
      idx: this.items.length, typeId, group: t.group, category: t.category, kind, state: State.Online, loc,
      owned: kind === Kind.Module || kind === Kind.Charge || kind === Kind.Drone || kind === Kind.Fighter || kind === Kind.Ship,
      parent: -1, charge: -1, slot: null, reqIndex: null, quantity: 1, activeCount: 0,
      base: null, ovA: -1, ovV: 0, tattrs: this.ds.typeAttrs(typeId), cells: null,
      reqSkills: this.ds.requiredSkills(typeId), effects: t.effects,
      fighterAbilities: null, boosterSideEffects: NO_IDS, spool: null, distance: null,
    };
    this.items.push(it);
    return it.idx;
  }

  private applyMutation(idx: number, m: Mutation): void {
    const ds = this.ds;
    const it = this.items[idx];
    const base = ds.types.get(m.base_type_id);
    if (base) {
      // base type's attributes first, the mutated type's own attributes win
      const ownRaw = ds.types.get(it.typeId)!.rawAttrs;
      for (const k in base.rawAttrs) if (!(k in ownRaw)) this.setBase(idx, Number(k), base.rawAttrs[k]);
      const extra = base.effects.filter(([e]) => !it.effects.some(([x]) => x === e));
      if (extra.length) it.effects = it.effects.concat(extra);
      if (it.reqSkills.length === 0) it.reqSkills = ds.requiredSkills(m.base_type_id);
      if (this.base(idx, 4) === 0 && base.mass !== 0) this.setBase(idx, 4, base.mass);
    }
    const muta = m.mutaplasmid_type_id != null ? ds.mutaplasmids.get(m.mutaplasmid_type_id) : undefined;
    for (const k of Object.keys(m.attributes ?? {}).sort()) {
      const aid = Number(k);
      if (!Number.isInteger(aid)) continue;
      let val = m.attributes![k];
      if (muta && base) {
        const range = muta.attrs[k];
        const bv = base.rawAttrs[k];
        if (range && bv !== undefined && bv !== 0) {
          const a = bv * range[0], b = bv * range[1];
          val = Math.min(Math.max(val, Math.min(a, b)), Math.max(a, b));
        }
      }
      this.setBase(idx, aid, val);
    }
  }

  private addModule(i: number, m: ModuleReq, path: string): void {
    const idx = this.newItem(m.type_id, Kind.Module, Loc.Ship, path);
    const it = this.items[idx];
    const slot = m.slot ?? inferSlot(it.effects);
    it.slot = slot;
    it.reqIndex = i;
    it.spool = m.spool ?? null;
    it.state = stateOf(m.state) ?? State.Online;
    if ((slot === 'rig' || slot === 'subsystem') && it.state !== State.Offline) it.state = State.Online;
    if (m.mutation) this.applyMutation(idx, m.mutation);
    if (m.charge_type_id != null) {
      const c = this.newItem(m.charge_type_id, Kind.Charge, Loc.Ship, `${path}/charge_type_id`);
      this.items[c].parent = idx;
      this.items[c].reqIndex = i;
      this.items[idx].charge = c;
    }
  }

  static build(ds: Dataset, req: NormRequest): Fit {
    const fit = new Fit(ds);
    fit.ship = fit.newItem(req.ship.type_id, Kind.Ship, Loc.Ship, '/ship/type_id');
    fit.isStructure = fit.items[fit.ship].category === 65;
    fit.char = fit.newItem(1373, Kind.Char, Loc.Char, '/character');
    if (req.character.security_status != null) {
      const a = ds.attrId('pilotSecurityStatus');
      if (a !== 0) fit.setBase(fit.char, a, req.character.security_status);
    }
    // skills: every published skill exists (untrained = level 0); items are created after the rest of the fit
    // (see addSkills) so that skills which cannot reach any item of this fit are never materialised
    fit.skillDefault = Math.min(req.character.skills.default_level ?? 0, 5);
    for (const [k, v] of Object.entries(req.character.skills.levels)) {
      const id = /^\d+$/.test(k) ? Number(k) : ds.typeByName(k);
      if (id !== undefined) fit.skillCustom.set(id, Math.min(v, 5));
    }
    // tactical destroyer mode: default to the lowest type id mode named after the ship
    let mode = req.ship.mode_type_id;
    if (mode == null) {
      const best = ds.defaultMode(req.ship.type_id);
      if (best !== null) {
        fit.warnings.push(`no tactical mode given; defaulted to type ${best}`);
        mode = best;
      }
    }
    if (mode != null) {
      const idx = fit.newItem(mode, Kind.Mode, Loc.Nowhere, '/ship/mode_type_id');
      fit.items[idx].owned = false;
    }
    req.modules.forEach((m, i) => fit.addModule(i, m, `/modules/${i}`));
    req.drones.forEach((d, i) => {
      const idx = fit.newItem(d.type_id, Kind.Drone, Loc.Space, `/drones/${i}`);
      if (d.mutation) fit.applyMutation(idx, d.mutation);
      const it = fit.items[idx];
      it.quantity = Math.max(d.quantity ?? 1, 1);
      it.activeCount = Math.min(d.active ?? 0, it.quantity);
      it.state = it.activeCount > 0 ? State.Active : State.Offline;
      it.reqIndex = i;
    });
    req.fighters.forEach((f, i) => {
      const idx = fit.newItem(f.type_id, Kind.Fighter, Loc.Space, `/fighters/${i}`);
      const sq = ds.attrId('fighterSquadronMaxSize');
      const maxsq = fit.has(idx, sq) ? Math.trunc(fit.base(idx, sq)) : 1;
      const it = fit.items[idx];
      const q = f.quantity ?? maxsq;
      it.quantity = Math.min(Math.max(q, 1), Math.max(maxsq, 1));
      if ((f.quantity ?? 0) > maxsq) fit.warnings.push(`fighters/${i}: squadron size ${f.quantity} capped to ${maxsq}`);
      const active = f.active ?? true;
      it.activeCount = active ? it.quantity : 0;
      it.state = active ? State.Active : State.Offline;
      it.fighterAbilities = f.abilities ?? defaultFighterAbilities(ds, it.effects);
      it.reqIndex = i;
    });
    req.implants.forEach((imp, i) => {
      const idx = fit.newItem(imp, Kind.Implant, Loc.Char, `/implants/${i}`);
      fit.items[idx].owned = false;
      fit.items[idx].reqIndex = i;
    });
    req.boosters.forEach((b, i) => {
      const idx = fit.newItem(b.type_id, Kind.Booster, Loc.Char, `/boosters/${i}`);
      fit.items[idx].owned = false;
      fit.items[idx].boosterSideEffects = b.side_effects ?? [];
      fit.items[idx].reqIndex = i;
    });
    req.environment.effect_type_ids.forEach((e, i) => {
      const idx = fit.newItem(e, Kind.Beacon, Loc.Nowhere, `/environment/effect_type_ids/${i}`);
      fit.items[idx].owned = false;
    });
    req.projected.forEach((p, i) => {
      const amount = Math.max(p.amount ?? 1, 1);
      if (p.kind === 'module') {
        if (!p.module) return;
        for (let k = 0; k < amount; k++) {
          const idx = fit.newItem(p.module.type_id, Kind.Projected, Loc.Nowhere, `/projected/${i}`);
          const it = fit.items[idx];
          it.owned = false;
          it.state = stateOf(p.module.state) ?? State.Active;
          it.distance = p.distance_m ?? null;
          it.reqIndex = i;
          if (p.module.charge_type_id != null) {
            const c = fit.newItem(p.module.charge_type_id, Kind.Charge, Loc.Nowhere, `/projected/${i}/module/charge_type_id`);
            fit.items[c].parent = idx;
            fit.items[c].owned = false;
            fit.items[idx].charge = c;
          }
        }
      } else if (p.kind === 'fit') {
        if (p.fit) fit.addProjectedFit(i, p.fit, amount, p.distance_m ?? null);
      } else if (p.kind === 'drone') {
        if (!p.drone) return;
        for (let k = 0; k < amount * Math.max(p.drone.quantity ?? 1, 1); k++) {
          const idx = fit.newItem(p.drone.type_id, Kind.Projected, Loc.Nowhere, `/projected/${i}`);
          const it = fit.items[idx];
          it.owned = false;
          it.state = State.Active;
          it.distance = p.distance_m ?? null;
        }
      } else {
        fit.warnings.push(`projected kind '${p.kind}' not supported yet (index ${i})`);
      }
    });
    // system security -> securityModifier (default nullsec, like Pyfa)
    {
      const sec = (req.environment.system_security ?? 'nullsec').toLowerCase();
      let src: string;
      if (['hisec', 'highsec', 'high'].includes(sec)) src = 'hiSecModifier';
      else if (['lowsec', 'low'].includes(sec)) src = 'lowSecModifier';
      else if (['nullsec', 'null', 'wspace', 'wormhole', 'w-space'].includes(sec)) src = 'nullSecModifier';
      else {
        fit.warnings.push(`unknown system_security '${sec}', using nullsec`);
        src = 'nullSecModifier';
      }
      const srcId = ds.attrId(src), dst = ds.attrId('securityModifier');
      for (const it of fit.items) if (fit.has(it.idx, srcId)) fit.setBase(it.idx, dst, fit.base(it.idx, srcId));
    }
    for (const o of req.overrides) for (const it of fit.items) if (it.typeId === o.type_id) fit.setBase(it.idx, o.attribute_id, o.value);
    fit.index = new TargetIndex(fit);
    fit.addSkills(req);
    fit.registerAll(req);
    applyRah(fit, req);
    return fit;
  }

  /**
   * Whole projected fit: compute the source fit on its own (skills, implants, fleet), then project each active
   * module / active drone as a frozen item carrying the source-modified attribute values.
   */
  private addProjectedFit(i: number, srcReq: FitRequest, amount: number, distance: number | null): void {
    let src: Fit;
    try {
      src = Fit.build(this.ds, normalize({ ...srcReq, projected: [] }));
    } catch (e) {
      this.warnings.push(`projected[${i}] fit: ${(e as Error).message}`);
      return;
    }
    const frozen: [number, number, Map<number, number>][] = [];
    for (const it of src.items) {
      const copies = it.kind === Kind.Module && it.state >= State.Active ? 1 : it.kind === Kind.Drone ? it.activeCount : 0;
      if (copies === 0) continue;
      const vals = new Map<number, number>();
      for (const a of src.attrKeys(it.idx)) vals.set(a, src.get(it.idx, a));
      frozen.push([it.typeId, copies, vals]);
    }
    for (const [typeId, copies, vals] of frozen) {
      for (let k = 0; k < copies * amount; k++) {
        const idx = this.newItem(typeId, Kind.Projected, Loc.Nowhere, `/projected/${i}`);
        const it = this.items[idx];
        it.owned = false;
        it.state = State.Active;
        it.distance = distance;
        it.reqIndex = i;
        it.base = new Map(vals);
        it.ovA = -1;
      }
    }
  }

  /** every attribute id present on an item (type, base overrides, modified cells) */
  attrKeys(i: number): number[] {
    const it = this.items[i];
    const keys = new Set<number>(it.tattrs.keys());
    if (it.base) for (const k of it.base.keys()) keys.add(k);
    if (it.cells) for (const k of it.cells.keys()) keys.add(k);
    return [...keys].sort((a, b) => a - b);
  }

  // ---------------------------------------------------------------- registration
  push(target: number, attr: number, op: number, s: SrcSpec, sourceItem: number, sourceCat: number): void {
    const info = this.ds.attrs.get(attr);
    const stackable = info ? info.stackable : true;
    const m: Mod = {
      op, pen: !stackable && !EXEMPT_CATEGORIES.has(sourceCat), k: s.k, item: s.item ?? -1, attr: s.attr ?? 0, v: s.v ?? 0,
      a2: s.a2 ?? 0, a3: s.a3 ?? -1, mul: s.mul ?? false, src: sourceItem,
    };
    this.addMod(target, attr, m);
  }
  /** modifier whose value is attribute `srcAttr` of item `srcItem` */
  pushAttr(target: number, attr: number, op: number, srcItem: number, srcAttr: number, sourceCat: number): void {
    const info = this.ds.attrs.get(attr);
    this.pushAttrNS(target, attr, op, srcItem, srcAttr, sourceCat, info !== undefined && !info.stackable);
  }

  /** pushAttr with the target attribute's non-stackable flag precomputed (compiled plans) */
  pushAttrNS(target: number, attr: number, op: number, srcItem: number, srcAttr: number, sourceCat: number, nonStack: boolean): void {
    const pen = nonStack && !EXEMPT_CATEGORIES.has(sourceCat);
    this.addMod(target, attr, { op, pen, k: SrcK.Attr, item: srcItem, attr: srcAttr, v: 0, a2: 0, a3: -1, mul: false, src: srcItem });
  }

  targets(src: number, func: Func, domain: Domain, extra: number): readonly number[] {
    return resolveTargets(this, this.index, src, func, domain, extra, this.ship, this.char, this.isStructure);
  }

  effectiveState(i: number): State {
    const it = this.items[i];
    switch (it.kind) {
      case Kind.Charge: return it.parent >= 0 ? this.items[it.parent].state : State.Online;
      case Kind.Ship: case Kind.Char: case Kind.Skill: case Kind.Implant: case Kind.Booster: case Kind.Mode: case Kind.Beacon:
        return State.Online;
      case Kind.Drone: case Kind.Fighter: return it.activeCount > 0 ? State.Active : State.Offline;
      default: return it.state;
    }
  }

  private registerAll(req: NormRequest): void {
    const ds = this.ds;
    const eBastion = ds.effectId('moduleBonusBastionModule');
    const structureOk = new Set(STRUCTURE_SKILL_EFFECT_NAMES.map((n) => ds.effectId(n)));
    const n = this.items.length;
    for (let i = 0; i < n; i++) {
      const it = this.items[i];
      const kind = it.kind;
      if (kind === Kind.Projected) {
        this.registerProjected(i);
        continue;
      }
      if (this.isStructure && (kind === Kind.Drone || kind === Kind.Implant || kind === Kind.Booster)) continue;
      // A skill's attributes are only ever read through its own outgoing modifiers: if none of them reaches
      // an item of this fit, the skill (incl. its self-modifiers) is irrelevant and is skipped.
      const state = this.effectiveState(i);
      const cat = it.category;
      const plan = planFor(ds, it.typeId, it.effects, eBastion);
      for (const pe of plan.effects) {
        const eid = pe.eid;
        const e = pe.e;
        if (this.isStructure && kind === Kind.Skill && !structureOk.has(eid) && !e.itemOnly) continue;
        if (e.fittingUsageChanceAttr !== null && !it.boosterSideEffects.includes(eid)) continue;
        if (kind === Kind.Fighter && e.category !== 0) {
          const used = it.fighterAbilities !== null ? it.fighterAbilities.includes(eid) : pe.isDefault;
          if (!used) continue;
        }
        if (!stateOk(e.category, state)) continue;
        if (pe.special !== null) {
          pe.special({ fit: this, item: i, cat });
          continue;
        }
        for (const m of pe.mods) {
          const c = m.bastion ? 6 : cat;
          const ts = this.targets(i, m.func, m.domain, m.extra);
          for (let k = 0; k < ts.length; k++) this.pushAttrNS(ts[k], m.modified, m.op, i, m.modifying, c, m.nonStack);
        }
      }
    }
    this.registerBuffs(req);
  }

  private addSkills(req: NormRequest): void {
    const ds = this.ds;
    let list = ds.publishedSkills;
    if (this.skillCustom.size) list = [...new Set([...list, ...this.skillCustom.keys()])].sort((a, b) => a - b);
    const eBastion = ds.effectId('moduleBonusBastionModule');
    for (const s of list) {
      const t = ds.types.get(s);
      if (!t) continue;
      // A skill's attributes are only read through its own outgoing modifiers: skip it if none reaches the fit.
      const plan = planFor(ds, s, t.effects, eBastion);
      let reach = plan.hasSpecial;
      for (let k = 0; !reach && k < plan.outgoing.length; k++) {
        const m = plan.outgoing[k];
        if (m.domain !== Domain.Other && resolveTargets(this, this.index, this.char, m.func, m.domain, m.extra, this.ship, this.char, this.isStructure).length > 0) reach = true;
      }
      if (!reach) continue;
      const idx = this.newItem(s, Kind.Skill, Loc.Char, '/character/skills');
      this.items[idx].owned = false;
      this.setBase(idx, ATTR_SKILL_LEVEL, this.skillLevel(s));
      for (const o of req.overrides) if (o.type_id === s) this.setBase(idx, o.attribute_id, o.value);
      this.index.addCharItem(this.items[idx]);
    }
  }

  private registerProjected(i: number): void {
    const ds = this.ds;
    const it = this.items[i];
    const ship = this.ship;
    for (const [eid] of it.effects) {
      const e = ds.effects.get(eid);
      if (!e || (e.category !== 2 && e.category !== 3)) continue;
      if (it.state < State.Active) continue;
      const opt = e.rangeAttr !== null && this.has(i, e.rangeAttr) ? this.base(i, e.rangeAttr) : 0;
      const fo = e.falloffAttr !== null && this.has(i, e.falloffAttr) ? this.base(i, e.falloffAttr) : 0;
      const factor = rangeFactor(opt, fo, it.distance, true);
      const rr = ds.attrId('remoteResistanceID');
      const resist = e.resistanceAttr ?? (this.has(i, rr) ? Math.trunc(this.base(i, rr)) : 0);
      const push = (targetAttr: number, srcAttr: number, op: number) =>
        this.push(ship, targetAttr, op, { k: SrcK.Projected, item: i, attr: srcAttr, v: factor, a2: resist, a3: ship, mul: op === 4 || op === 0 }, i, it.category);
      if (e.mods.length > 0) {
        for (const m of e.mods) {
          if ((m.domain === Domain.TargetId || m.domain === Domain.Target || m.domain === Domain.Ship) && m.func === Func.Item) push(m.modified, m.modifying, m.op);
        }
        continue;
      }
      const pm = projectedSpecial(e.name);
      if (pm) {
        for (const [t, s, op] of pm) push(ds.attrId(t), ds.attrId(s), op);
        continue;
      }
      const feed = projectedFeed(this, i, e.name, resist);
      if (feed !== null) this.projSpecial.push(...feed);
      else if (!PROJECTED_DAMAGE_EFFECTS.has(e.name)) this.warnings.push(`projected effect '${e.name}' not modelled yet`);
    }
  }

  private registerBuffs(req: NormRequest): void {
    const ds = this.ds;
    const agg = new Map<number, number>();
    for (const b of req.fleet.buffs) {
      const info = ds.dbuffs.get(b.buff_id);
      if (!info) {
        this.warnings.push(`unknown warfare buff ${b.buff_id}`);
        continue;
      }
      const cur = agg.get(b.buff_id);
      agg.set(b.buff_id, cur === undefined ? b.value : info.aggregate === 'Minimum' ? Math.min(cur, b.value) : Math.max(cur, b.value));
    }
    // Pyfa keeps, per buff id, the single strongest (by |value|) source among the fit's own bursts and the
    // fleet booster fits; explicit fleet.buffs override both.
    const pairs: [number, number][] = [1, 2, 3, 4].map((k) => [ds.attrId(`warfareBuff${k}ID`), ds.attrId(`warfareBuff${k}Value`)]);
    const best = new Map<number, { v: number; s: SrcSpec; target: number }>();
    const offer = (id: number, v: number, s: SrcSpec, target: number) => {
      const old = best.get(id);
      if (old === undefined || Math.abs(old.v) < Math.abs(v)) best.set(id, { v, s, target });
    };
    const scan = (f: Fit, local: boolean) => {
      for (const it of f.items) {
        if (it.kind !== Kind.Module || it.state < State.Active) continue;
        for (const [ida, vala] of pairs) {
          const id = f.has(it.idx, ida) ? Math.trunc(f.get(it.idx, ida)) : 0;
          if (id === 0 || agg.has(id)) continue;
          const v = f.get(it.idx, vala);
          offer(id, v, local ? { k: SrcK.Attr, item: it.idx, attr: vala } : { k: SrcK.Const, v }, local ? it.idx : this.ship);
        }
      }
    };
    scan(this, true);
    req.fleet.booster_fits.forEach((bf, k) => {
      try {
        scan(Fit.build(ds, normalize({ ...bf, fleet: { ...(bf.fleet ?? {}), booster_fits: [] } })), false);
      } catch (e) {
        this.warnings.push(`fleet.booster_fits[${k}]: ${(e as Error).message}`);
      }
    });
    for (const [id, v] of agg) best.set(id, { v, s: { k: SrcK.Const, v }, target: this.ship });
    for (const id of [...best.keys()].sort((a, b) => a - b)) {
      const b = best.get(id)!;
      this.applyBuff(id, b.s, b.target);
    }
    this.invalidate();
  }

  applyBuff(id: number, s: SrcSpec, sourceItem: number): void {
    const info = this.ds.dbuffs.get(id);
    if (!info) return;
    const op = info.op;
    const ship = this.ship;
    const cat = 0; // buffs are never exempt
    for (const a of info.item) this.push(ship, a, op, s, sourceItem, cat);
    for (const a of info.location) for (const t of this.targets(ship, Func.Location, Domain.Ship, 0)) this.push(t, a, op, s, sourceItem, cat);
    for (const [a, g] of info.location_group) for (const t of this.targets(ship, Func.LocationGroup, Domain.Ship, g)) this.push(t, a, op, s, sourceItem, cat);
    for (const [a, sk] of info.location_skill) for (const t of this.targets(ship, Func.LocationRequiredSkill, Domain.Ship, sk)) this.push(t, a, op, s, sourceItem, cat);
  }
}

/** Pyfa default fighter abilities: standard attack on; others (except MWD/evasive/MJD) only before it in effect id order. */
function defaultFighterAbilities(ds: Dataset, effects: [number, number][]): number[] {
  const ids = effects.map(([e]) => e).sort((a, b) => a - b);
  const on: number[] = [];
  let stdSeen = false;
  for (const e of ids) {
    const n = ds.effects.get(e)?.name;
    if (!n || !n.startsWith('fighterAbility')) continue;
    if (n === 'fighterAbilityAttackM') {
      on.push(e);
      stdSeen = true;
    } else if (!stdSeen && n !== 'fighterAbilityMicroWarpDrive' && n !== 'fighterAbilityEvasiveManeuvers' && n !== 'fighterAbilityMicroJumpDrive') on.push(e);
  }
  return on;
}

/**
 * Reactive Armor Hardener adaptation (no modifierInfo). Same algorithm as Pyfa/eos (LGPL):
 * simulate cycles vs the incoming pattern until a loop, average the loop, apply as penalised PreMul.
 */
function applyRah(fit: Fit, req: NormRequest): void {
  const ds = fit.ds;
  const eid = ds.effectId('adaptiveArmorHardener');
  if (eid === 0) return;
  const attrs = ['armorEmDamageResonance', 'armorThermalDamageResonance', 'armorKineticDamageResonance', 'armorExplosiveDamageResonance'].map((n) => ds.attrId(n));
  const shiftAttr = ds.attrId('resistanceShiftAmount');
  const rahs = fit.items.filter((it) => it.kind === Kind.Module && it.state >= State.Active && it.effects.some(([e]) => e === eid)).map((it) => it.idx);
  const disable = req.options.rah === 'disable';
  const dp = req.damage_pattern ?? { em: 25, thermal: 25, kinetic: 25, explosive: 25 };
  const pattern = [dp.em, dp.thermal, dp.kinetic, dp.explosive];
  const ship = fit.ship;
  for (const m of rahs) {
    fit.invalidate();
    const res = attrs.map((a) => fit.get(m, a));
    if (!disable) {
      const base = [0, 1, 2, 3].map((k) => pattern[k] * fit.get(ship, attrs[k]));
      const shift = fit.get(m, shiftAttr) / 100;
      const cycles: number[][] = [];
      let loopStart = -20;
      for (let n = 0; n < 50; n++) {
        // in-game tie order em, explosive, kinetic, thermal; stable sort like Python
        const t = [0, 3, 2, 1].map((k) => ({ k, dmg: base[k] * res[k], r: res[k] }));
        t.sort((a, b) => a.dmg - b.dmg);
        let c0: number, c1: number, c2: number, c3: number;
        if (t[2].dmg === 0) {
          c0 = 1 - t[0].r; c1 = 1 - t[1].r; c2 = 1 - t[2].r; c3 = -(c0 + c1 + c2);
        } else if (t[1].dmg === 0) {
          c0 = 1 - t[0].r; c1 = 1 - t[1].r; c2 = -(c0 + c1) / 2; c3 = c2;
        } else {
          c0 = Math.min(shift, 1 - t[0].r); c1 = Math.min(shift, 1 - t[1].r); c2 = -(c0 + c1) / 2; c3 = c2;
        }
        res[t[0].k] = t[0].r + c0;
        res[t[1].k] = t[1].r + c1;
        res[t[2].k] = t[2].r + c2;
        res[t[3].k] = t[3].r + c3;
        const hit = cycles.findIndex((v) => v.every((x, k) => Math.abs(res[k] - x) <= 1e-6));
        if (hit >= 0) {
          loopStart = hit;
          break;
        }
        cycles.push(res.slice());
      }
      const start = loopStart >= 0 ? loopStart : Math.max(cycles.length - 20, 0);
      const lp = cycles.slice(start);
      if (lp.length) for (let k = 0; k < 4; k++) res[k] = roundHalfAway((lp.reduce((s, v) => s + v[k], 0) / lp.length) * 1000) / 1000;
    }
    const cat = fit.items[m].category;
    for (let k = 0; k < 4; k++) {
      if (!disable) fit.push(m, attrs[k], 7, { k: SrcK.Const, v: res[k] }, m, cat);
      fit.push(ship, attrs[k], 0, { k: SrcK.Const, v: res[k] }, m, cat);
    }
  }
  fit.invalidate();
}

// ------------------------------------------------------------------ compiled per-type registration plans
interface PlanMod { func: Func; domain: Domain; modified: number; modifying: number; op: number; extra: number; bastion: boolean; nonStack: boolean }
interface PlanEffect { eid: number; e: import('./dataset.js').EffectInfo; isDefault: boolean; special: SpecialHandler | null; mods: PlanMod[] }
interface Plan { effects: PlanEffect[]; outgoing: PlanMod[]; hasSpecial: boolean }
/** memo keyed by the (immutable) effects array of a type, or of a mutated item */
const PLANS = new WeakMap<[number, number][], Plan>();

function planFor(ds: Dataset, typeId: number, effects: [number, number][], eBastion: number): Plan {
  let p = PLANS.get(effects);
  if (p) return p;
  p = { effects: [], outgoing: [], hasSpecial: false };
  for (const [eid, d] of effects) {
    if (eid === EFFECT_SKILL_EFFECT) continue;
    const e = ds.effects.get(eid);
    if (!e) continue;
    const special = localSpecial(e.name) ?? null;
    if (special) p.hasSpecial = true;
    const mods: PlanMod[] = [];
    for (const m of e.mods) {
      if (m.func === Func.EffectStopper || m.op === 9) continue;
      if (m.domain === Domain.TargetId || m.domain === Domain.Target) continue;
      // EXCT convention: skill filter 0 = the type owning the effect (skill self-bonuses)
      const extra = m.extra === 0 && (m.func === Func.LocationRequiredSkill || m.func === Func.OwnerRequiredSkill) ? typeId : m.extra;
      // Bastion hull resists are not stacking penalised in game (observed by Pyfa)
      const pm = { func: m.func, domain: m.domain, modified: m.modified, modifying: m.modifying, op: m.op, extra, bastion: eid === eBastion && HULL_RESONANCES.includes(m.modified), nonStack: ds.attrs.get(m.modified)?.stackable === false };
      mods.push(pm);
      if (m.domain !== Domain.Item) p.outgoing.push(pm);
    }
    p.effects.push({ eid, e, isDefault: d !== 0, special, mods });
  }
  PLANS.set(effects, p);
  return p;
}
