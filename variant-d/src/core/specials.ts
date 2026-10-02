/**
 * Special effects: effects CCP ships without (usable) modifierInfo. One registry entry per effect.
 * Everything else is driven by the SDE modifierInfo (+ eve-sde-pipeline data patches).
 */
import type { Fit } from './fit.js';
import { SrcK } from './graph.js';
import { rangeFactor } from '../stats/util.js';

export interface SpecialCtx {
  fit: Fit;
  /** item owning the effect */
  item: number;
  /** source category used for stacking-penalty exemption */
  cat: number;
}
export type SpecialHandler = (c: SpecialCtx) => void;

const LOCAL = new Map<string, SpecialHandler>();
/** register a local special effect by effect name */
export function special(name: string, h: SpecialHandler): void {
  LOCAL.set(name, h);
}
export function localSpecial(name: string): SpecialHandler | undefined {
  return LOCAL.get(name);
}

// --- propulsion: AB / MWD -------------------------------------------------------------
const propulsion = (mwd: boolean): SpecialHandler => ({ fit, item, cat }) => {
  const ds = fit.ds;
  const ship = fit.ship;
  fit.pushAttr(ship, 4, 2, item, ds.attrId('massAddition'), cat);
  // speed: PostMul by 1 + speedFactor/100 * speedBoostFactor / mass
  fit.push(ship, ds.attrId('maxVelocity'), 4, { k: SrcK.Prop, item, attr: ds.attrId('speedFactor'), a2: ds.attrId('speedBoostFactor'), a3: ship }, item, cat);
  if (mwd) fit.pushAttr(ship, ds.attrId('signatureRadius'), 6, item, ds.attrId('signatureRadiusBonus'), cat);
};
special('moduleBonusAfterburner', propulsion(false));
special('moduleBonusMicrowarpdrive', propulsion(true));

// --- MJD: sig bloom, not stacking penalised (exempt category 6 = ship) ------------------
special('microJumpDrive', ({ fit, item }) => {
  const ds = fit.ds;
  fit.pushAttr(fit.ship, ds.attrId('signatureRadius'), 6, item, ds.attrId('signatureRadiusBonusPercent'), 6);
});

// --- T3C subsystems: slots and hardpoints ------------------------------------------------
special('slotModifier', ({ fit, item, cat }) => {
  for (const [t, s] of [['hiSlots', 'hiSlotModifier'], ['medSlots', 'medSlotModifier'], ['lowSlots', 'lowSlotModifier']]) {
    fit.pushAttr(fit.ship, fit.ds.attrId(t), 2, item, fit.ds.attrId(s), cat);
  }
});
special('hardPointModifierEffect', ({ fit, item, cat }) => {
  for (const [t, s] of [['turretSlotsLeft', 'turretHardPointModifier'], ['launcherSlotsLeft', 'launcherHardPointModifier']]) {
    fit.pushAttr(fit.ship, fit.ds.attrId(t), 2, item, fit.ds.attrId(s), cat);
  }
});

// --- fighter self abilities (Pyfa hand-written handlers, eos LGPL): modify the squadron itself --------------------
const fighterSelf = (mods: [string, string, number][]): SpecialHandler => ({ fit, item, cat }) => {
  for (const [t, a, op] of mods) fit.pushAttr(item, fit.ds.attrId(t), op, item, fit.ds.attrId(a), cat);
};
special('fighterAbilityMicroWarpDrive', fighterSelf([
  ['maxVelocity', 'fighterAbilityMicroWarpDriveSpeedBonus', 6], ['signatureRadius', 'fighterAbilityMicroWarpDriveSignatureRadiusBonus', 6],
]));
special('fighterAbilityAfterburner', fighterSelf([['maxVelocity', 'fighterAbilityAfterburnerSpeedBonus', 6]]));
special('fighterAbilityEvasiveManeuvers', fighterSelf([
  ['maxVelocity', 'fighterAbilityEvasiveManeuversSpeedBonus', 6], ['signatureRadius', 'fighterAbilityEvasiveManeuversSignatureRadiusBonus', 6],
  ['shieldEmDamageResonance', 'fighterAbilityEvasiveManeuversEmResonance', 4], ['shieldThermalDamageResonance', 'fighterAbilityEvasiveManeuversThermResonance', 4],
  ['shieldKineticDamageResonance', 'fighterAbilityEvasiveManeuversKinResonance', 4], ['shieldExplosiveDamageResonance', 'fighterAbilityEvasiveManeuversExpResonance', 4],
]));

