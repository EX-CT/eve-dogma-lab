using System.IO.Compression;
using System.Security.Cryptography;
using System.Text.Json;

namespace EveDogmaK.Data;

/// <summary>Loads dataset-&lt;build&gt;.json.gz (format "exct-eve-dataset" v1, produced by EX-CT/eve-sde-pipeline).</summary>
public static class DatasetLoader
{
    private static readonly int[] RequiredSkillAttrs = { 182, 183, 184, 1285, 1289, 1290 };
    private static readonly HashSet<string> CentiRounded = new() { "cpu", "power", "cpuOutput", "powerOutput" };

    public static Dataset LoadPath(string path) => LoadBytes(File.ReadAllBytes(path));

    public static Dataset LoadBytes(byte[] bytes)
    {
        byte[] json = bytes;
        if (bytes.Length > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b)
        {
            using var gz = new GZipStream(new MemoryStream(bytes), CompressionMode.Decompress);
            using var ms = new MemoryStream(bytes.Length * 12);
            gz.CopyTo(ms);
            json = ms.ToArray();
        }
        string sha = Convert.ToHexString(SHA256.HashData(json)).ToLowerInvariant();
        using var doc = JsonDocument.Parse(json, new JsonDocumentOptions { MaxDepth = 64 });
        var root = doc.RootElement;
        if (Str(root, "format") != "exct-eve-dataset" || root.GetProperty("format_version").GetInt32() != 1)
            throw new InvalidDataException($"unsupported dataset format {Str(root, "format")}");

        // attributes
        var attrProps = root.GetProperty("attributes");
        int maxAttr = 0;
        foreach (var p in attrProps.EnumerateObject()) maxAttr = Math.Max(maxAttr, int.Parse(p.Name));
        var attrById = new AttrInfo?[maxAttr + 1];
        var attrByName = new Dictionary<string, int>();
        foreach (var p in attrProps.EnumerateObject())
        {
            int id = int.Parse(p.Name);
            var a = p.Value;
            string name = Str(a, "name") ?? "";
            attrByName[name] = id;
            attrById[id] = new AttrInfo
            {
                Id = new AttrId(id), Name = name, Default = Num(a, "default") ?? 0.0,
                Stackable = Bool(a, "stackable") ?? true, HighIsGood = Bool(a, "high_is_good") ?? true,
                MinAttr = OptAttr(a, "min_attr"), MaxAttr = OptAttr(a, "max_attr"),
                RoundToCentis = CentiRounded.Contains(name),
            };
        }

        // effects
        var effects = new Dictionary<int, EffectInfo>();
        var effectByName = new Dictionary<string, int>();
        foreach (var p in root.GetProperty("effects").EnumerateObject())
        {
            int id = int.Parse(p.Name);
            var e = p.Value;
            string name = Str(e, "name") ?? "";
            effectByName[name] = id;
            var mods = new List<Modifier>();
            if (e.TryGetProperty("mods", out var me) && me.ValueKind == JsonValueKind.Array)
                foreach (var m in me.EnumerateArray())
                {
                    int f = m[0].GetInt32(), d = m[1].GetInt32(), op = m[4].GetInt32();
                    mods.Add(new Modifier(FuncOf(f), DomainOf(d), new AttrId(m[2].GetInt32()), new AttrId(m[3].GetInt32()),
                        OpOf(op), m[5].GetInt32()));
                }
            effects[id] = new EffectInfo
            {
                Id = new EffectId(id), Name = name, Category = (EffectCategory)(byte)(Num(e, "category") ?? 0),
                DurationAttr = OptAttr(e, "duration_attr"), RangeAttr = OptAttr(e, "range_attr"),
                FalloffAttr = OptAttr(e, "falloff_attr"), ResistanceAttr = OptAttr(e, "resistance_attr"),
                FittingUsageChanceAttr = OptAttr(e, "fitting_usage_chance_attr"),
                IsOffensive = Bool(e, "is_offensive") ?? false, IsAssistance = Bool(e, "is_assistance") ?? false,
                Modifiers = mods.ToArray(),
            };
        }

        var groups = new Dictionary<int, GroupInfo>();
        foreach (var p in root.GetProperty("groups").EnumerateObject())
            groups[int.Parse(p.Name)] = new GroupInfo(Str(p.Value, "name") ?? "", (int)(Num(p.Value, "category") ?? 0));

        // types
        var rawTypes = new List<RawType>();
        foreach (var p in root.GetProperty("types").EnumerateObject())
        {
            var t = p.Value;
            var raw = new List<KeyValuePair<int, double>>();
            if (t.TryGetProperty("attrs", out var at) && at.ValueKind == JsonValueKind.Object)
                foreach (var a in at.EnumerateObject()) raw.Add(new(int.Parse(a.Name), a.Value.GetDouble()));
            var effs = new List<EffectRef>();
            if (t.TryGetProperty("effects", out var te) && te.ValueKind == JsonValueKind.Array)
                foreach (var x in te.EnumerateArray()) effs.Add(new EffectRef(new EffectId(x[0].GetInt32()), x[1].GetInt32() != 0));
            rawTypes.Add(new RawType(int.Parse(p.Name), Str(t, "name") ?? "", (int)(Num(t, "group") ?? 0), (int)(Num(t, "category") ?? 0),
                Bool(t, "published") ?? false, Num(t, "mass") ?? 0, Num(t, "volume") ?? 0, Num(t, "capacity") ?? 0, Num(t, "radius") ?? 0,
                t.TryGetProperty("meta_level", out var ml) && ml.ValueKind == JsonValueKind.Number ? ml.GetInt32() : null,
                raw.ToArray(), effs.ToArray(),
                t.TryGetProperty("market_group", out var mg) && mg.ValueKind == JsonValueKind.Number ? mg.GetInt32() : null));
        }
        var dbuffs = new Dictionary<int, DbuffInfo>();
        if (root.TryGetProperty("dbuffs", out var db))
            foreach (var p in db.EnumerateObject())
            {
                var b = p.Value;
                dbuffs[int.Parse(p.Name)] = new DbuffInfo
                {
                    Name = Str(b, "name"), Aggregate = Str(b, "aggregate"), Op = OpOf((int)(Num(b, "op") ?? 0)),
                    Item = b.GetProperty("item").EnumerateArray().Select(x => new AttrId(x.GetInt32())).ToArray(),
                    Location = b.GetProperty("location").EnumerateArray().Select(x => new AttrId(x.GetInt32())).ToArray(),
                    LocationGroup = b.GetProperty("location_group").EnumerateArray().Select(x => (new AttrId(x[0].GetInt32()), x[1].GetInt32())).ToArray(),
                    LocationSkill = b.GetProperty("location_skill").EnumerateArray().Select(x => (new AttrId(x[0].GetInt32()), x[1].GetInt32())).ToArray(),
                };
            }
        var mutas = new Dictionary<int, MutaplasmidInfo>();
        if (root.TryGetProperty("mutaplasmids", out var mu))
            foreach (var p in mu.EnumerateObject())
            {
                var r = new Dictionary<int, (double, double)>();
                foreach (var a in p.Value.GetProperty("attrs").EnumerateObject())
                    r[int.Parse(a.Name)] = (a.Value[0].GetDouble(), a.Value[1].GetDouble());
                var map = new List<(int[], int)>();
                if (p.Value.TryGetProperty("mapping", out var mp) && mp.ValueKind == JsonValueKind.Array)
                    foreach (var x in mp.EnumerateArray())
                        map.Add((x.GetProperty("inputs").EnumerateArray().Select(v => v.GetInt32()).ToArray(), x.GetProperty("output").GetInt32()));
                mutas[int.Parse(p.Name)] = new MutaplasmidInfo { Ranges = r, Mapping = map.ToArray() };
            }
        var zh = new Dictionary<int, string>();
        if (root.TryGetProperty("names", out var nm) && nm.TryGetProperty("zh", out var z))
            foreach (var p in z.EnumerateObject()) zh[int.Parse(p.Name)] = p.Value.GetString() ?? "";

        var cats = new Dictionary<int, string>();
        if (root.TryGetProperty("categories", out var ce))
            foreach (var p in ce.EnumerateObject()) cats[int.Parse(p.Name)] = Str(p.Value, "name") ?? "";
        var sde = root.GetProperty("sde");
        return Assemble(sde.GetProperty("build").GetInt64(), Str(sde, "release_date"), sha,
            attrById.Where(x => x != null).ToList()!, effects.Values.ToList(), groups, rawTypes, dbuffs, mutas, zh, cats);
    }

