import { Kind } from '../core/graph.js';
import { SlotName, State } from '../core/request.js';
import { StatsCtx } from './ctx.js';
import { ResourceTotals } from './resources.js';

const SLOT_DEBUG: Record<SlotName, string> = { high: 'High', mid: 'Mid', low: 'Low', rig: 'Rig', subsystem: 'Subsystem', service: 'Service' };
const fmt2 = (v: number) => v.toFixed(2);

export function validate(c: StatsCtx, used: ResourceTotals): object[] {
  const { fit, ds } = c;
  const ship = fit.ship;
  const out: object[] = [];
  const push = (code: string, message: string, idx: number | null) => out.push({ code, message, module_index: idx });
  const g = (n: string) => c.g(ship, n);
  if (used.cpu > g('cpuOutput') + 1e-9) push('CPU_OVERLOAD', `CPU used ${fmt2(used.cpu)} > output ${fmt2(g('cpuOutput'))}`, null);
  if (used.pg > g('powerOutput') + 1e-9) push('POWER_OVERLOAD', `Powergrid used ${fmt2(used.pg)} > output ${fmt2(g('powerOutput'))}`, null);
  if (used.calib > g('upgradeCapacity') + 1e-9) push('CALIBRATION_OVERLOAD', `Calibration used ${used.calib} > ${g('upgradeCapacity')}`, null);
  if (used.bw > g('droneBandwidth') + 1e-9) push('DRONE_BANDWIDTH', `Drone bandwidth used ${used.bw} > ${g('droneBandwidth')}`, null);
  for (const [slot, attr] of [['high', 'hiSlots'], ['mid', 'medSlots'], ['low', 'lowSlots'], ['rig', 'rigSlots'], ['subsystem', 'maxSubSystems'], ['service', 'serviceSlots']] as [SlotName, string][]) {
    const n = c.modules.filter((i) => c.item(i).slot === slot).length;
    if (n > g(attr)) push('SLOTS_EXCEEDED', `${SLOT_DEBUG[slot]} slots used ${n} > ${g(attr)}`, null);
  }
  const t = c.modules.filter((i) => c.hasEffect(i, ['turretFitted'])).length;
  if (t > g('turretSlotsLeft')) push('TURRET_HARDPOINTS', `turrets ${t} > hardpoints ${g('turretSlotsLeft')}`, null);
  const l = c.modules.filter((i) => c.hasEffect(i, ['launcherFitted'])).length;
  if (l > g('launcherSlotsLeft')) push('LAUNCHER_HARDPOINTS', `launchers ${l} > hardpoints ${g('launcherSlotsLeft')}`, null);
  const shipT = ds.types.get(fit.items[ship].typeId)!;
  const { groupAttrs, typeAttrs, chargeGroups, reqSkill, reqLevel } = ids(ds);
  const raw = (typeId: number, a: number): number | undefined => ds.types.get(typeId)!.rawAttrs[a];
  const fittedGroup = new Map<number, number>(), fittedType = new Map<number, number>(), activeGroup = new Map<number, number>(), onlineGroup = new Map<number, number>();
  const inc = (m: Map<number, number>, k: number) => m.set(k, (m.get(k) ?? 0) + 1);
  for (const i of c.modules) {
    const it = c.item(i);
    const idx = it.reqIndex;
    const name = c.typeName(i);
    const mt = ds.types.get(it.typeId)!;
    if (it.slot === null) push('NOT_FITTABLE', `${name} is not a fittable module`, idx);
    const gr = groupAttrs.map((a) => raw(it.typeId, a)).filter((v): v is number => v !== undefined && v !== 0).map((v) => v | 0);
    const ty = typeAttrs.map((a) => raw(it.typeId, a)).filter((v): v is number => v !== undefined && v !== 0).map((v) => v | 0);
    if ((gr.length || ty.length) && !gr.includes(shipT.group) && !ty.includes(shipT.id)) push('SHIP_RESTRICTION', `${name} cannot be fitted to ${shipT.name}`, idx);
    if (it.slot === 'rig') {
      const rs = raw(it.typeId, ds.attrId('rigSize')) ?? 0;
      const srs = g('rigSize');
      if (rs !== 0 && rs !== srs) push('RIG_SIZE', `${name} rig size ${rs} != ship rig size ${srs}`, idx);
    }
    inc(fittedGroup, it.group);
    inc(fittedType, it.typeId);
    if (it.state >= State.Online) inc(onlineGroup, it.group);
    if (it.state >= State.Active) inc(activeGroup, it.group);
    const check = (attr: string, m: Map<number, number>, key: number): [number, number] | null => {
      const lim = raw(it.typeId, ds.attrId(attr));
      if (lim === undefined) return null;
      const n = m.get(key) ?? 0;
      return lim > 0 && n > lim ? [lim, n] : null;
    };
    let r: [number, number] | null;
    if ((r = check('maxGroupFitted', fittedGroup, it.group))) push('MAX_GROUP_FITTED', `${name}: ${r[1]} fitted of group, max ${r[0]}`, idx);
    if ((r = check('maxTypeFitted', fittedType, it.typeId))) push('MAX_TYPE_FITTED', `${name}: ${r[1]} fitted, max ${r[0]}`, idx);
    if ((r = check('maxGroupOnline', onlineGroup, it.group))) push('MAX_GROUP_ONLINE', `${name}: ${r[1]} online of group, max ${r[0]}`, idx);
    if ((r = check('maxGroupActive', activeGroup, it.group))) push('MAX_GROUP_ACTIVE', `${name}: ${r[1]} active of group, max ${r[0]}`, idx);
    if (it.charge >= 0) {
      const ct = ds.types.get(fit.items[it.charge].typeId)!;
      const cg = chargeGroups.map((a) => raw(it.typeId, a)).filter((v): v is number => v !== undefined && v !== 0).map((v) => v | 0);
      if (!cg.includes(ct.group)) push('CHARGE_GROUP', `${ct.name} cannot be loaded into ${name}`, idx);
      const ms = raw(it.typeId, ds.attrId('chargeSize'));
      const cs = ct.rawAttrs[ds.attrId('chargeSize')];
      if (ms !== undefined && cs !== undefined && ms !== cs) push('CHARGE_SIZE', `${ct.name} size ${cs} != launcher size ${ms}`, idx);
      if (ct.volume > mt.capacity && mt.capacity > 0) push('CHARGE_CAPACITY', `${ct.name} does not fit into ${name}`, idx);
    }
  }
  // skills
  const missing: [number, number, number][] = [];
  const checked = new Set([Kind.Ship, Kind.Module, Kind.Charge, Kind.Drone, Kind.Fighter, Kind.Implant, Kind.Booster]);
  for (const it of fit.items) {
    if (!checked.has(it.kind)) continue;
    for (let k = 1; k <= 6; k++) {
      const s = (raw(it.typeId, reqSkill[k - 1]) ?? 0) | 0;
      if (s === 0) continue;
      const need = raw(it.typeId, reqLevel[k - 1]) ?? 1;
      if (fit.skillLevel(s) < need && !missing.some((m) => m[0] === s && m[1] >= need)) missing.push([s, need, it.typeId]);
    }
  }
  for (const [s, need, by] of missing) push('MISSING_SKILL', `${ds.types.get(s)?.name ?? '?'} ${need} required by ${ds.types.get(by)!.name}`, null);
  return out;
}

const IDS = new WeakMap<object, ReturnType<typeof mkIds>>();
function mkIds(ds: import('../core/dataset.js').Dataset) {
  return {
    groupAttrs: Array.from({ length: 20 }, (_, k) => ds.attrId(`canFitShipGroup${String(k + 1).padStart(2, '0')}`)).filter((x) => x !== 0),
    typeAttrs: Array.from({ length: 11 }, (_, k) => ds.attrId(`canFitShipType${k + 1}`)).filter((x) => x !== 0),
    chargeGroups: [1, 2, 3, 4, 5].map((k) => ds.attrId(`chargeGroup${k}`)),
    reqSkill: [1, 2, 3, 4, 5, 6].map((k) => ds.attrId(`requiredSkill${k}`)),
    reqLevel: [1, 2, 3, 4, 5, 6].map((k) => ds.attrId(`requiredSkill${k}Level`)),
  };
}
function ids(ds: import('../core/dataset.js').Dataset) {
  let v = IDS.get(ds);
  if (!v) { v = mkIds(ds); IDS.set(ds, v); }
  return v;
}
