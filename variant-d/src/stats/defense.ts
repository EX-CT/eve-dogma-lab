import type { CapInfo } from './capacitor.js';
import { StatsCtx } from './ctx.js';

/** resonance attribute names per layer (static strings: no per-calc string building) */
const RES_SHIELD = ['shieldEmDamageResonance', 'shieldThermalDamageResonance', 'shieldKineticDamageResonance', 'shieldExplosiveDamageResonance'] as const;
const RES_ARMOR = ['armorEmDamageResonance', 'armorThermalDamageResonance', 'armorKineticDamageResonance', 'armorExplosiveDamageResonance'] as const;
const RES_HULL = ['emDamageResonance', 'thermalDamageResonance', 'kineticDamageResonance', 'explosiveDamageResonance'] as const;

export function defense(c: StatsCtx, cap: CapInfo): object {
  const { fit, req } = c;
  const ship = fit.ship;
  const dp = req.damage_pattern ?? { em: 25, thermal: 25, kinetic: 25, explosive: 25 };
  const dpTot = Math.max(dp.em + dp.thermal + dp.kinetic + dp.explosive, 1e-12);
  const layer = (names: readonly string[]): number[] => names.map((n) => c.g(ship, n));
  const effectivify = (amount: number, r: number[]) => {
    const div = (dp.em * r[0] + dp.thermal * r[1] + dp.kinetic * r[2] + dp.explosive * r[3]) / dpTot;
    return div === 0 ? amount : amount / div;
  };
  const rs = layer(RES_SHIELD), ra = layer(RES_ARMOR), rh = layer(RES_HULL);
  const hpS = c.g(ship, 'shieldCapacity'), hpA = c.g(ship, 'armorHP'), hpH = fit.get(ship, 9);
  const eS = effectivify(hpS, rs), eA = effectivify(hpA, ra), eH = effectivify(hpH, rh);
  const resJson = (r: number[]) => ({ em: r[0], thermal: r[1], kinetic: r[2], explosive: r[3] });
  let shieldRep = 0, armorRep = 0, hullRep = 0;
  for (let k1 = 0; k1 < c.modules.length; k1++) {
    const i = c.modules[k1];
    if (!c.active(i)) continue;
    const dur = fit.get(i, c.A.duration) / 1000;
    if (dur <= 0) continue;
    if (c.hasEffect(i, ['shieldBoosting', 'fueledShieldBoosting'])) shieldRep += c.g(i, 'shieldBonus') / dur;
    if (c.hasEffect1(i, 'armorRepair')) armorRep += c.g(i, 'armorDamageAmount') / dur;
    if (c.hasEffect1(i, 'fueledArmorRepair')) {
      const ch = c.item(i).charge;
      const paste = ch >= 0 && c.ds.types.get(fit.items[ch].typeId)!.name === 'Nanite Repair Paste';
      armorRep += (c.g(i, 'armorDamageAmount') * (paste ? 3 : 1)) / dur;
    }
    if (c.hasEffect1(i, 'structureRepair')) hullRep += c.g(i, 'structureDamageAmount') / dur;
  }
  // incoming remote repairs (Pyfa __getAppliedRr diminishing-returns formula)
  const lists: [number, number][][] = [[], [], []];
  for (let k2 = 0; k2 < fit.projSpecial.length; k2++) {
    const ps = fit.projSpecial[k2];
    if (ps.kind !== 'rep') continue;
    const dur = fit.get(ps.item, c.A.duration) / 1000;
    if (dur > 0) lists[ps.layer].push([fit.get(ps.item, ps.amount) * ps.mult * ps.factor, dur]);
  }
  const applied = (l: [number, number][]) => {
    const total = l.reduce((s, [a, cy]) => s + a / Math.trunc(cy), 0);
    return l.reduce((s, [a, cy]) => {
      const rrps = a / Math.trunc(cy);
      const m = 7000 + rrps * 20;
      return s + ((1 - Math.pow((rrps + m) / (total + m) - 1, 2)) * a) / cy;
    }, 0);
  };
  shieldRep += applied(lists[0]);
  armorRep += applied(lists[1]);
  hullRep += applied(lists[2]);
  const rrS = c.g(ship, 'shieldRechargeRate') / 1000;
  const passive = rrS > 0 ? (10 / rrS) * 0.5 * 0.5 * hpS : 0;
  const sus = sustained(c, cap, [shieldRep, armorRep, hullRep]);
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
      sustained: { passive_shield: passive, shield_repair: sus[0], armor_repair: sus[1], hull_repair: sus[2] },
      sustained_effective: {
        passive_shield: effectivify(passive, rs), shield_repair: effectivify(sus[0], rs),
        armor_repair: effectivify(sus[1], ra), hull_repair: effectivify(sus[2], rh),
      },
    },
  };
}

