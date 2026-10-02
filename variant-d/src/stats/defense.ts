import { StatsCtx } from './ctx.js';

export function defense(c: StatsCtx): object {
  const { fit, req } = c;
  const ship = fit.ship;
  const dp = req.damage_pattern ?? { em: 25, thermal: 25, kinetic: 25, explosive: 25 };
  const dpTot = Math.max(dp.em + dp.thermal + dp.kinetic + dp.explosive, 1e-12);
  const layer = (prefix: string): number[] =>
    (prefix === '' ? ['emDamageResonance', 'thermalDamageResonance', 'kineticDamageResonance', 'explosiveDamageResonance']
      : ['Em', 'Thermal', 'Kinetic', 'Explosive'].map((d) => `${prefix}${d}DamageResonance`)).map((n) => c.g(ship, n));
  const effectivify = (amount: number, r: number[]) => {
    const div = (dp.em * r[0] + dp.thermal * r[1] + dp.kinetic * r[2] + dp.explosive * r[3]) / dpTot;
    return div === 0 ? amount : amount / div;
  };
  const rs = layer('shield'), ra = layer('armor'), rh = layer('');
  const hpS = c.g(ship, 'shieldCapacity'), hpA = c.g(ship, 'armorHP'), hpH = fit.get(ship, 9);
  const eS = effectivify(hpS, rs), eA = effectivify(hpA, ra), eH = effectivify(hpH, rh);
  const resJson = (r: number[]) => ({ em: r[0], thermal: r[1], kinetic: r[2], explosive: r[3] });
  let shieldRep = 0, armorRep = 0, hullRep = 0;
  for (const i of c.modules) {
    if (!c.active(i)) continue;
    const dur = fit.get(i, c.A.duration) / 1000;
    if (dur <= 0) continue;
    if (c.hasEffect(i, ['shieldBoosting', 'fueledShieldBoosting'])) shieldRep += c.g(i, 'shieldBonus') / dur;
    if (c.hasEffect(i, ['armorRepair'])) armorRep += c.g(i, 'armorDamageAmount') / dur;
    if (c.hasEffect(i, ['fueledArmorRepair'])) {
      const ch = c.item(i).charge;
      const paste = ch >= 0 && c.ds.types.get(fit.items[ch].typeId)!.name === 'Nanite Repair Paste';
      armorRep += (c.g(i, 'armorDamageAmount') * (paste ? 3 : 1)) / dur;
    }
    if (c.hasEffect(i, ['structureRepair'])) hullRep += c.g(i, 'structureDamageAmount') / dur;
  }
  const rrS = c.g(ship, 'shieldRechargeRate') / 1000;
  const passive = rrS > 0 ? (10 / rrS) * 0.5 * 0.5 * hpS : 0;
  return {
    hp: { shield: hpS, armor: hpA, hull: hpH, total: hpS + hpA + hpH },
    resonance: { shield: resJson(rs), armor: resJson(ra), hull: resJson(rh) },
    ehp: { shield: eS, armor: eA, hull: eH, total: eS + eA + eH },
    damage_pattern: { em: dp.em, thermal: dp.thermal, kinetic: dp.kinetic, explosive: dp.explosive },
    tank: {
      raw: { passive_shield: passive, shield_repair: shieldRep, armor_repair: armorRep, hull_repair: hullRep },
      effective: {
        passive_shield: effectivify(passive, rs), shield_repair: effectivify(shieldRep, rs),
        armor_repair: effectivify(armorRep, ra), hull_repair: effectivify(hullRep, rh),
      },
    },
  };
}
