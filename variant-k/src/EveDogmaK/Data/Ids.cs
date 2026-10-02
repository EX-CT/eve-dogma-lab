namespace EveDogmaK.Data;

/// <summary>Strongly typed dogma attribute id (dgmAttributeTypes.attributeID).</summary>
public readonly record struct AttrId(int Value)
{
    public static readonly AttrId None = new(0);
    public bool IsNone => Value == 0;
    public override string ToString() => $"attr#{Value}";
}

/// <summary>Strongly typed dogma effect id (dgmEffects.effectID).</summary>
public readonly record struct EffectId(int Value)
{
    public static readonly EffectId None = new(0);
    public bool IsNone => Value == 0;
    public override string ToString() => $"effect#{Value}";
}

/// <summary>Dogma modifier operator (SDE modifierInfo.operation). Evaluation order is the numeric order.</summary>
public enum Op : sbyte
{
    PreAssign = -1,
    PreMul = 0,
    PreDiv = 1,
    ModAdd = 2,
    ModSub = 3,
    PostMul = 4,
    PostDiv = 5,
    PostPercent = 6,
    PostAssign = 7,
    /// <summary>skill-level style ops the engine does not apply (8, 9, ...).</summary>
    Unsupported = 9,
}

/// <summary>modifierInfo.func: how a modifier selects its targets.</summary>
public enum ModFunc : byte { Item, Location, LocationGroup, LocationRequiredSkill, OwnerRequiredSkill, EffectStopper }

/// <summary>modifierInfo.domain: whose items a modifier reaches.</summary>
public enum ModDomain : byte { Item, Ship, Char, Other, Structure, TargetId, Target, None }

/// <summary>dgmEffects.effectCategory.</summary>
public enum EffectCategory : byte
{
    Passive = 0, Active = 1, Target = 2, Area = 3, Online = 4, Overload = 5, Dungeon = 6, System = 7,
}

/// <summary>One data-driven modifier: "on targets selected by (Func, Domain, Filter) apply Op(Modifying) to Modified".</summary>
public readonly record struct Modifier(ModFunc Func, ModDomain Domain, AttrId Modified, AttrId Modifying, Op Op, int Filter);

public readonly record struct EffectRef(EffectId Id, bool IsDefault);
