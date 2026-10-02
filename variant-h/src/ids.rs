//! Attribute / effect ids resolved by name once at dataset load (no magic numbers in the systems).
use rustc_hash::FxHashMap;

macro_rules! id_table {
    ($name:ident { $($f:ident = $s:expr),* $(,)? } arrays { $($af:ident : [$n:expr] = $fmt:expr),* $(,)? }) => {
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
        pub struct $name { $(pub $f: u32,)* $(pub $af: [u32; $n],)* }
        impl $name {
            pub fn resolve(m: &FxHashMap<String, u32>) -> Self {
                let g = |s: &str| *m.get(s).unwrap_or(&0);
                Self {
                    $($f: g($s),)*
                    $($af: { let names: [String; $n] = $fmt; let mut out = [0u32; $n]; for (i, s) in names.iter().enumerate() { out[i] = g(s); } out },)*
                }
            }
        }
    };
}

id_table!(AttrIds {
    mass = "mass", capacity = "capacity", volume = "volume", hp = "hp",
    mass_addition = "massAddition", speed_factor = "speedFactor", speed_boost_factor = "speedBoostFactor",
    max_velocity = "maxVelocity", sig_bonus = "signatureRadiusBonus", sig = "signatureRadius",
    sig_bonus_percent = "signatureRadiusBonusPercent",
    hi_slots = "hiSlots", hi_slot_mod = "hiSlotModifier", med_slots = "medSlots", med_slot_mod = "medSlotModifier",
    low_slots = "lowSlots", low_slot_mod = "lowSlotModifier", rig_slots = "rigSlots", max_subsystems = "maxSubSystems",
    service_slots = "serviceSlots",
    turret_slots = "turretSlotsLeft", turret_hp_mod = "turretHardPointModifier",
    launcher_slots = "launcherSlotsLeft", launcher_hp_mod = "launcherHardPointModifier",
    remote_resistance_id = "remoteResistanceID", max_target_range = "maxTargetRange",
    max_target_range_bonus = "maxTargetRangeBonus", scan_resolution = "scanResolution",
    scan_resolution_bonus = "scanResolutionBonus", resistance_shift = "resistanceShiftAmount",
    pilot_sec = "pilotSecurityStatus", fighter_sq_max = "fighterSquadronMaxSize",
    hisec_mod = "hiSecModifier", lowsec_mod = "lowSecModifier", nullsec_mod = "nullSecModifier", sec_mod = "securityModifier",
    skill_level = "skillLevel",
    cpu = "cpu", power = "power", cpu_out = "cpuOutput", power_out = "powerOutput",
    upgrade_cost = "upgradeCost", upgrade_cap = "upgradeCapacity", speed = "speed", duration = "duration",
    cap_need = "capacitorNeed", reload = "reloadTime", reactivation = "moduleReactivationDelay", charge_rate = "chargeRate",
    dmg_mult = "damageMultiplier",
    crystals_get_damaged = "crystalsGetDamaged", crystal_vol_chance = "crystalVolatilityChance",
    crystal_vol_damage = "crystalVolatilityDamage", missile_dmg_mult = "missileDamageMultiplier",
    drone_bw_used = "droneBandwidthUsed", drone_bw = "droneBandwidth", drone_capacity = "droneCapacity",
    fighter_capacity = "fighterCapacity", fighter_heavy = "fighterSquadronIsHeavy", fighter_support = "fighterSquadronIsSupport",
    fighter_tubes = "fighterTubes", fighter_light_slots = "fighterLightSlots", fighter_support_slots = "fighterSupportSlots",
    fighter_heavy_slots = "fighterHeavySlots",
    spool_max = "damageMultiplierBonusMax", spool_step = "damageMultiplierBonusPerCycle",
    max_range = "maxRange", falloff = "falloff", tracking = "trackingSpeed", explosion_delay = "explosionDelay",
    aoe_cloud = "aoeCloudSize", aoe_velocity = "aoeVelocity", emp_range = "empFieldRange",
    shield_capacity = "shieldCapacity", armor_hp = "armorHP", shield_bonus = "shieldBonus",
    armor_dmg_amount = "armorDamageAmount", structure_dmg_amount = "structureDamageAmount",
    shield_recharge = "shieldRechargeRate", cap_capacity = "capacitorCapacity", recharge_rate = "rechargeRate",
    capacitor_bonus = "capacitorBonus", power_transfer = "powerTransferAmount", speed_limit = "speedLimit",
    agility = "agility", base_warp = "baseWarpSpeed", warp_mult = "warpSpeedMultiplier", warp_cap_need = "warpCapacitorNeed",
    warp_scramble = "warpScrambleStatus", max_locked = "maxLockedTargets", max_active_drones = "maxActiveDrones",
    drone_control = "droneControlDistance", rig_size = "rigSize", max_group_fitted = "maxGroupFitted",
    max_type_fitted = "maxTypeFitted", max_group_online = "maxGroupOnline", max_group_active = "maxGroupActive",
    charge_size = "chargeSize",
} arrays {
    dmg: [4] = ["emDamage".into(), "thermalDamage".into(), "kineticDamage".into(), "explosiveDamage".into()],
    extra_durations: [5] = ["durationHighisGood".into(), "durationSensorDampeningBurstProjector".into(),
        "durationTargetIlluminationBurstProjector".into(), "durationECMJammerBurstProjector".into(),
        "durationWeaponDisruptionBurstProjector".into()],
    warfare_id: [4] = std::array::from_fn(|k| format!("warfareBuff{}ID", k + 1)),
    warfare_value: [4] = std::array::from_fn(|k| format!("warfareBuff{}Value", k + 1)),
    res_shield: [4] = ["shieldEmDamageResonance".into(), "shieldThermalDamageResonance".into(), "shieldKineticDamageResonance".into(), "shieldExplosiveDamageResonance".into()],
    res_armor: [4] = ["armorEmDamageResonance".into(), "armorThermalDamageResonance".into(), "armorKineticDamageResonance".into(), "armorExplosiveDamageResonance".into()],
    res_hull: [4] = ["emDamageResonance".into(), "thermalDamageResonance".into(), "kineticDamageResonance".into(), "explosiveDamageResonance".into()],
    sensor: [4] = ["scanRadarStrength".into(), "scanLadarStrength".into(), "scanMagnetometricStrength".into(), "scanGravimetricStrength".into()],
    fam: [6] = ["DamageMultiplier", "DamageEM", "DamageTherm", "DamageKin", "DamageExp", "Duration"].map(|s| format!("fighterAbilityAttackMissile{s}")),
    fmi: [6] = ["DamageMultiplier", "DamageEM", "DamageTherm", "DamageKin", "DamageExp", "Duration"].map(|s| format!("fighterAbilityMissiles{s}")),
    charge_group: [5] = std::array::from_fn(|k| format!("chargeGroup{}", k + 1)),
    can_fit_group: [20] = std::array::from_fn(|k| format!("canFitShipGroup{:02}", k + 1)),
    can_fit_type: [11] = std::array::from_fn(|k| format!("canFitShipType{}", k + 1)),
    req_skill_level: [6] = std::array::from_fn(|k| format!("requiredSkill{}Level", k + 1)),
    req_skill: [6] = std::array::from_fn(|k| format!("requiredSkill{}", k + 1)),
});

