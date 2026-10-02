namespace EveDogmaK.Data;

/// <summary>
/// Every attribute / effect the engine refers to by name, resolved once per dataset.
/// Rules and stats use these typed handles instead of string lookups in hot paths.
/// </summary>
public sealed class KnownIds
{
    // fixed SDE ids
    public static readonly AttrId Mass = new(4), Capacity = new(38), Volume = new(161), Radius = new(162), Hp = new(9);
    public static readonly AttrId SkillLevel = new(280);
    public static readonly EffectId SkillEffect = new(132);
    /// <summary>em/explosive/kinetic/thermal DamageResonance (hull)</summary>
    public static readonly AttrId[] HullResonances = { new(113), new(111), new(109), new(110) };

    public readonly AttrId Cpu, Power, CpuOutput, PowerOutput, UpgradeCost, UpgradeCapacity, Speed, Duration, CapacitorNeed,
        ReloadTime, ReactivationDelay, ChargeRate, DamageMultiplier, MissileDamageMultiplier,
        MassAddition, SpeedFactor, SpeedBoostFactor, MaxVelocity, SignatureRadius, SignatureRadiusBonus, SignatureRadiusBonusPercent,
        HiSlots, MedSlots, LowSlots, RigSlots, MaxSubSystems, ServiceSlots, HiSlotModifier, MedSlotModifier, LowSlotModifier,
        TurretSlotsLeft, LauncherSlotsLeft, TurretHardPointModifier, LauncherHardPointModifier,
        MaxTargetRange, MaxTargetRangeBonus, ScanResolution, ScanResolutionBonus, RemoteResistanceId,
        PilotSecurityStatus, SecurityModifier, HiSecModifier, LowSecModifier, NullSecModifier,
        FighterSquadronMaxSize, ResistanceShiftAmount, DamageMultiplierBonusMax, DamageMultiplierBonusPerCycle,
        MaxRange, Falloff, TrackingSpeed, ExplosionDelay, AoeCloudSize, AoeVelocity, EmpFieldRange,
        CrystalsGetDamaged, CrystalVolatilityChance, CrystalVolatilityDamage,
        DroneBandwidthUsed, DroneBandwidth, DroneCapacity, FighterCapacity, FighterTubes, FighterLightSlots, FighterSupportSlots,
        FighterHeavySlots, FighterSquadronIsHeavy, FighterSquadronIsSupport,
        ShieldCapacity, ArmorHp, ShieldRechargeRate, ShieldBonus, ArmorDamageAmount, StructureDamageAmount,
        CapacitorCapacity, RechargeRate, CapacitorBonus, PowerTransferAmount,
        SpeedLimit, Agility, BaseWarpSpeed, WarpSpeedMultiplier, WarpCapacitorNeed, WarpScrambleStatus,
        MaxLockedTargets, ScanRadarStrength, ScanLadarStrength, ScanMagnetometricStrength, ScanGravimetricStrength,
        MaxActiveDrones, DroneControlDistance, RigSize, ChargeSize, MaxGroupFitted, MaxTypeFitted, MaxGroupOnline, MaxGroupActive;

    public readonly AttrId[] Damage;            // em, thermal, kinetic, explosive
    public readonly AttrId[] ShieldResonance, ArmorResonance, HullResonance; // em, thermal, kinetic, explosive
    public readonly AttrId[] ExtraCycleAttrs;   // durationHighisGood + burst projector durations
    public readonly (AttrId Id, AttrId Value)[] WarfareBuffs; // warfareBuff1..4 ID/Value
    public readonly AttrId[] CanFitShipGroups, CanFitShipTypes, ChargeGroups;
    public readonly AttrId[] RequiredSkill, RequiredSkillLevel;
    public readonly (AttrId, AttrId, Op)[] SensorStrengthBonuses; // scan{T}Strength += scan{T}StrengthPercent
    public readonly AttrId ShipRadius, MaxFofTargetRange, EnergyNeutralizerSignatureResolution;
    public readonly EffectId FofMissileLaunching;

    public readonly EffectId Afterburner, Microwarpdrive, SlotModifier, HardPointModifier, MicroJumpDrive, Bastion,
        AdaptiveArmorHardener, TurretFitted, LauncherFitted, EmpWave, ChainLightning,
        ShieldBoosting, FueledShieldBoosting, ArmorRepair, FueledArmorRepair, StructureRepair, EnergyNosferatuFalloff,
        FighterAttackM, FighterMissiles;
    public readonly EffectId[] StructureSkillEffects;

