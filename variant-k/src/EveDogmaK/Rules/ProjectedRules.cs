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
        Array.Empty<string>(), SensorMods);
    private static (AttrId, AttrId, Op)[] SensorMods(KnownIds k) => new[]
    {
        (k.MaxTargetRange, k.MaxTargetRangeBonus, Op.PostPercent), (k.ScanResolution, k.ScanResolutionBonus, Op.PostPercent),
    };
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
            if (rule == null) { fit.Warnings.Add($"projected effect '{e.Name}' not modelled yet"); continue; }
            foreach (var (target, source, op) in rule.Modifiers(e, fit.K))
            {
                bool mul = op is Op.PostMul or Op.PreMul;
                fit.AddModifier(ship, target, op, ModSource.Projected(i, source, factor, ship, resist, mul), i, it.Category);
            }
        }
    }
}