const SUSTAIN_SPEC: Record<string, [number, string]> = {
  'Shield Booster': [0, 'shieldBonus'], 'Ancillary Shield Booster': [0, 'shieldBonus'],
  'Armor Repair Unit': [1, 'armorDamageAmount'], 'Ancillary Armor Repairer': [1, 'armorDamageAmount'],
  'Hull Repair Unit': [2, 'structureDamageAmount'],
};

/**
 * Sustainable tank (Pyfa Fit.sustainableTank, eos LGPL): when the capacitor is not stable (or reload is factored),
 * local cap-using repairers only run as far as peak recharge + injected cap allow (most cap-efficient first).
 */
function sustained(c: StatsCtx, cap: CapInfo, raw: number[]): number[] {
  const { fit, req, ds } = c;
  const factorReload = req.options.factor_reload;
  const sus = raw.slice();
  if (cap.stable && !factorReload) return sus;
  const A = c.A;
  const grp = (i: number) => ds.groups.get(c.item(i).group)?.name ?? '';
  const isPaste = (ch: number) => ch >= 0 && ds.types.get(fit.items[ch].typeId)!.name === 'Nanite Repair Paste';
  const pasteMult = (i: number) => { const m = c.g(i, 'chargedArmorDamageMultiplier'); return m === 0 ? 1 : m; };
  const adj = [0, 0, 0];
  let used = cap.used;
  const reps: [number, number, string, number][] = [];
  for (let layer = 0; layer < 3; layer++) {
    for (let k3 = 0; k3 < c.modules.length; k3++) {
      const i = c.modules[k3];
      if (!c.active(i)) continue;
      const g = grp(i);
      const spec = SUSTAIN_SPEC[g];
      if (!spec || spec[0] !== layer) continue;
      const [l, attr] = spec;
      const capNeed = fit.get(i, A.capNeed);
      const avg = c.avgCycleMs(i, factorReload);
      const capUse = capNeed !== 0 && avg > 0 ? capNeed / (avg / 1000) : 0;
      const cyc = c.rawCycleMs(i);
      if (cyc <= 0) continue;
      const amount = c.g(i, attr);
      const ch = c.item(i).charge;
      if (capUse !== 0) {
        used -= capUse;
        adj[l] -= (amount * (isPaste(ch) ? pasteMult(i) : 1)) / (cyc / 1000);
        reps.push([i, l, attr, capUse]);
      } else if (g === 'Ancillary Shield Booster') {
        const reload = factorReload && ch >= 0 ? fit.get(i, A.reload) : 0;
        const shots = Math.max(c.numShots(i), 1);
        const off = reload / (shots * cyc + reload);
        adj[l] -= (amount * off) / (cyc / 1000);
      }
    }
  }
  const eff = (i: number, attr: string) => (c.g(i, attr) * pasteMult(i)) / fit.get(i, A.capNeed);
  // stable sort, descending efficiency (Rust sort_by is stable)
  const keyed = reps.map((r, k) => [eff(r[0], r[2]), k, r] as const);
  keyed.sort((a, b) => (b[0] > a[0] ? 1 : b[0] < a[0] ? -1 : a[1] - b[1]));
  const totalPeak = cap.peak + cap.added;
  for (const [, , [i, l, attr, capUse]] of keyed) {
    if (used > totalPeak) break;
    const ch = c.item(i).charge;
    const reload = factorReload && ch >= 0 ? fit.get(i, A.reload) : 0;
    const cyc = c.rawCycleMs(i);
    const sustain = Math.min((totalPeak - used) / capUse, 1);
    const amount = c.g(i, attr);
    if (ch < 0) adj[l] += (sustain * amount) / (cyc / 1000);
    else {
      const mult = isPaste(ch) ? pasteMult(i) : 1;
      const shots = Math.max(c.numShots(i), 1);
      const on = (shots * cyc) / (shots * cyc + reload);
      adj[l] += (sustain * amount * on * mult) / (cyc / 1000);
    }
    used += capUse;
  }
  for (let l = 0; l < 3; l++) sus[l] += adj[l];
  return sus;
}
