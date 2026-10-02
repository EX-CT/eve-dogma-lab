using EveDogmaK.Json;
using EveDogmaK.Stats;

namespace EveDogmaK.Requests;

/// <summary>FitRequest -> contract JSON (the inverse of <see cref="RequestParser"/>; used by eft_parse).</summary>
public static class RequestWriter
{
    private static JArr Arr<T>(IEnumerable<T> xs, Func<T, JNode> f) => new(xs.Select(f));
    private static JNode Ints(IEnumerable<int>? xs) => xs == null ? JNode.Null : Arr(xs, x => (JNode)x);

    private static string SpoolName(SpoolType t) => t switch
    {
        SpoolType.SpoolScale => "spool_scale", SpoolType.CycleScale => "cycle_scale", SpoolType.Time => "time", _ => "cycles",
    };
    private static JNode SpoolJson(Spool? s) => s is { } v ? new JObj { { "type", SpoolName(v.Type) }, { "amount", v.Amount } } : JNode.Null;

    private static JNode MutationJson(Mutation? m)
    {
        if (m == null) return JNode.Null;
        var attrs = new JObj();
        foreach (var (k, v) in m.Attributes) attrs[k] = v;
        return new JObj { { "base_type_id", m.BaseTypeId }, { "mutaplasmid_type_id", JNode.Of(m.MutaplasmidTypeId) }, { "attributes", attrs } };
    }

    private static JNode Module(ModuleReq m) => new JObj
    {
        { "type_id", m.TypeId }, { "slot", m.Slot is { } s ? StatsCalculator.SlotName(s) : null },
        { "state", m.State is { } st ? StatsCalculator.StateName(st) : null }, { "charge_type_id", JNode.Of(m.ChargeTypeId) },
        { "mutation", MutationJson(m.Mutation) }, { "spool", SpoolJson(m.Spool) },
    };
    private static JNode Drone(DroneReq d) => new JObj
    {
        { "type_id", d.TypeId }, { "quantity", d.Quantity }, { "active", JNode.Of(d.Active) }, { "mutation", MutationJson(d.Mutation) },
    };
    private static JNode Fighter(FighterReq f) => new JObj
    {
        { "type_id", f.TypeId }, { "quantity", JNode.Of(f.Quantity) }, { "active", f.Active }, { "abilities", Ints(f.Abilities) },
    };
    private static JNode Profile(DamageProfile p) => new JObj { { "em", p.Em }, { "thermal", p.Thermal }, { "kinetic", p.Kinetic }, { "explosive", p.Explosive } };

    public static JObj Write(FitRequest r)
    {
        var levels = new JObj();
        foreach (var (k, v) in r.SkillLevels) levels[k] = v;
        var o = r.Options;
        return new JObj
        {
            { "schema_version", 1 },
            { "ship", new JObj { { "type_id", r.ShipTypeId }, { "mode_type_id", JNode.Of(r.ModeTypeId) } } },
            { "character", new JObj
                {
                    { "security_status", JNode.Of(r.SecurityStatus) },
                    { "skills", new JObj { { "default_level", JNode.Of(r.DefaultSkillLevel) }, { "levels", levels } } },
                } },
            { "modules", Arr(r.Modules, Module) }, { "drones", Arr(r.Drones, Drone) }, { "fighters", Arr(r.Fighters, Fighter) },
            { "implants", Ints(r.Implants) },
            { "boosters", Arr(r.Boosters, b => (JNode)new JObj { { "type_id", b.TypeId }, { "side_effects", Ints(b.SideEffects) } }) },
            { "cargo", Arr(r.Cargo, c => (JNode)new JObj { { "type_id", c.TypeId }, { "quantity", c.Quantity } }) },
            { "fleet", new JObj
                {
                    { "buffs", Arr(r.FleetBuffs, b => (JNode)new JObj { { "buff_id", b.BuffId }, { "value", b.Value } }) },
                    { "booster_fits", Arr(r.BoosterFits, f => (JNode)Write(f)) },
                } },
            { "projected", Arr(r.Projected, p => (JNode)new JObj
                {
                    { "kind", p.Kind }, { "module", p.Module == null ? JNode.Null : Module(p.Module) }, { "drone", p.Drone == null ? JNode.Null : Drone(p.Drone) },
                    { "fit", p.Fit == null ? JNode.Null : Write(p.Fit) }, { "fighter", p.Fighter == null ? JNode.Null : Fighter(p.Fighter) },
                    { "amount", p.Amount }, { "distance_m", JNode.Of(p.DistanceM) },
                }) },
            { "environment", new JObj { { "effect_type_ids", Ints(r.EnvironmentEffects) }, { "system_security", r.SystemSecurity } } },
            { "damage_pattern", r.DamagePattern is { } dp ? Profile(dp) : JNode.Null },
            { "target_profile", r.TargetProfile is { } tp ? new JObj
                {
                    { "em", tp.Resists.Em }, { "thermal", tp.Resists.Thermal }, { "kinetic", tp.Resists.Kinetic }, { "explosive", tp.Resists.Explosive },
                    { "signature_radius", JNode.Of(tp.SignatureRadius) }, { "max_velocity", JNode.Of(tp.MaxVelocity) }, { "radius", JNode.Of(tp.Radius) },
                } : JNode.Null },
            { "overrides", Arr(r.Overrides, x => (JNode)new JObj { { "type_id", x.TypeId }, { "attribute_id", x.AttributeId }, { "value", x.Value } }) },
            { "options", new JObj
                {
                    { "nos_no_target_cap", o.NosNoTargetCap }, { "factor_reload", o.FactorReload }, { "default_spool", SpoolJson(o.DefaultSpool) },
                    { "rah", o.Rah }, { "include_attributes", o.IncludeAttributes }, { "sources", o.Sources }, { "validate", o.Validate },
                    { "cap_sim", new JObj { { "reload", o.CapSim.Reload }, { "stagger", o.CapSim.Stagger }, { "max_time_s", JNode.Of(o.CapSim.MaxTimeS) } } },
                } },
        };
    }
}
