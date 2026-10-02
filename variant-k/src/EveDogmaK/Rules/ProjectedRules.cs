using EveDogmaK.Data;
using EveDogmaK.Engine;
using EveDogmaK.Requests;
using EveDogmaK.Stats;

namespace EveDogmaK.Rules;

/// <summary>Projected effect with SDE modifiers: its Item-func modifiers on target/ship domains apply to our ship.</summary>
public sealed class DataDrivenProjectedRule : IProjectedRule
{
    public string Name => "sde-projected";
    public bool Matches(EffectInfo e) => e.Modifiers.Length > 0;
    public IEnumerable<(AttrId, AttrId, Op)> Modifiers(EffectInfo e, KnownIds k)
    {
        foreach (var m in e.Modifiers)
            if (m.Domain is ModDomain.TargetId or ModDomain.Target or ModDomain.Ship && m.Func == ModFunc.Item)
                yield return (m.Modified, m.Modifying, m.Op);
    }
}

/// <summary>Named projected effects without modifierInfo: (name prefix or exact name) -> fixed modifier list.</summary>
public sealed class NamedProjectedRule : IProjectedRule
{
    private readonly string[] _prefixes, _exact;
    private readonly Func<KnownIds, (AttrId, AttrId, Op)[]> _mods;
    public string Name { get; }
    public NamedProjectedRule(string name, string[] prefixes, string[] exact, Func<KnownIds, (AttrId, AttrId, Op)[]> mods)
    { Name = name; _prefixes = prefixes; _exact = exact; _mods = mods; }
    public bool Matches(EffectInfo e) =>
        _prefixes.Any(p => e.Name.StartsWith(p, StringComparison.Ordinal)) || _exact.Contains(e.Name);
    public IEnumerable<(AttrId, AttrId, Op)> Modifiers(EffectInfo e, KnownIds k) => _mods(k);

    public static readonly NamedProjectedRule Webifier = new("stasis-web", new[] { "remoteWebifier" },
        new[] { "structureModuleEffectStasisWebifier" }, k => new[] { (k.MaxVelocity, k.SpeedFactor, Op.PostPercent) });
    public static readonly NamedProjectedRule TargetPainter = new("target-painter", new[] { "remoteTargetPaint" },
        new[] { "structureModuleEffectTargetPainter" }, k => new[] { (k.SignatureRadius, k.SignatureRadiusBonus, Op.PostPercent) });
    public static readonly NamedProjectedRule SensorDampener = new("sensor-dampener", new[] { "remoteSensorDamp" },
        new[] { "structureModuleEffectRemoteSensorDampener" }, SensorMods);
    public static readonly NamedProjectedRule SensorBooster = new("remote-sensor-booster", new[] { "remoteSensorBoost" },
        Array.Empty<string>(), SensorBoostMods);
    private static (AttrId, AttrId, Op)[] SensorMods(KnownIds k) => new[]
    {
        (k.MaxTargetRange, k.MaxTargetRangeBonus, Op.PostPercent), (k.ScanResolution, k.ScanResolutionBonus, Op.PostPercent),
    };
    /// <summary>Remote sensor boosters also raise sensor strength (scan*StrengthPercent).</summary>
    private static (AttrId, AttrId, Op)[] SensorBoostMods(KnownIds k) => SensorMods(k).Concat(k.SensorStrengthBonuses).ToArray();
}

/// <summary>Registers effects projected onto this fit: range factor from the projector's optimal/falloff, resisted by the target attribute.</summary>
public static class ProjectedRegistration
{
    public static void Register(Fit fit, int i, IReadOnlyList<IProjectedRule> rules)
    {
        var it = fit[i];
        int ship = fit.Ship;
        foreach (var eref in it.Effects)
        {
            var e = fit.Ds.Effect(eref.Id);
            if (e == null || e.Category is not (EffectCategory.Target or EffectCategory.Area)) continue;
            if (it.State < ModuleState.Active) continue;
            double opt = e.RangeAttr is { } ra && fit.Has(i, ra) ? fit.Base(i, ra) : 0.0;
            double fo = e.FalloffAttr is { } fa && fit.Has(i, fa) ? fit.Base(i, fa) : 0.0;
            double factor = Formulas.RangeFactor(opt, fo, it.DistanceM, restricted: true);
            AttrId resist = e.ResistanceAttr ?? (fit.Has(i, fit.K.RemoteResistanceId) ? new AttrId((int)fit.Base(i, fit.K.RemoteResistanceId)) : AttrId.None);
            var rule = rules.FirstOrDefault(r => r.Matches(e));
            if (rule == null)
            {
                if (IncomingEffects.TryCreate(fit, i, e.Name, resist, out var incoming)) fit.Incoming.AddRange(incoming);
                else if (!IncomingEffects.WeaponDamage.Contains(e.Name)) fit.Warnings.Add($"projected effect '{e.Name}' not modelled yet");
                continue;
            }
            foreach (var (target, source, op) in rule.Modifiers(e, fit.K))
            {
                bool mul = op is Op.PostMul or Op.PreMul;
                fit.AddModifier(ship, target, op, ModSource.Projected(i, source, factor, ship, resist, mul), i, it.Category);
            }
        }
    }
}

