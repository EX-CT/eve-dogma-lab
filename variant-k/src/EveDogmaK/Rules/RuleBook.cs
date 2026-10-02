using EveDogmaK.Data;
using EveDogmaK.Engine;
using EveDogmaK.Requests;

namespace EveDogmaK.Rules;

/// <summary>
/// The complete, ordered rule set of the engine. Reading this class top to bottom tells you everything the engine
/// does beyond plain SDE modifiers: which effects are gated, which specials exist, and which whole-fit passes run.
/// </summary>
public sealed class RuleBook
{
    public IReadOnlyList<IEffectGate> Gates { get; }
    public IReadOnlyList<IEffectRule> Rules { get; }
    public IReadOnlyList<IProjectedRule> ProjectedRules { get; }
    public IReadOnlyList<IFitPass> Passes { get; }

    public RuleBook(IEffectGate[] gates, IEffectRule[] rules, IProjectedRule[] projected, IFitPass[] passes)
    {
        if (rules.Length == 0 || rules[^1] is not DataDrivenModifierRule)
            throw new ArgumentException("the data-driven SDE rule must be the last (catch-all) rule");
        Gates = gates; Rules = rules; ProjectedRules = projected; Passes = passes;
    }

    [ThreadStatic] private static RuleBook? _default;
    /// <summary>Per-thread instance (rules keep small scratch buffers).</summary>
    public static RuleBook Default => _default ??= Create();

    public static RuleBook Create() => new(
        gates: new IEffectGate[] { new StructureSkillGate(), new BoosterSideEffectGate(), new FighterAbilityGate(), new StateGate() },
        rules: new IEffectRule[] { new PropulsionRule(), new MicroJumpDriveRule(), new SlotModifierRule(), new HardpointModifierRule(), new DataDrivenModifierRule() },
        projected: new IProjectedRule[]
        {
            new DataDrivenProjectedRule(), NamedProjectedRule.Webifier, NamedProjectedRule.TargetPainter,
            NamedProjectedRule.SensorDampener, NamedProjectedRule.SensorBooster,
        },
        passes: new IFitPass[] { new WarfareBuffPass(), new ReactiveArmorHardenerPass() });

    // per-dataset cache: effect id -> rule index (rules match on the effect only)
    private Dataset? _ruleCacheDs;
    private Dictionary<int, IEffectRule>? _ruleCache;

    private IEffectRule RuleFor(EffectInfo e, KnownIds k)
    {
        if (_ruleCache!.TryGetValue(e.Id.Value, out var r)) return r;
        r = Rules.First(x => x.Matches(e, k));
        _ruleCache[e.Id.Value] = r;
        return r;
    }

    /// <summary>Register all modifiers of the fit, then run the whole-fit passes.</summary>
    public void Register(Fit fit)
    {
        if (!ReferenceEquals(_ruleCacheDs, fit.Ds)) { _ruleCacheDs = fit.Ds; _ruleCache = new(); }
        var ds = fit.Ds;
        var k = fit.K;
        int n = fit.Items.Count;
        for (int i = 0; i < n; i++)
        {
            var it = fit[i];
            if (it.Kind == ItemKind.Projected) { ProjectedRegistration.Register(fit, i, ProjectedRules); continue; }
            // structures ignore pilot implants/boosters and cannot use drones
            if (fit.IsStructure && it.Kind is ItemKind.Drone or ItemKind.Implant or ItemKind.Booster) continue;
            var state = fit.EffectiveState(i);
            for (int x = 0; x < it.Effects.Count; x++)
            {
                var eref = it.Effects[x];
                if (eref.Id == KnownIds.SkillEffect) continue; // the generic "skill" marker effect
                var e = ds.Effect(eref.Id);
                if (e == null) continue;
                var ctx = new EffectContext(fit, it, e, eref.IsDefault, state);
                bool allowed = true;
                foreach (var g in Gates) if (!g.Allows(ctx)) { allowed = false; break; }
                if (!allowed) continue;
                RuleFor(e, k).Apply(ctx);
            }
        }
        foreach (var p in Passes) p.Run(fit);
    }
}
