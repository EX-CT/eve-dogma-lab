namespace EveDogmaK.Requests;

/// <summary>Module state; ordering matters (Offline &lt; Online &lt; Active &lt; Overheated).</summary>
public enum ModuleState { Offline, Online, Active, Overheated }

public enum Slot { High, Mid, Low, Rig, Subsystem, Service }

public enum SpoolType { SpoolScale, CycleScale, Time, Cycles }

public readonly record struct Spool(SpoolType Type, double Amount);

public sealed record Mutation(int BaseTypeId, int? MutaplasmidTypeId, IReadOnlyList<KeyValuePair<string, double>> Attributes);

public sealed record ModuleReq(int TypeId, Slot? Slot, ModuleState? State, int? ChargeTypeId, Mutation? Mutation, Spool? Spool);

public sealed record DroneReq(int TypeId, int Quantity, int? Active, Mutation? Mutation);

public sealed record FighterReq(int TypeId, int? Quantity, bool Active, int[]? Abilities);

public sealed record BoosterReq(int TypeId, int[] SideEffects);

public sealed record CargoReq(int TypeId, int Quantity);

public sealed record Buff(int BuffId, double Value);

public sealed record ProjectedReq(string Kind, ModuleReq? Module, DroneReq? Drone, FitRequest? Fit, int Amount, double? DistanceM, FighterReq? Fighter = null);

public readonly record struct DamageProfile(double Em, double Thermal, double Kinetic, double Explosive)
{
    public static readonly DamageProfile Uniform = new(25, 25, 25, 25);
    public double Total => Em + Thermal + Kinetic + Explosive;
    public double this[int i] => i switch { 0 => Em, 1 => Thermal, 2 => Kinetic, _ => Explosive };
}

public sealed record TargetProfile(DamageProfile Resists, double? SignatureRadius, double? MaxVelocity, double? Radius)
{
    public static readonly TargetProfile Default = new(new DamageProfile(0, 0, 0, 0), null, null, null);
}

public sealed record Override(int TypeId, int AttributeId, double Value);

public sealed record CapSimOptions(bool Reload, bool Stagger, double? MaxTimeS);

public sealed record Options(
    bool NosNoTargetCap, bool FactorReload, Spool? DefaultSpool, string? Rah, string? IncludeAttributes,
    bool Sources, bool Validate, CapSimOptions CapSim)
{
    public static readonly Options Default = new(false, false, null, null, null, false, true, new CapSimOptions(false, false, null));
}

/// <summary>FitRequest v1 (contract.md). Immutable; one request = one calculation.</summary>
public sealed record FitRequest(
    int ShipTypeId, int? ModeTypeId,
    int? DefaultSkillLevel, IReadOnlyList<KeyValuePair<string, int>> SkillLevels, double? SecurityStatus,
    IReadOnlyList<ModuleReq> Modules, IReadOnlyList<DroneReq> Drones, IReadOnlyList<FighterReq> Fighters,
    IReadOnlyList<int> Implants, IReadOnlyList<BoosterReq> Boosters, IReadOnlyList<CargoReq> Cargo,
    IReadOnlyList<Buff> FleetBuffs, IReadOnlyList<FitRequest> BoosterFits, IReadOnlyList<ProjectedReq> Projected,
    IReadOnlyList<int> EnvironmentEffects, string? SystemSecurity,
    DamageProfile? DamagePattern, TargetProfile? TargetProfile, IReadOnlyList<Override> Overrides, Options Options);