id_table!(EffectIds {
    afterburner = "moduleBonusAfterburner", mwd = "moduleBonusMicrowarpdrive", slot_mod = "slotModifier",
    hardpoint_mod = "hardPointModifierEffect", mjd = "microJumpDrive", bastion = "moduleBonusBastionModule",
    rah = "adaptiveArmorHardener", turret = "turretFitted", launcher = "launcherFitted", emp_wave = "empWave",
    vorton = "ChainLightning", shield_boost = "shieldBoosting", fueled_shield_boost = "fueledShieldBoosting",
    armor_repair = "armorRepair", fueled_armor_repair = "fueledArmorRepair", structure_repair = "structureRepair",
    nos = "energyNosferatuFalloff", f_attack = "fighterAbilityAttackM", f_missiles = "fighterAbilityMissiles",
    f_mwd = "fighterAbilityMicroWarpDrive", f_evasive = "fighterAbilityEvasiveManeuvers", f_mjd = "fighterAbilityMicroJumpDrive",
    skill_effect = "skillEffect",
} arrays {
    structure_skill_ok: [5] = ["targetingMaxTargetBonusModAddMaxLockedTargetsLocationChar".into(),
        "skillStructureMissileDamageBonus".into(), "skillStructureElectronicSystemsCapNeedBonus".into(),
        "skillStructureEngineeringSystemsCapNeedBonus".into(), "skillStructureDoomsdayDurationBonus".into()],
});