// --- projected effects without modifierInfo -----------------------------------------------
/** [target attribute name, source attribute name, operator] */
type ProjMap = [string, string, number][];
const PROJECTED: { match: (name: string) => boolean; mods: ProjMap }[] = [
  { match: (n) => n.startsWith('remoteWebifier') || n === 'structureModuleEffectStasisWebifier', mods: [['maxVelocity', 'speedFactor', 6]] },
  { match: (n) => n.startsWith('remoteTargetPaint') || n === 'structureModuleEffectTargetPainter', mods: [['signatureRadius', 'signatureRadiusBonus', 6]] },
  {
    match: (n) => n.startsWith('remoteSensorDamp') || n === 'structureModuleEffectRemoteSensorDampener',
    mods: [['maxTargetRange', 'maxTargetRangeBonus', 6], ['scanResolution', 'scanResolutionBonus', 6]],
  },
  {
    match: (n) => n.startsWith('remoteSensorBoost'),
    mods: [
      ['maxTargetRange', 'maxTargetRangeBonus', 6], ['scanResolution', 'scanResolutionBonus', 6],
      ...(['Gravimetric', 'Ladar', 'Magnetometric', 'Radar'].map((t) => [`scan${t}Strength`, `scan${t}StrengthPercent`, 6]) as ProjMap),
    ],
  },
];
export function projectedSpecial(name: string): ProjMap | null {
  for (const p of PROJECTED) if (p.match(name)) return p.mods;
  return null;
}

// --- projected effects that feed tank / capacitor stats instead of attributes (Pyfa fit._xxxRr, addDrain) ----
export type ProjSpecial =
  /** layer 0 shield, 1 armor, 2 hull: attr `amount` * mult * factor every cycle */
  | { kind: 'rep'; item: number; layer: 0 | 1 | 2; amount: number; mult: number; factor: number }
  /** capacitor drain (sign +1) or fill (sign -1) per cycle of attr `duration` */
  | { kind: 'drain'; item: number; amount: number; duration: number; factor: number; resist: number; sign: number }
  /** ECM jam strength vs the target's strongest sensor type (Pyfa addProjectedEcm / jamChance) */
  | { kind: 'ecm'; item: number; fighter: boolean; factor: number; resist: number };

/** weapon damage / mining projected onto the target: not part of the target's own stats */
export const PROJECTED_DAMAGE_EFFECTS = new Set([
  'projectileFired', 'targetAttack', 'useMissiles', 'barrage', 'targetDisintegratorAttack', 'missileLaunchingForEntity',
  'fighterAbilityAttackM', 'fighterAbilityMissiles', 'superWeaponAmarr', 'superWeaponCaldari', 'superWeaponGallente',
  'superWeaponMinmatar', 'mining', 'miningLaser', 'miningClouds', 'dotMissileLaunching',
]);

