using EveDogmaK.Data;
using EveDogmaK.Engine;
using EveDogmaK.Requests;

namespace EveDogmaK.Rules;

/// <summary>On structures pilot skills only reach the structure through a short allow-list (max targets + skillStructure*), plus self-only effects.</summary>
public sealed class StructureSkillGate : IEffectGate
{
    public string Name => "structure-skills";
    public bool Allows(in EffectContext c)
    {
        if (!c.Fit.IsStructure || c.Item.Kind != ItemKind.Skill) return true;
        if (Array.IndexOf(c.K.StructureSkillEffects, c.Effect.Id) >= 0) return true;
        foreach (var m in c.Effect.Modifiers) if (m.Domain != ModDomain.Item) return false;
        return true;
    }
}

/// <summary>Booster side effects (effects with a fittingUsageChance attribute) apply only when selected in the request.</summary>
public sealed class BoosterSideEffectGate : IEffectGate
{
    public string Name => "booster-side-effects";
    public bool Allows(in EffectContext c) =>
        c.Effect.FittingUsageChanceAttr is null || Array.IndexOf(c.Item.BoosterSideEffects, c.Effect.Id.Value) >= 0;
}

/// <summary>Fighter abilities (non-passive effects) apply only when enabled (explicit list or Pyfa defaults).</summary>
public sealed class FighterAbilityGate : IEffectGate
{
    public string Name => "fighter-abilities";
    public bool Allows(in EffectContext c)
    {
        if (c.Item.Kind != ItemKind.Fighter || c.Effect.Category == EffectCategory.Passive) return true;
        return c.Item.FighterAbilities is { } a ? Array.IndexOf(a, c.Effect.Id.Value) >= 0 : c.IsDefault;
    }
}

/// <summary>Effect category vs item state: passive/online need online, active needs active, overload needs overheated.</summary>
public sealed class StateGate : IEffectGate
{
    public string Name => "state";
    public bool Allows(in EffectContext c) => IsActiveIn(c.Effect.Category, c.State);

    public static bool IsActiveIn(EffectCategory cat, ModuleState s) => cat switch
    {
        EffectCategory.Passive or EffectCategory.Online => s >= ModuleState.Online,
        EffectCategory.Active => s >= ModuleState.Active,
        EffectCategory.Overload => s >= ModuleState.Overheated,
        EffectCategory.System => true,
        _ => false, // target, area, dungeon: not local
    };
}