/// <summary>
/// Pyfa's 'projected' handlers for remote reps, cap transfers and neuts/nos (eos/effects.py, LGPL; re-implemented
/// from the reference engine). Optimal/falloff come from the projector's (frozen) base values.
/// </summary>
public static class IncomingEffects
{
    /// <summary>Weapon damage onto the target: not part of the target's own stats (silently ignored).</summary>
    public static readonly HashSet<string> WeaponDamage = new()
    {
        "projectileFired", "targetAttack", "useMissiles", "barrage", "targetDisintegratorAttack", "missileLaunchingForEntity",
        "fighterAbilityAttackM", "fighterAbilityMissiles", "superWeaponAmarr", "superWeaponCaldari", "superWeaponGallente",
        "superWeaponMinmatar", "mining", "miningLaser", "miningClouds", "dotMissileLaunching",
    };

    public static bool TryCreate(Fit fit, int i, string name, AttrId resist, out List<IncomingEffect> result)
    {
        var ds = fit.Ds;
        var it = fit[i];
        AttrId A(string n) => ds.AttrIdOf(n);
        double Base(string n) { var a = A(n); return fit.Has(i, a) ? fit.Base(i, a) : 0.0; }
        double FalloffFactor() => Formulas.RangeFactor(Base("maxRange"), Base("falloffEffectiveness"), it.DistanceM, restricted: true);
        double Gate(double optimal) => optimal < (it.DistanceM ?? 0.0) ? 0.0 : 1.0;
        var noAssistAttr = A("disallowAssistance");
        bool noAssist = fit.Has(fit.Ship, noAssistAttr) && fit.Base(fit.Ship, noAssistAttr) != 0.0;
        List<IncomingEffect> Rep(int layer, string amount, double mult, double factor) =>
            noAssist ? new() : new() { new IncomingRepair(i, layer, A(amount), mult, factor) };
        List<IncomingEffect> Drain(string amount, string duration, double factor, double sign) =>
            new() { new IncomingCapacitor(i, A(amount), A(duration), factor, resist, sign) };
        bool paste = it.Charge >= 0 && fit[it.Charge].Type.Name == "Nanite Repair Paste";
        List<IncomingEffect>? r = name switch
        {
            "shipModuleRemoteShieldBooster" or "shipModuleAncillaryRemoteShieldBooster" => Rep(0, "shieldBonus", 1.0, FalloffFactor()),
            "shipModuleRemoteArmorRepairer" or "ShipModuleRemoteArmorMutadaptiveRepairer" => Rep(1, "armorDamageAmount", 1.0, FalloffFactor()),
            "shipModuleAncillaryRemoteArmorRepairer" => Rep(1, "armorDamageAmount", paste ? 3.0 : 1.0, FalloffFactor()),
            "shipModuleRemoteHullRepairer" => Rep(2, "structureDamageAmount", 1.0, FalloffFactor()),
            "npcEntityRemoteShieldBooster" => Rep(0, "shieldBonus", 1.0, Gate(Base("maxRange"))),
            "npcEntityRemoteArmorRepairer" => Rep(1, "armorDamageAmount", 1.0, Gate(Base("maxRange"))),
            "npcEntityRemoteHullRepairer" => Rep(2, "structureDamageAmount", 1.0, Gate(Base("maxRange"))),
            "shipModuleRemoteCapacitorTransmitter" => noAssist ? new() : Drain("powerTransferAmount", "duration", Gate(Base("maxRange")), -1.0),
            "energyNeutralizerFalloff" => Drain("energyNeutralizerAmount", "duration", FalloffFactor(), 1.0),
            "energyNosferatuFalloff" => Drain("powerTransferAmount", "duration", FalloffFactor(), 1.0),
            "structureEnergyNeutralizerFalloff" => Drain("energyNeutralizerAmount", "duration", 1.0, 1.0),
            "entityEnergyNeutralizerFalloff" => Drain("energyNeutralizerAmount", "energyNeutralizerDuration", Gate(Base("energyNeutralizerRangeOptimal")), 1.0),
            _ => null,
        };
        result = r ?? new();
        return r != null;
    }
}
