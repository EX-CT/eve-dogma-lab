import { STATE_NAMES } from '../core/request.js';
import { Drain, simulate } from './capsim.js';
import { StatsCtx } from './ctx.js';

/** capacitor section + per-module rows */
export interface CapInfo { json: Record<string, unknown>; modules: object[]; peak: number; used: number; added: number; stable: boolean }

export function capacitor(c: StatsCtx): CapInfo {
  const { fit, req, A, ds } = c;
  const ship = fit.ship;
  const factorReload = req.options.factor_reload;
  const cap = c.g(ship, 'capacitorCapacity');
  const rr = c.g(ship, 'rechargeRate');
  const peak = rr > 0 ? (10 / (rr / 1000)) * 0.5 * 0.5 * cap : 0;
  const drains: Drain[] = [];
  let used = 0, added = 0;
  const rows: object[] = [];
  for (let k1 = 0; k1 < c.modules.length; k1++) {
    const i = c.modules[k1];
    const it = c.item(i);
    let capNeed = fit.get(i, A.capNeed);
    const isInj = ds.groups.get(it.group)?.name === 'Capacitor Booster';
    if (isInj) capNeed = -(it.charge >= 0 ? c.g(it.charge, 'capacitorBonus') : 0);
    // local nosferatu counts as cap income (assumes the target has cap), like Pyfa
    if (c.hasEffect1(i, 'energyNosferatuFalloff') && !req.options.nos_no_target_cap) capNeed = -c.g(i, 'powerTransferAmount');
    const cycRaw = c.rawCycleMs(i);
    const full = cycRaw + fit.get(i, A.reactivation);
    const row: Record<string, unknown> = {
      module_index: it.reqIndex, type_id: it.typeId, name: c.typeName(i), slot: it.slot, state: STATE_NAMES[it.state],
      cpu: fit.get(i, A.cpu), power: fit.get(i, A.power),
    };
    if (cycRaw > 0) row.cycle_time_ms = cycRaw;
    if (c.active(i) && capNeed !== 0 && full > 0) {
      // Pyfa forces reload into capacitor boosters' average cycle (module.forceReload)
      const avg = c.avgCycleMs(i, factorReload || isInj);
      const use = avg > 0 ? capNeed / (avg / 1000) : 0;
      if (use > 0) used += use;
      else added -= use;
      row.cap_use_gj_s = use;
      drains.push({
        duration: Math.trunc(full), capNeed, clipSize: c.numShots(i), reloadMs: fit.get(i, A.reload),
        isInjector: isInj, disableStagger: c.hasEffect1(i, 'turretFitted'),
      });
    }
    rows.push(row);
  }
  // incoming neuts / nos / cap transfers (Pyfa fit.addDrain): no stagger, after the fit's own modules
  const sigNow = c.g(ship, 'signatureRadius');
  for (let k2 = 0; k2 < fit.projSpecial.length; k2++) {
    const ps = fit.projSpecial[k2];
    if (ps.kind !== 'drain') continue;
    let need = fit.get(ps.item, ps.amount) * ps.factor * ps.sign;
    if (ps.resist !== 0) need *= fit.get(ship, ps.resist);
    const sres = c.g(ps.item, 'energyNeutralizerSignatureResolution');
    if (sres !== 0) need *= Math.min(sigNow / sres, 1);
    const dur = fit.get(ps.item, ps.duration);
    if (need !== 0 && dur > 0) {
      const u = need / (Math.trunc(dur) / 1000);
      if (u > 0) used += u;
      else added -= u;
    }
    if (need !== 0 && dur > 0) drains.push({ duration: Math.trunc(dur), capNeed: need, clipSize: 0, reloadMs: 0, isInjector: false, disableStagger: false });
  }
  const j: Record<string, unknown> = {
    capacity: cap, recharge_time_s: rr / 1000, peak_recharge_gj_s: peak, use_gj_s: used, injected_gj_s: added, delta_gj_s: peak + added - used,
  };
  if (drains.length === 0) {
    j.stable = true;
    j.stable_percent = 100;
  } else {
    const o = req.options.cap_sim;
    const r = simulate(cap, rr, drains, 1, o.reload || factorReload, true, (o.max_time_s ?? 6 * 3600) * 1000);
    const st = (r.stableLow + r.stableHigh) / 2;
    j.stable = r.stable && st > 0;
    if (r.stable && st > 0) j.stable_percent = Math.min(st * 100, 100);
    else j.depletes_in_s = r.tS;
    j.eve_stable_percent = r.eveStable * 100;
    j.sim_iterations = r.iterations;
  }
  return { json: j, modules: rows, peak, used, added, stable: j.stable as boolean };
}