    public KnownIds(Dataset ds)
    {
        AttrId A(string n) => ds.AttrIdOf(n);
        EffectId E(string n) => ds.EffectIdOf(n);
        Cpu = A("cpu"); Power = A("power"); CpuOutput = A("cpuOutput"); PowerOutput = A("powerOutput");
        UpgradeCost = A("upgradeCost"); UpgradeCapacity = A("upgradeCapacity"); Speed = A("speed"); Duration = A("duration");
        CapacitorNeed = A("capacitorNeed"); ReloadTime = A("reloadTime"); ReactivationDelay = A("moduleReactivationDelay");
        ChargeRate = A("chargeRate"); DamageMultiplier = A("damageMultiplier"); MissileDamageMultiplier = A("missileDamageMultiplier");
        MassAddition = A("massAddition"); SpeedFactor = A("speedFactor"); SpeedBoostFactor = A("speedBoostFactor");
        MaxVelocity = A("maxVelocity"); SignatureRadius = A("signatureRadius"); SignatureRadiusBonus = A("signatureRadiusBonus");
        SignatureRadiusBonusPercent = A("signatureRadiusBonusPercent");
        HiSlots = A("hiSlots"); MedSlots = A("medSlots"); LowSlots = A("lowSlots"); RigSlots = A("rigSlots");
        MaxSubSystems = A("maxSubSystems"); ServiceSlots = A("serviceSlots");
        HiSlotModifier = A("hiSlotModifier"); MedSlotModifier = A("medSlotModifier"); LowSlotModifier = A("lowSlotModifier");
        TurretSlotsLeft = A("turretSlotsLeft"); LauncherSlotsLeft = A("launcherSlotsLeft");
        TurretHardPointModifier = A("turretHardPointModifier"); LauncherHardPointModifier = A("launcherHardPointModifier");
        MaxTargetRange = A("maxTargetRange"); MaxTargetRangeBonus = A("maxTargetRangeBonus");
        ScanResolution = A("scanResolution"); ScanResolutionBonus = A("scanResolutionBonus"); RemoteResistanceId = A("remoteResistanceID");
        PilotSecurityStatus = A("pilotSecurityStatus"); SecurityModifier = A("securityModifier");
        HiSecModifier = A("hiSecModifier"); LowSecModifier = A("lowSecModifier"); NullSecModifier = A("nullSecModifier");
        FighterSquadronMaxSize = A("fighterSquadronMaxSize"); ResistanceShiftAmount = A("resistanceShiftAmount");
        DamageMultiplierBonusMax = A("damageMultiplierBonusMax"); DamageMultiplierBonusPerCycle = A("damageMultiplierBonusPerCycle");
        MaxRange = A("maxRange"); Falloff = A("falloff"); TrackingSpeed = A("trackingSpeed"); ExplosionDelay = A("explosionDelay");
        AoeCloudSize = A("aoeCloudSize"); AoeVelocity = A("aoeVelocity"); EmpFieldRange = A("empFieldRange");
        CrystalsGetDamaged = A("crystalsGetDamaged"); CrystalVolatilityChance = A("crystalVolatilityChance");
        CrystalVolatilityDamage = A("crystalVolatilityDamage");
        DroneBandwidthUsed = A("droneBandwidthUsed"); DroneBandwidth = A("droneBandwidth"); DroneCapacity = A("droneCapacity");
        FighterCapacity = A("fighterCapacity"); FighterTubes = A("fighterTubes"); FighterLightSlots = A("fighterLightSlots");
        FighterSupportSlots = A("fighterSupportSlots"); FighterHeavySlots = A("fighterHeavySlots");
        FighterSquadronIsHeavy = A("fighterSquadronIsHeavy"); FighterSquadronIsSupport = A("fighterSquadronIsSupport");
        ShieldCapacity = A("shieldCapacity"); ArmorHp = A("armorHP"); ShieldRechargeRate = A("shieldRechargeRate");
        ShieldBonus = A("shieldBonus"); ArmorDamageAmount = A("armorDamageAmount"); StructureDamageAmount = A("structureDamageAmount");
        CapacitorCapacity = A("capacitorCapacity"); RechargeRate = A("rechargeRate"); CapacitorBonus = A("capacitorBonus");
        PowerTransferAmount = A("powerTransferAmount"); SpeedLimit = A("speedLimit"); Agility = A("agility");
        BaseWarpSpeed = A("baseWarpSpeed"); WarpSpeedMultiplier = A("warpSpeedMultiplier"); WarpCapacitorNeed = A("warpCapacitorNeed");
        WarpScrambleStatus = A("warpScrambleStatus"); MaxLockedTargets = A("maxLockedTargets");
        ScanRadarStrength = A("scanRadarStrength"); ScanLadarStrength = A("scanLadarStrength");
        ScanMagnetometricStrength = A("scanMagnetometricStrength"); ScanGravimetricStrength = A("scanGravimetricStrength");
        MaxActiveDrones = A("maxActiveDrones"); DroneControlDistance = A("droneControlDistance"); RigSize = A("rigSize");
        ChargeSize = A("chargeSize"); MaxGroupFitted = A("maxGroupFitted"); MaxTypeFitted = A("maxTypeFitted");
        MaxGroupOnline = A("maxGroupOnline"); MaxGroupActive = A("maxGroupActive");

        Damage = new[] { A("emDamage"), A("thermalDamage"), A("kineticDamage"), A("explosiveDamage") };
        ShieldResonance = Layer("shield"); ArmorResonance = Layer("armor"); HullResonance = Layer("");
        AttrId[] Layer(string p) => p == ""
            ? new[] { A("emDamageResonance"), A("thermalDamageResonance"), A("kineticDamageResonance"), A("explosiveDamageResonance") }
            : new[] { A(p + "EmDamageResonance"), A(p + "ThermalDamageResonance"), A(p + "KineticDamageResonance"), A(p + "ExplosiveDamageResonance") };
        ExtraCycleAttrs = new[] { "durationHighisGood", "durationSensorDampeningBurstProjector", "durationTargetIlluminationBurstProjector",
            "durationECMJammerBurstProjector", "durationWeaponDisruptionBurstProjector" }.Select(A).Where(a => !a.IsNone).ToArray();
        WarfareBuffs = Enumerable.Range(1, 4).Select(k => (A($"warfareBuff{k}ID"), A($"warfareBuff{k}Value"))).ToArray();
        CanFitShipGroups = Enumerable.Range(1, 20).Select(k => A($"canFitShipGroup{k:00}")).Where(a => !a.IsNone).ToArray();
        CanFitShipTypes = Enumerable.Range(1, 11).Select(k => A($"canFitShipType{k}")).Where(a => !a.IsNone).ToArray();
        ChargeGroups = Enumerable.Range(1, 5).Select(k => A($"chargeGroup{k}")).ToArray();
        RequiredSkill = Enumerable.Range(1, 6).Select(k => A($"requiredSkill{k}")).ToArray();
        RequiredSkillLevel = Enumerable.Range(1, 6).Select(k => A($"requiredSkill{k}Level")).ToArray();

        SensorStrengthBonuses = new[] { "Gravimetric", "Ladar", "Magnetometric", "Radar" }
            .Select(t => (A($"scan{t}Strength"), A($"scan{t}StrengthPercent"), Op.PostPercent)).ToArray();
        ShipRadius = A("radius"); MaxFofTargetRange = A("maxFOFTargetRange");
        EnergyNeutralizerSignatureResolution = A("energyNeutralizerSignatureResolution");
        FofMissileLaunching = E("fofMissileLaunching");
        Afterburner = E("moduleBonusAfterburner"); Microwarpdrive = E("moduleBonusMicrowarpdrive"); SlotModifier = E("slotModifier");
        HardPointModifier = E("hardPointModifierEffect"); MicroJumpDrive = E("microJumpDrive"); Bastion = E("moduleBonusBastionModule");
        AdaptiveArmorHardener = E("adaptiveArmorHardener"); TurretFitted = E("turretFitted"); LauncherFitted = E("launcherFitted");
        EmpWave = E("empWave"); ChainLightning = E("ChainLightning"); ShieldBoosting = E("shieldBoosting");
        FueledShieldBoosting = E("fueledShieldBoosting"); ArmorRepair = E("armorRepair"); FueledArmorRepair = E("fueledArmorRepair");
        StructureRepair = E("structureRepair"); EnergyNosferatuFalloff = E("energyNosferatuFalloff");
        FighterAttackM = E("fighterAbilityAttackM"); FighterMissiles = E("fighterAbilityMissiles");
        StructureSkillEffects = new[] { "targetingMaxTargetBonusModAddMaxLockedTargetsLocationChar", "skillStructureMissileDamageBonus",
            "skillStructureElectronicSystemsCapNeedBonus", "skillStructureEngineeringSystemsCapNeedBonus",
            "skillStructureDoomsdayDurationBonus" }.Select(E).ToArray();
    }
}
