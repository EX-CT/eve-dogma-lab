using EveDogmaK.Data;
using EveDogmaK.Requests;

namespace EveDogmaK.Engine;

public enum ItemKind { Ship, Character, Skill, Module, Charge, Drone, Fighter, Implant, Booster, Mode, Beacon, Projected }

/// <summary>Where an item lives: decides which location modifiers reach it.</summary>
public enum ItemLocation { Ship, Character, Space, Nowhere }

public enum SourceKind : byte { Attribute, Constant, Propulsion, Projected }

/// <summary>
/// Where a modifier's value comes from. Tagged union (struct, no allocation):
/// <list type="bullet">
/// <item>Attribute: value of <c>Attr</c> on item <c>Item</c> (the normal dogma case)</item>
/// <item>Constant: fixed value (fleet buffs, adapted RAH resonances)</item>
/// <item>Propulsion: AB/MWD velocity multiplier 1 + speedFactor/100 * speedBoostFactor / ship mass</item>
/// <item>Projected: remote effect value scaled by range factor and the target's resistance attribute</item>
/// </list>
/// </summary>
public readonly struct ModSource
{
    public readonly SourceKind Kind;
    public readonly int Item;       // source item (Attribute, Propulsion module, Projected source)
    public readonly AttrId Attr;    // Attribute / Projected value attr, Propulsion speedFactor
    public readonly AttrId Attr2;   // Propulsion speedBoostFactor, Projected resistance attr
    public readonly int Target;     // Propulsion ship, Projected target
    public readonly double Number;  // Constant value, Projected range factor
    public readonly bool Multiplicative; // Projected: (v-1)*f+1 instead of v*f

    private ModSource(SourceKind k, int item, AttrId a, AttrId a2, int target, double n, bool mul)
    { Kind = k; Item = item; Attr = a; Attr2 = a2; Target = target; Number = n; Multiplicative = mul; }

    public static ModSource FromAttr(int item, AttrId attr) => new(SourceKind.Attribute, item, attr, AttrId.None, 0, 0, false);
    public static ModSource Const(double v) => new(SourceKind.Constant, 0, AttrId.None, AttrId.None, 0, v, false);
    public static ModSource Propulsion(int module, int ship, AttrId speedFactor, AttrId thrust) =>
        new(SourceKind.Propulsion, module, speedFactor, thrust, ship, 0, false);
    public static ModSource Projected(int item, AttrId attr, double rangeFactor, int target, AttrId resist, bool mul) =>
        new(SourceKind.Projected, item, attr, resist, target, rangeFactor, mul);
}

/// <summary>A modifier bound to a concrete target attribute.</summary>
public readonly record struct AppliedModifier(Op Op, bool Penalized, ModSource Source, int SourceItem);

/// <summary>One attribute of one item: base value, incoming modifiers, memoised result.</summary>
public sealed class AttrNode
{
    public double Base;
    public List<AppliedModifier>? Mods;
    internal double Value;
    internal int Generation = -1;
    internal bool Busy;
    public AttrNode(double b) { Base = b; }
}

public sealed class Item
{
    public required int Index { get; init; }
    public required TypeInfo Type { get; init; }
    public int TypeId => Type.Id;
    public int Group => Type.Group;
    public int Category => Type.Category;
    public required ItemKind Kind { get; init; }
    public required ItemLocation Location { get; init; }
    public ModuleState State { get; set; } = ModuleState.Online;
    /// <summary>"Owned by the pilot" (OwnerRequiredSkill reach): modules, charges, drones, fighters, ship.</summary>
    public bool Owned { get; set; }
    public int Parent { get; set; } = -1;
    public int Charge { get; set; } = -1;
    public Slot? Slot { get; set; }
    public int? RequestIndex { get; set; }
    public int Quantity { get; set; } = 1;
    public int ActiveCount { get; set; }
    public int[] RequiredSkills { get; set; } = Array.Empty<int>();
    public List<EffectRef> Effects { get; set; } = new();
    public int[]? FighterAbilities { get; set; }
    public int[] BoosterSideEffects { get; set; } = Array.Empty<int>();
    public Spool? Spool { get; set; }
    public double? DistanceM { get; set; }

    /// <summary>Base attribute table (the type's, or a merged table for mutated items).</summary>
    public AttrTable BaseAttrs { get; set; } = AttrTable.Empty;
    /// <summary>Materialised attributes (overridden, modified, or already evaluated).</summary>
    public readonly Dictionary<int, AttrNode> Nodes = new();

    public bool HasEffect(EffectId e) { foreach (var x in Effects) if (x.Id == e) return true; return false; }
    public bool RequiresSkill(int skill) { foreach (var s in RequiredSkills) if (s == skill) return true; return false; }
}

public sealed class EngineException : Exception
{
    public string Code { get; }
    public string Path { get; }
    public EngineException(string code, string message, string path) : base(message) { Code = code; Path = path; }
}
