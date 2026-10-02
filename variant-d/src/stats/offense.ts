import { Resists, Spool } from '../core/request.js';
import { StatsCtx } from './ctx.js';
import { Dmg, floatUnerr, spoolup } from './util.js';
import { pyFloatUnerr } from '../core/operators.js';

function weaponKind(c: StatsCtx, i: number): string {
  if (c.hasEffect1(i, 'turretFitted')) return 'turret';
  if (c.hasEffect1(i, 'launcherFitted')) return 'missile';
  if (c.hasEffect1(i, 'empWave')) return 'smartbomb';
  if (c.hasEffect1(i, 'ChainLightning')) return 'vorton';
  return 'other';
}

function moduleVolley(c: StatsCtx, i: number): [Dmg, string] {
  const { fit, A } = c;
  const it = c.item(i);
  const kind = weaponKind(c, i);
  const src = it.charge >= 0 ? it.charge : i;
  let mult = fit.has(i, A.dmgMult) ? fit.get(i, A.dmgMult) : 1;
  // missile damage is scaled by the pilot's missileDamageMultiplier
  if (kind === 'missile' && it.charge >= 0) mult *= c.g(fit.char, 'missileDamageMultiplier');
  return [new Dmg(fit.get(src, A.dmg[0]) * mult, fit.get(src, A.dmg[1]) * mult, fit.get(src, A.dmg[2]) * mult, fit.get(src, A.dmg[3]) * mult), kind];
}