    /// <summary>Type as stored in the dataset (before derived fields are computed).</summary>
    public sealed record RawType(int Id, string Name, int Group, int Category, bool Published, double Mass, double Volume,
        double Capacity, double Radius, int? MetaLevel, KeyValuePair<int, double>[] Attrs, EffectRef[] Effects, int? MarketGroup = null);

    /// <summary>Build the immutable <see cref="Dataset"/> (indexes and derived per-type fields) from raw tables.</summary>
    public static Dataset Assemble(long build, string? releaseDate, string sha, List<AttrInfo> attrList, List<EffectInfo> effectList,
        Dictionary<int, GroupInfo> groups, List<RawType> rawTypes, Dictionary<int, DbuffInfo> dbuffs,
        Dictionary<int, MutaplasmidInfo> mutas, Dictionary<int, string> zh, Dictionary<int, string>? categories = null)
    {
        var attrById = new AttrInfo?[attrList.Count == 0 ? 1 : attrList.Max(a => a.Id.Value) + 1];
        var attrByName = new Dictionary<string, int>();
        foreach (var a in attrList.OrderBy(a => a.Id.Value)) { attrById[a.Id.Value] = a; attrByName[a.Name] = a.Id.Value; }
        var effects = new Dictionary<int, EffectInfo>();
        var effectByName = new Dictionary<string, int>();
        foreach (var e in effectList.OrderBy(e => e.Id.Value)) { effects[e.Id.Value] = e; effectByName[e.Name] = e.Id.Value; }
        var types = new Dictionary<int, TypeInfo>(rawTypes.Count);
        var skills = new List<int>();
        var modes = new List<(string, int)>();
        var set = new (int, double)[4];
        foreach (var r in rawTypes)
        {
            var raw = AttrTable.From(r.Attrs);
            // type-level fields are authoritative (mass 4 / capacity 38 / volume 161 / radius 162)
            int ns = 0;
            foreach (var (aid, v) in new[] { (4, r.Mass), (38, r.Capacity), (161, r.Volume), (162, r.Radius) })
                if (v != 0.0 || !raw.TryGet(new AttrId(aid), out _)) set[ns++] = (aid, v);
            var req = new List<int>();
            foreach (var a in RequiredSkillAttrs)
                if (raw.TryGet(new AttrId(a), out var v) && (int)v != 0) req.Add((int)v);
            var ti = new TypeInfo
            {
                Id = r.Id, Name = r.Name, Group = r.Group, Category = r.Category, Published = r.Published, Mass = r.Mass,
                Volume = r.Volume, Capacity = r.Capacity, Radius = r.Radius, MetaLevel = r.MetaLevel, MarketGroup = r.MarketGroup,
                RawAttrs = raw, Attrs = raw.With(set.AsSpan(0, ns)), Effects = r.Effects, RequiredSkills = req.ToArray(), Raw = r,
            };
            types[r.Id] = ti;
            if (r.Category == 16 && r.Published) skills.Add(r.Id);
            if (r.Group == 1306) modes.Add((r.Name.ToLowerInvariant(), r.Id));
        }
        var typeByName = new Dictionary<string, int>();
        foreach (var id in types.Keys.OrderBy(x => x))
        {
            var t = types[id];
            var key = t.Name.ToLowerInvariant();
            if (t.Published || !typeByName.ContainsKey(key)) typeByName[key] = id;
        }
        skills.Sort();
        modes.Sort((a, b) => a.Item2.CompareTo(b.Item2));
        return new Dataset
        {
            Build = build, ReleaseDate = releaseDate, Sha256 = sha,
            Types = types, Groups = groups, AttrById = attrById, Effects = effects, Dbuffs = dbuffs, Mutaplasmids = mutas,
            NamesZh = zh, Categories = categories ?? new(), AttrByName = attrByName, EffectByName = effectByName, TypeByName = typeByName,
            PublishedSkills = skills.ToArray(), TacticalModes = modes.ToArray(),
        };
    }

