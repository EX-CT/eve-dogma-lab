/** Shared, read-only context for stats sections. */
import type { Fit } from '../core/fit.js';
import { Kind } from '../core/graph.js';
import { NormRequest, State } from '../core/request.js';
import { floatUnerr } from './util.js';

export class StatsCtx {
  readonly modules: number[] = [];
  readonly drones: number[] = [];
  readonly fighters: number[] = [];
  readonly A: {
    cpu: number; power: number; speed: number; duration: number; capNeed: number; reload: number;
    reactivation: number; chargeRate: number; dmgMult: number; dmg: number[];
  };

  private durAttrs: number[];
  constructor(readonly fit: Fit, readonly req: NormRequest) {
    this.durAttrs = ['durationHighisGood', 'durationSensorDampeningBurstProjector', 'durationTargetIlluminationBurstProjector',
      'durationECMJammerBurstProjector', 'durationWeaponDisruptionBurstProjector'].map((n) => fit.ds.attrId(n));
    for (const it of fit.items) {
      if (it.kind === Kind.Module) this.modules.push(it.idx);
      else if (it.kind === Kind.Drone) this.drones.push(it.idx);
      else if (it.kind === Kind.Fighter) this.fighters.push(it.idx);
    }
    const a = (n: string) => this.id(n);
    this.A = {
      cpu: a('cpu'), power: a('power'), speed: a('speed'), duration: a('duration'), capNeed: a('capacitorNeed'), reload: a('reloadTime'),
      reactivation: a('moduleReactivationDelay'), chargeRate: a('chargeRate'), dmgMult: a('damageMultiplier'),
      dmg: [a('emDamage'), a('thermalDamage'), a('kineticDamage'), a('explosiveDamage')],
    };
  }

  get ds() { return this.fit.ds; }
  id(name: string): number { return this.fit.ds.attrId(name); }
  /** modified attribute by name */
  g(i: number, name: string): number { return this.fit.get(i, this.id(name)); }
  item(i: number) { return this.fit.items[i]; }
  online(i: number) { return this.fit.items[i].state >= State.Online; }
  active(i: number) { return this.fit.items[i].state >= State.Active; }
  typeName(i: number) { return this.ds.types.get(this.fit.items[i].typeId)!.name; }

  /** has the item an effect of this name (single-name form of hasEffect, no array per call) */
  hasEffect1(i: number, name: string): boolean {
    return effectNames(this.ds, this.fit.items[i].effects).has(name);
  }

  hasEffect(i: number, names: string[]): boolean {
    const set = effectNames(this.ds, this.fit.items[i].effects);
    for (let k = 0; k < names.length; k++) if (set.has(names[k])) return true;
    return false;
  }

  rawCycleMs(i: number): number {
    const f = this.fit;
    let v = Math.max(f.get(i, this.A.speed), f.get(i, this.A.duration));
    for (const a of this.durAttrs) if (a !== 0) v = Math.max(v, f.get(i, a));
    return v;
  }

  numCharges(i: number): number {
    const c = this.fit.items[i].charge;
    if (c < 0) return 0;
    const vol = this.fit.get(c, 161);
    const cap = this.fit.base(i, 38);
    return vol <= 0 ? 0 : Math.floor(floatUnerr(cap / vol));
  }

  numShots(i: number): number {
    const f = this.fit;
    const c = f.items[i].charge;
    if (c < 0) return 0;
    const n = this.numCharges(i);
    if (n > 0 && f.has(i, this.A.chargeRate)) {
      const r = f.get(i, this.A.chargeRate);
      return r > 0 ? Math.floor(n / r) : 0;
    }
    const cgd = this.id('crystalsGetDamaged');
    if (n > 0 && f.has(c, cgd)) {
      if (f.get(c, cgd) === 1) {
        const hp = f.get(c, 9);
        const chance = this.g(c, 'crystalVolatilityChance');
        const dmg = this.g(c, 'crystalVolatilityDamage');
        if (dmg * chance > 0) return Math.floor((n * hp) / (dmg * chance));
      }
      return 0;
    }
    return 0;
  }

  /** Average cycle time in ms (Pyfa getCycleParameters().averageTime) */
  avgCycleMs(i: number, factorReload: boolean): number {
    const active = this.rawCycleMs(i);
    if (active === 0) return 0;
    const inactive = this.fit.get(i, this.A.reactivation);
    const shots = this.numShots(i);
    const reload = this.fit.get(i, this.A.reload);
    if (!factorReload || shots === 0 || inactive >= reload) return active + inactive;
    return ((active + inactive) * (shots - 1) + (active + reload)) / shots;
  }
}

/** memo: effect-name set per (immutable) effects array */
const NAMES = new WeakMap<[number, number][], Set<string>>();
function effectNames(ds: import('../core/dataset.js').Dataset, effects: [number, number][]): Set<string> {
  let s = NAMES.get(effects);
  if (!s) {
    s = new Set();
    for (const [e] of effects) { const n = ds.effects.get(e)?.name; if (n !== undefined) s.add(n); }
    NAMES.set(effects, s);
  }
  return s;
}