type FeedFn = (h: FeedHelpers) => ProjSpecial[];
interface FeedHelpers {
  rep: (layer: 0 | 1 | 2, amount: string, mult: number, factor: number) => ProjSpecial[];
  drain: (amount: string, duration: string, factor: number, sign: number) => ProjSpecial[];
  ecm: (fighter: boolean, factor: number) => ProjSpecial[];
  falloff: () => number;
  gate: (attr: string) => number;
  /** range factor from named optimal / falloff attributes */
  rf: (opt: string, falloff: string) => number;
  /** squadron size (projected fighters), >= 1 */
  qty: number;
  paste: boolean;
  noAssist: boolean;
}
const FEEDS: Record<string, FeedFn> = {
  shipModuleRemoteShieldBooster: (h) => h.rep(0, 'shieldBonus', 1, h.falloff()),
  shipModuleAncillaryRemoteShieldBooster: (h) => h.rep(0, 'shieldBonus', 1, h.falloff()),
  shipModuleRemoteArmorRepairer: (h) => h.rep(1, 'armorDamageAmount', 1, h.falloff()),
  ShipModuleRemoteArmorMutadaptiveRepairer: (h) => h.rep(1, 'armorDamageAmount', 1, h.falloff()),
  shipModuleAncillaryRemoteArmorRepairer: (h) => h.rep(1, 'armorDamageAmount', h.paste ? 3 : 1, h.falloff()),
  shipModuleRemoteHullRepairer: (h) => h.rep(2, 'structureDamageAmount', 1, h.falloff()),
  npcEntityRemoteShieldBooster: (h) => h.rep(0, 'shieldBonus', 1, h.gate('maxRange')),
  npcEntityRemoteArmorRepairer: (h) => h.rep(1, 'armorDamageAmount', 1, h.gate('maxRange')),
  npcEntityRemoteHullRepairer: (h) => h.rep(2, 'structureDamageAmount', 1, h.gate('maxRange')),
  shipModuleRemoteCapacitorTransmitter: (h) => (h.noAssist ? [] : h.drain('powerTransferAmount', 'duration', h.gate('maxRange'), -1)),
  energyNeutralizerFalloff: (h) => h.drain('energyNeutralizerAmount', 'duration', h.falloff(), 1),
  energyNosferatuFalloff: (h) => h.drain('powerTransferAmount', 'duration', h.falloff(), 1),
  structureEnergyNeutralizerFalloff: (h) => h.drain('energyNeutralizerAmount', 'duration', 1, 1),
  entityEnergyNeutralizerFalloff: (h) => h.drain('energyNeutralizerAmount', 'energyNeutralizerDuration', h.gate('energyNeutralizerRangeOptimal'), 1),
  fighterAbilityEnergyNeutralizer: (h) => h.drain('fighterAbilityEnergyNeutralizerAmount', 'fighterAbilityEnergyNeutralizerDuration',
    h.rf('fighterAbilityEnergyNeutralizerOptimalRange', 'fighterAbilityEnergyNeutralizerFalloffRange') * h.qty, 1),
  remoteECMFalloff: (h) => h.ecm(false, h.falloff()),
  structureModuleEffectECM: (h) => h.ecm(false, h.falloff()),
  entityECMFalloff: (h) => h.ecm(false, h.gate('ECMRangeOptimal')),
  ECMBurstJammer: (h) => h.ecm(false, h.gate('ecmBurstRange')),
  fighterAbilityECM: (h) => h.ecm(true, h.rf('fighterAbilityECMRangeOptimal', 'fighterAbilityECMRangeFalloff') * h.qty),
};

/** Pyfa's projected handlers for remote reps, cap transfers and neuts/nos (eos/effects.py, LGPL); null = not a feed effect */
export function projectedFeed(fit: Fit, item: number, name: string, resist: number): ProjSpecial[] | null {
  const f = FEEDS[name];
  if (!f) return null;
  const ds = fit.ds;
  const it = fit.items[item];
  const base = (n: string) => { const a = ds.attrId(n); return fit.has(item, a) ? fit.base(item, a) : 0; };
  const dist = it.distance;
  const da = ds.attrId('disallowAssistance');
  const noAssist = fit.has(fit.ship, da) && fit.base(fit.ship, da) !== 0;
  const dof = ds.attrId('disallowOffensiveModifiers');
  const noOffense = fit.has(fit.ship, dof) && fit.base(fit.ship, dof) !== 0;
  const paste = it.charge >= 0 && ds.types.get(fit.items[it.charge].typeId)?.name === 'Nanite Repair Paste';
  return f({
    noAssist, paste, qty: Math.max(it.quantity, 1),
    rf: (o, fo) => rangeFactor(base(o), base(fo), dist, true),
    ecm: (fighter, factor) => (noOffense ? [] : [{ kind: 'ecm', item, fighter, factor, resist }]),
    falloff: () => rangeFactor(base('maxRange'), base('falloffEffectiveness'), dist, true),
    gate: (attr) => (base(attr) < (dist ?? 0) ? 0 : 1),
    rep: (layer, amount, mult, factor) => (noAssist ? [] : [{ kind: 'rep', item, layer, amount: ds.attrId(amount), mult, factor }]),
    drain: (amount, duration, factor, sign) => [{ kind: 'drain', item, amount: ds.attrId(amount), duration: ds.attrId(duration), factor, resist, sign }],
  });
}