    private static ModFunc FuncOf(int c) => c switch
    {
        0 => ModFunc.Item, 1 => ModFunc.Location, 2 => ModFunc.LocationGroup, 3 => ModFunc.LocationRequiredSkill,
        4 => ModFunc.OwnerRequiredSkill, _ => ModFunc.EffectStopper,
    };
    private static ModDomain DomainOf(int c) => c switch
    {
        0 => ModDomain.Item, 1 => ModDomain.Ship, 2 => ModDomain.Char, 3 => ModDomain.Other, 4 => ModDomain.Structure,
        5 => ModDomain.TargetId, 6 => ModDomain.Target, _ => ModDomain.None,
    };
    private static Op OpOf(int c) => c is >= -1 and <= 7 ? (Op)c : Op.Unsupported;

    private static string? Str(JsonElement e, string k) => e.TryGetProperty(k, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;
    private static double? Num(JsonElement e, string k) => e.TryGetProperty(k, out var v) && v.ValueKind == JsonValueKind.Number ? v.GetDouble() : null;
    private static bool? Bool(JsonElement e, string k) => e.TryGetProperty(k, out var v) && (v.ValueKind == JsonValueKind.True || v.ValueKind == JsonValueKind.False) ? v.GetBoolean() : null;
    private static AttrId? OptAttr(JsonElement e, string k) => Num(e, k) is double d ? new AttrId((int)d) : null;
}
