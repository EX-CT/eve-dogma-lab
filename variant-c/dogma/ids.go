package dogma

import "fmt"

// wellKnown caches attribute/effect ids resolved once per dataset.
type wellKnown struct {
	eAB, eMWD, eSlot, eHardpoint, eMJD, eBastion                       uint32
	structureSkillEffects                                              [5]uint32
	massAddition, maxVelocity, speedFactor, speedBoostFactor           uint32
	signatureRadius, signatureRadiusBonus, signatureRadiusBonusPercent uint32
	slotAttrs, slotModAttrs                                            [3]uint32
	hpAttrs, hpModAttrs                                                [2]uint32
	warfareID, warfareVal                                              [4]uint32

	// stats
	cpu, power, cpuOut, powerOut, upgradeCost, upgradeCap        uint32
	speed, duration, capNeed, reload, reactivation, chargeRate   uint32
	dmgMult                                                      uint32
	dmg                                                          [4]uint32
	durationExtra                                                []uint32
	eTurret, eLauncher, eEmpWave, eChain                         uint32
	eShieldBoost, eFueledShieldBoost, eArmorRep, eFueledArmorRep uint32
	eHullRep, eNos                                               uint32
	eFighterAttackM, eFighterMissiles                            uint32
}

func newWellKnown(ds *Dataset) wellKnown {
	a, e := ds.AttrID, ds.EffectID
	w := wellKnown{
		eAB: e("moduleBonusAfterburner"), eMWD: e("moduleBonusMicrowarpdrive"), eSlot: e("slotModifier"),
		eHardpoint: e("hardPointModifierEffect"), eMJD: e("microJumpDrive"), eBastion: e("moduleBonusBastionModule"),
		massAddition: a("massAddition"), maxVelocity: a("maxVelocity"), speedFactor: a("speedFactor"),
		speedBoostFactor: a("speedBoostFactor"), signatureRadius: a("signatureRadius"),
		signatureRadiusBonus: a("signatureRadiusBonus"), signatureRadiusBonusPercent: a("signatureRadiusBonusPercent"),
		slotAttrs:    [3]uint32{a("hiSlots"), a("medSlots"), a("lowSlots")},
		slotModAttrs: [3]uint32{a("hiSlotModifier"), a("medSlotModifier"), a("lowSlotModifier")},
		hpAttrs:      [2]uint32{a("turretSlotsLeft"), a("launcherSlotsLeft")},
		hpModAttrs:   [2]uint32{a("turretHardPointModifier"), a("launcherHardPointModifier")},
		cpu:          a("cpu"), power: a("power"), cpuOut: a("cpuOutput"), powerOut: a("powerOutput"),
		upgradeCost: a("upgradeCost"), upgradeCap: a("upgradeCapacity"), speed: a("speed"), duration: a("duration"),
		capNeed: a("capacitorNeed"), reload: a("reloadTime"), reactivation: a("moduleReactivationDelay"),
		chargeRate: a("chargeRate"), dmgMult: a("damageMultiplier"),
		dmg:     [4]uint32{a("emDamage"), a("thermalDamage"), a("kineticDamage"), a("explosiveDamage")},
		eTurret: e("turretFitted"), eLauncher: e("launcherFitted"), eEmpWave: e("empWave"), eChain: e("ChainLightning"),
		eShieldBoost: e("shieldBoosting"), eFueledShieldBoost: e("fueledShieldBoosting"), eArmorRep: e("armorRepair"),
		eFueledArmorRep: e("fueledArmorRepair"), eHullRep: e("structureRepair"), eNos: e("energyNosferatuFalloff"),
		eFighterAttackM: e("fighterAbilityAttackM"), eFighterMissiles: e("fighterAbilityMissiles"),
	}
	for k, n := range structureSkillEffectNames {
		w.structureSkillEffects[k] = e(n)
	}
	for k := 0; k < 4; k++ {
		w.warfareID[k] = a(fmt.Sprintf("warfareBuff%dID", k+1))
		w.warfareVal[k] = a(fmt.Sprintf("warfareBuff%dValue", k+1))
	}
	for _, n := range []string{"durationHighisGood", "durationSensorDampeningBurstProjector", "durationTargetIlluminationBurstProjector",
		"durationECMJammerBurstProjector", "durationWeaponDisruptionBurstProjector"} {
		if x := a(n); x != 0 {
			w.durationExtra = append(w.durationExtra, x)
		}
	}
	return w
}