export function offense(c: StatsCtx): object {
  const { fit, req, A, ds } = c;
  const tp = req.target_profile ?? {};
  const tpRes: Resists = { em: tp.em ?? 0, thermal: tp.thermal ?? 0, kinetic: tp.kinetic ?? 0, explosive: tp.explosive ?? 0 };
  const defaultSpool: Spool = req.options.default_spool ?? { type: 'spool_scale', amount: 1 };
  const factorReload = req.options.factor_reload;
  const weapons: object[] = [];
  const wVol = new Dmg(), wDps = new Dmg();
  for (const i of c.modules) {
    if (!c.active(i)) continue;
    const [base, kind] = moduleVolley(c, i);
    if (base.total() === 0) continue;
    const cyc = c.avgCycleMs(i, factorReload);
    const raw = c.rawCycleMs(i);
    const spool = c.item(i).spool ?? defaultSpool;
    const [sp] = spoolup(c.g(i, 'damageMultiplierBonusMax'), c.g(i, 'damageMultiplierBonusPerCycle'), raw / 1000, spool);
    const vol = base.scale(1 + sp);
    // doomsdays / lances deal their volley every doomsdayDamageCycleTime during doomsdayDamageDuration (Pyfa
    // getVolleyParameters subcycles; the Reaper slash hits once); volley = one tick
    const dd = c.g(i, 'doomsdayDamageDuration'), dsub = c.g(i, 'doomsdayDamageCycleTime');
    const subcycles = dd !== 0 && dsub !== 0 && !c.hasEffect1(i, 'doomsdaySlash') ? Math.max(Math.floor(pyFloatUnerr(dd / dsub)), 0) : 1;
    const dps = cyc > 0 ? vol.scale((subcycles * 1000) / cyc) : new Dmg();
    wVol.add(vol);
    wDps.add(dps);
    const it = c.item(i);
    const w: Record<string, unknown> = {
      module_index: it.reqIndex, type_id: it.typeId, name: c.typeName(i), kind,
      charge_type_id: it.charge >= 0 ? fit.items[it.charge].typeId : null,
      volley: vol.json(), dps: dps.json(), cycle_time_ms: cyc,
    };
    if (kind === 'turret') {
      w.optimal_m = c.g(i, 'maxRange');
      w.falloff_m = c.g(i, 'falloff');
      w.tracking = c.g(i, 'trackingSpeed');
    } else if (kind === 'missile') {
      if (it.charge >= 0) {
        const ch = it.charge;
        const r = missileRange(c, ch);
        if (r !== null) w.range_m = r;
        w.explosion_radius = c.g(ch, 'aoeCloudSize');
        w.explosion_velocity = c.g(ch, 'aoeVelocity');
      }
    } else if (kind === 'smartbomb') w.range_m = c.g(i, 'empFieldRange');
    if (sp > 0) {
      w.spool_multiplier = 1 + sp;
      w.volley_unspooled = base.json();
    }
    weapons.push(w);
  }
  const dVol = new Dmg(), dDps = new Dmg();
  const droneOut: object[] = [];
  for (const i of c.drones) {
    const n = c.item(i).activeCount;
    if (n === 0) continue;
    const mult = fit.has(i, A.dmgMult) ? fit.get(i, A.dmgMult) : 1;
    const v = new Dmg(fit.get(i, A.dmg[0]), fit.get(i, A.dmg[1]), fit.get(i, A.dmg[2]), fit.get(i, A.dmg[3])).scale(mult * n);
    const cyc = c.rawCycleMs(i);
    if (v.total() === 0 || cyc === 0) continue;
    const dps = v.scale(1000 / cyc);
    dVol.add(v);
    dDps.add(dps);
    droneOut.push({ drone_index: c.item(i).reqIndex, type_id: c.item(i).typeId, name: c.typeName(i), count: n, volley: v.json(), dps: dps.json(),
      optimal_m: c.g(i, 'maxRange'), falloff_m: c.g(i, 'falloff'), tracking: c.g(i, 'trackingSpeed'),
      max_velocity: c.g(i, 'maxVelocity'), signature_radius: c.g(i, 'signatureRadius') });
  }
  const fVol = new Dmg(), fDps = new Dmg();
  const fighterOut: object[] = [];
  for (const i of c.fighters) {
    const it = c.item(i);
    const n = it.activeCount;
    if (n === 0) continue;
    const fv = new Dmg(), fd = new Dmg();
    for (const [eff, prefix] of [['fighterAbilityAttackM', 'fighterAbilityAttackMissile'], ['fighterAbilityMissiles', 'fighterAbilityMissiles']]) {
      const eid = ds.effectId(eff);
      const found = it.effects.find(([e]) => e === eid);
      if (!found) continue;
      const used = it.fighterAbilities !== null ? it.fighterAbilities.includes(eid) : found[1] !== 0;
      if (!used) continue;
      let m = c.g(i, `${prefix}DamageMultiplier`);
      if (m === 0) m = 1;
      const v = new Dmg(c.g(i, `${prefix}DamageEM`), c.g(i, `${prefix}DamageTherm`), c.g(i, `${prefix}DamageKin`), c.g(i, `${prefix}DamageExp`)).scale(m * n);
      const dur = c.g(i, `${prefix}Duration`);
      fv.add(v);
      if (dur > 0) fd.add(v.scale(1000 / dur));
    }
    if (fv.total() > 0) {
      fVol.add(fv);
      fDps.add(fd);
      fighterOut.push({ fighter_index: it.reqIndex, type_id: it.typeId, name: c.typeName(i), squadron_size: n, volley: fv.json(), dps: fd.json(),
        max_velocity: c.g(i, 'maxVelocity'), signature_radius: c.g(i, 'signatureRadius') });
    }
  }
  const tVol = wVol.clone(); tVol.add(dVol); tVol.add(fVol);
  const tDps = wDps.clone(); tDps.add(dDps); tDps.add(fDps);
  return {
    weapons, drones: droneOut, fighters: fighterOut,
    total: {
      weapon_dps: wDps.total(), weapon_volley: wVol.total(), drone_dps: dDps.total(), drone_volley: dVol.total(),
      fighter_dps: fDps.total(), fighter_volley: fVol.total(), dps: tDps.json(), volley: tVol.json(),
    },
    vs_target_profile: { dps: tDps.vs(tpRes), volley: tVol.vs(tpRes) },
  };
}

/**
 * Pyfa missileMaxRangeData: flight time + ship radius bonus, acceleration phase, floor/ceil blend, FoF limit,
 * centre-to-surface (eos/saveddata/module.py, LGPL).
 */
function missileRange(c: StatsCtx, ch: number): number | null {
  const vel = c.g(ch, 'maxVelocity');
  if (!(vel > 0)) return null;
  const radius = c.g(c.fit.ship, 'radius');
  const ft = floatUnerr(c.g(ch, 'explosionDelay') / 1000 + radius / vel);
  const accelCap = (c.g(ch, 'mass') * c.g(ch, 'agility')) / 1e6;
  const rangeAt = (t: number) => { const acc = Math.min(t, accelCap); return (vel / 2) * acc + vel * (t - acc); };
  const lt = Math.floor(ft), ht = Math.ceil(ft);
  let lr = rangeAt(lt), hr = rangeAt(ht);
  if (c.hasEffect1(ch, 'fofMissileLaunching')) {
    const lim = c.g(ch, 'maxFOFTargetRange');
    if (lim > 0) { lr = Math.min(lr, lim); hr = Math.min(hr, lim); }
  }
  lr = Math.max(lr - radius, 0);
  hr = Math.max(hr - radius, 0);
  const hc = ft - lt;
  return lr * (1 - hc) + hr * hc;
}
