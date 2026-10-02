using EveDogmaK.Data;
using EveDogmaK.Engine;
using EveDogmaK.Requests;

namespace EveDogmaK.Rules;

/// <summary>Everything a rule needs to know about one (item, effect) pair being registered.</summary>
public readonly record struct EffectContext(Fit Fit, Item Item, EffectInfo Effect, bool IsDefault, ModuleState State)
{
    public int Source => Item.Index;
    public int SourceCategory => Item.Category;
    public KnownIds K => Fit.K;
    public Dataset Ds => Fit.Ds;

    /// <summary>Modifier on the ship with this effect's item as source.</summary>
    public void OnShip(AttrId target, Op op, ModSource src, int? categoryOverride = null) =>
        Fit.AddModifier(Fit.Ship, target, op, src, Source, categoryOverride ?? SourceCategory);

    public ModSource FromSelf(AttrId attr) => ModSource.FromAttr(Source, attr);
}

/// <summary>
/// A gate decides whether an effect of an item is active at all for this calculation
/// (state, booster side effects, fighter abilities, structure skill filtering, ...).
/// Gates run before rules; the first gate that says no wins.
/// </summary>
public interface IEffectGate
{
    string Name { get; }
    bool Allows(in EffectContext c);
}

/// <summary>
/// A rule turns an active effect into modifiers. Rules are tried in order; the first one whose
/// <see cref="Matches"/> returns true handles the effect. The data-driven SDE rule is always last.
/// </summary>
public interface IEffectRule
{
    string Name { get; }
    bool Matches(EffectInfo effect, KnownIds k);
    void Apply(in EffectContext c);
}

/// <summary>A rule for effects projected onto this fit by another ship (webs, painters, damps, ...).</summary>
public interface IProjectedRule
{
    string Name { get; }
    bool Matches(EffectInfo effect);
    /// <summary>(target attribute on our ship, source attribute on the projector, operator)</summary>
    IEnumerable<(AttrId Target, AttrId Source, Op Op)> Modifiers(EffectInfo effect, KnownIds k);
}

/// <summary>A whole-fit pass that runs after effect registration (fleet buffs, reactive armor adaptation).</summary>
public interface IFitPass
{
    string Name { get; }
    void Run(Fit fit);
}
