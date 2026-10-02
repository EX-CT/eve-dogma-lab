/**
 * Special effects: effects CCP ships without (usable) modifierInfo. One registry entry per effect.
 * Everything else is driven by the SDE modifierInfo (+ eve-sde-pipeline data patches).
 */
import type { Fit } from './fit.js';
import { SrcK } from './graph.js';

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

// --- projected effects without modifierInfo -----------------------------------------------
/** [target attribute name, source attribute name, operator] */
type ProjMap = [string, string, number][];
const PROJECTED: { match: (name: string) => boolean; mods: ProjMap }[] = [
  { match: (n) => n.startsWith('remoteWebifier') || n === 'structureModuleEffectStasisWebifier', mods: [['maxVelocity', 'speedFactor', 6]] },
  { match: (n) => n.startsWith('remoteTargetPaint') || n === 'structureModuleEffectTargetPainter', mods: [['signatureRadius', 'signatureRadiusBonus', 6]] },
  {
    match: (n) => n.startsWith('remoteSensorDamp') || n === 'structureModuleEffectRemoteSensorDampener' || n.startsWith('remoteSensorBoost'),
    mods: [['maxTargetRange', 'maxTargetRangeBonus', 6], ['scanResolution', 'scanResolutionBonus', 6]],
  },
];
export function projectedSpecial(name: string): ProjMap | null {
  for (const p of PROJECTED) if (p.match(name)) return p.mods;
  return null;
}
