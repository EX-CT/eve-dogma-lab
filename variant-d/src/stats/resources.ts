import { SlotName } from '../core/request.js';
import { StatsCtx } from './ctx.js';

export interface ResourceTotals { cpu: number; pg: number; calib: number; bw: number }

export function resources(c: StatsCtx): { json: object; used: ResourceTotals } {
  const { fit, A } = c;
  const ship = fit.ship;
  const sum = (attr: number, pred: (i: number) => boolean) => c.modules.reduce((s, i) => (pred(i) ? s + fit.get(i, attr) : s), 0);
  const cpu = sum(A.cpu, (i) => c.online(i));
  const pg = sum(A.power, (i) => c.online(i));
  const upgradeCost = c.id('upgradeCost');
  const calib = sum(upgradeCost, (i) => c.item(i).slot === 'rig');
  const bw = c.drones.reduce((s, i) => s + c.g(i, 'droneBandwidthUsed') * c.item(i).activeCount, 0);
  const bay = c.drones.reduce((s, i) => s + fit.get(i, 161) * c.item(i).quantity, 0);
  const fbay = c.fighters.reduce((s, i) => s + fit.get(i, 161) * c.item(i).quantity, 0);
  const cargo = c.req.cargo.reduce((s, x) => s + (c.ds.types.get(x.type_id)?.volume ?? 0) * (x.quantity ?? 1), 0);
  const countSlot = (s: SlotName) => c.modules.filter((i) => c.item(i).slot === s).length;
  const turrets = c.modules.filter((i) => c.hasEffect(i, ['turretFitted'])).length;
  const launchers = c.modules.filter((i) => c.hasEffect(i, ['launcherFitted'])).length;
  const u = (used: number, total: number) => ({ used, total });
  const tot = (n: string) => c.g(ship, n);
  const fclass = (i: number) => (c.g(i, 'fighterSquadronIsHeavy') > 0 ? 'heavy' : c.g(i, 'fighterSquadronIsSupport') > 0 ? 'support' : 'light');
  const activeF = c.fighters.filter((i) => c.item(i).activeCount > 0);
  const classUsed = (k: string) => activeF.filter((i) => fclass(i) === k).length;
  return {
    used: { cpu, pg, calib, bw },
    json: {
      cpu: u(cpu, fit.get(ship, c.id('cpuOutput'))),
      power: u(pg, fit.get(ship, c.id('powerOutput'))),
      calibration: u(calib, fit.get(ship, c.id('upgradeCapacity'))),
      drone_bandwidth: u(bw, tot('droneBandwidth')),
      drone_bay: u(bay, tot('droneCapacity')),
      fighter_bay: u(fbay, tot('fighterCapacity')),
      cargo: u(cargo, fit.get(ship, 38)),
      slots: {
        high: u(countSlot('high'), tot('hiSlots')), mid: u(countSlot('mid'), tot('medSlots')), low: u(countSlot('low'), tot('lowSlots')),
        rig: u(countSlot('rig'), tot('rigSlots')), subsystem: u(countSlot('subsystem'), tot('maxSubSystems')), service: u(countSlot('service'), tot('serviceSlots')),
      },
      hardpoints: { turret: u(turrets, tot('turretSlotsLeft')), launcher: u(launchers, tot('launcherSlotsLeft')) },
      fighter_tubes: {
        total: u(activeF.length, tot('fighterTubes')), light: u(classUsed('light'), tot('fighterLightSlots')),
        support: u(classUsed('support'), tot('fighterSupportSlots')), heavy: u(classUsed('heavy'), tot('fighterHeavySlots')),
      },
    },
  };
}
