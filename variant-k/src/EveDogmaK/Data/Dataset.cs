namespace EveDogmaK.Data;

public sealed class AttrInfo
{
    public required AttrId Id { get; init; }
    public required string Name { get; init; }
    public double Default { get; init; }
    public bool Stackable { get; init; } = true;
    public bool HighIsGood { get; init; } = true;
    public AttrId? MinAttr { get; init; }
    public AttrId? MaxAttr { get; init; }
    /// <summary>cpu/power/cpuOutput/powerOutput are rounded to 0.01 like the client.</summary>
    public bool RoundToCentis { get; init; }
}

public sealed class EffectInfo
{
    public required EffectId Id { get; init; }
    public required string Name { get; init; }
    public EffectCategory Category { get; init; }
    public AttrId? DurationAttr { get; init; }
    public AttrId? RangeAttr { get; init; }
    public AttrId? FalloffAttr { get; init; }
    public AttrId? ResistanceAttr { get; init; }
    public AttrId? FittingUsageChanceAttr { get; init; }
    public bool IsOffensive { get; init; }
    public bool IsAssistance { get; init; }
    public Modifier[] Modifiers { get; init; } = Array.Empty<Modifier>();
}

/// <summary>Sorted (attribute id, value) table with binary-search lookup.</summary>
public sealed class AttrTable
{
    public static readonly AttrTable Empty = new(Array.Empty<int>(), Array.Empty<double>());
    private readonly int[] _ids;
    private readonly double[] _vals;
    public AttrTable(int[] sortedIds, double[] vals) { _ids = sortedIds; _vals = vals; }
    public int Count => _ids.Length;
    public bool TryGet(AttrId a, out double v)
    {
        int i = Array.BinarySearch(_ids, a.Value);
        if (i >= 0) { v = _vals[i]; return true; }
        v = 0; return false;
    }
    public double? Get(AttrId a) => TryGet(a, out var v) ? v : null;
    public IEnumerable<(AttrId Attr, double Value)> Entries()
    {
        for (int i = 0; i < _ids.Length; i++) yield return (new AttrId(_ids[i]), _vals[i]);
    }
    public static AttrTable From(IEnumerable<KeyValuePair<int, double>> kv)
    {
        var arr = kv.OrderBy(x => x.Key).ToArray();
        return new AttrTable(arr.Select(x => x.Key).ToArray(), arr.Select(x => x.Value).ToArray());
    }
}

public sealed class TypeInfo
{
    public required int Id { get; init; }
    public required string Name { get; init; }
    public int Group { get; init; }
    public int Category { get; init; }
    public bool Published { get; init; }
    public double Mass { get; init; }
    public double Volume { get; init; }
    public double Capacity { get; init; }
    public double Radius { get; init; }
    public int? MetaLevel { get; init; }
    /// <summary>Dogma attributes exactly as in the SDE.</summary>
    public required AttrTable RawAttrs { get; init; }
    /// <summary>Item base attributes: raw attrs with the type-level mass/capacity/volume/radius folded in.</summary>
    public required AttrTable Attrs { get; init; }
    public required EffectRef[] Effects { get; init; }
    /// <summary>requiredSkill1..6 type ids (non-zero).</summary>
    public required int[] RequiredSkills { get; init; }
    public bool HasEffect(EffectId e) { foreach (var x in Effects) if (x.Id == e) return true; return false; }
}

public sealed record GroupInfo(string Name, int Category);

public sealed class DbuffInfo
{
    public string? Name { get; init; }
    public string? Aggregate { get; init; }
    public Op Op { get; init; }
    public AttrId[] Item { get; init; } = Array.Empty<AttrId>();
    public AttrId[] Location { get; init; } = Array.Empty<AttrId>();
    public (AttrId Attr, int Group)[] LocationGroup { get; init; } = Array.Empty<(AttrId, int)>();
    public (AttrId Attr, int Skill)[] LocationSkill { get; init; } = Array.Empty<(AttrId, int)>();
}

public sealed class MutaplasmidInfo
{
    public Dictionary<int, (double Lo, double Hi)> Ranges { get; init; } = new();
}

/// <summary>Immutable engine dataset (EXCT format v1). Shared read-only by every calculation.</summary>
public sealed class Dataset
{
    public required long Build { get; init; }
    public string? ReleaseDate { get; init; }
    public required string Sha256 { get; init; }
    public required Dictionary<int, TypeInfo> Types { get; init; }
    public required Dictionary<int, GroupInfo> Groups { get; init; }
    public required AttrInfo?[] AttrById { get; init; }
    public required Dictionary<int, EffectInfo> Effects { get; init; }
    public required Dictionary<int, DbuffInfo> Dbuffs { get; init; }
    public required Dictionary<int, MutaplasmidInfo> Mutaplasmids { get; init; }
    public required Dictionary<int, string> NamesZh { get; init; }
    public required Dictionary<string, int> AttrByName { get; init; }
    public required Dictionary<string, int> EffectByName { get; init; }
    public required Dictionary<string, int> TypeByName { get; init; }
    /// <summary>Published skill type ids, sorted.</summary>
    public required int[] PublishedSkills { get; init; }
    /// <summary>Tactical destroyer modes (group 1306): (lower-case name, id), sorted by id.</summary>
    public required (string Name, int Id)[] TacticalModes { get; init; }

    private KnownIds? _known;
    public KnownIds Known => _known ??= new KnownIds(this);

    public int AttrCount => AttrById.Count(a => a != null);

    public AttrInfo? Attr(AttrId id) => (uint)id.Value < (uint)AttrById.Length ? AttrById[id.Value] : null;
    public double AttrDefault(AttrId id) => Attr(id)?.Default ?? 0.0;
    public AttrId AttrIdOf(string name) => new(AttrByName.TryGetValue(name, out var v) ? v : 0);
    public EffectId EffectIdOf(string name) => new(EffectByName.TryGetValue(name, out var v) ? v : 0);
    public EffectInfo? Effect(EffectId id) => Effects.TryGetValue(id.Value, out var e) ? e : null;
    public TypeInfo? Type(int id) => Types.TryGetValue(id, out var t) ? t : null;
    public int? TypeByNameLookup(string name) => TypeByName.TryGetValue(name.Trim().ToLowerInvariant(), out var v) ? v : null;
    public string AttrName(AttrId id) => Attr(id)?.Name ?? id.Value.ToString();
}
