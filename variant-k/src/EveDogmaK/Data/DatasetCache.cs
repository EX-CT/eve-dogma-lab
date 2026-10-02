using System.IO.Compression;
using System.Security.Cryptography;
using System.Text;

namespace EveDogmaK.Data;

/// <summary>
/// Derived binary cache of the dataset (allowed by the brief): the first load parses the JSON dataset and writes a
/// compact binary image keyed by the SHA-256 of the dataset file; later process starts read the image instead
/// (several times faster than gunzip + JSON parse). The cache is only an accelerator: any mismatch or I/O problem falls
/// back to the JSON dataset. Location: $EVE_DOGMA_K_CACHE, else $XDG_CACHE_HOME/eve-dogma-k, else ~/.cache/eve-dogma-k.
/// Set EVE_DOGMA_K_CACHE=off to disable.
/// </summary>
public static class DatasetCache
{
    private const string Magic = "EVEDOGMAK-CACHE-5";

    public static Dataset Load(string path)
    {
        var bytes = File.ReadAllBytes(path);
        var dir = CacheDir();
        if (dir == null) return DatasetLoader.LoadBytes(bytes);
        string key = Convert.ToHexString(SHA256.HashData(bytes)).ToLowerInvariant();
        string file = Path.Combine(dir, key[..32] + ".bin");
        try
        {
            if (File.Exists(file))
            {
                var ds = Read(file, key);
                if (ds != null) return ds;
            }
        }
        catch (Exception) { /* corrupt or old cache: rebuild */ }
        var fresh = DatasetLoader.LoadBytes(bytes);
        try
        {
            Directory.CreateDirectory(dir);
            var tmp = file + "." + Environment.ProcessId + ".tmp";
            Write(fresh, tmp, key);
            File.Move(tmp, file, overwrite: true);
        }
        catch (Exception) { /* read-only cache dir: fine */ }
        return fresh;
    }

    private static string? CacheDir()
    {
        var env = Environment.GetEnvironmentVariable("EVE_DOGMA_K_CACHE");
        if (env == "off" || env == "0") return null;
        if (!string.IsNullOrEmpty(env)) return env;
        var xdg = Environment.GetEnvironmentVariable("XDG_CACHE_HOME");
        if (!string.IsNullOrEmpty(xdg)) return Path.Combine(xdg, "eve-dogma-k");
        var home = Environment.GetEnvironmentVariable("HOME");
        return string.IsNullOrEmpty(home) ? null : Path.Combine(home, ".cache", "eve-dogma-k");
    }

    // ------------------------------------------------------------------ format
    private static void Opt(BinaryWriter w, AttrId? a) => w.Write(a?.Value ?? -1);
    private static AttrId? OptAttr(BinaryReader r) { int v = r.ReadInt32(); return v < 0 ? null : new AttrId(v); }
    private static void OptStr(BinaryWriter w, string? s) { w.Write(s != null); if (s != null) w.Write(s); }
    private static string? OptStr(BinaryReader r) => r.ReadBoolean() ? r.ReadString() : null;

    public static void Write(Dataset ds, string file, string key)
    {
        using var fs = File.Create(file);
        using var bw = new BinaryWriter(new BufferedStream(fs, 1 << 16), Encoding.UTF8);
        bw.Write(Magic); bw.Write(key); bw.Write(ds.Sha256); bw.Write(ds.Build); OptStr(bw, ds.ReleaseDate);
        var attrs = ds.AttrById.Where(a => a != null).ToList();
        bw.Write(attrs.Count);
        foreach (var a in attrs)
        {
            bw.Write(a!.Id.Value); bw.Write(a.Name); bw.Write(a.Default); bw.Write(a.Stackable); bw.Write(a.HighIsGood);
            Opt(bw, a.MinAttr); Opt(bw, a.MaxAttr); bw.Write(a.RoundToCentis);
        }
        bw.Write(ds.Effects.Count);
        foreach (var e in ds.Effects.Values)
        {
            bw.Write(e.Id.Value); bw.Write(e.Name); bw.Write((byte)e.Category);
            Opt(bw, e.DurationAttr); Opt(bw, e.RangeAttr); Opt(bw, e.FalloffAttr); Opt(bw, e.ResistanceAttr); Opt(bw, e.FittingUsageChanceAttr);
            bw.Write(e.IsOffensive); bw.Write(e.IsAssistance);
            bw.Write(e.Modifiers.Length);
            foreach (var m in e.Modifiers)
            {
                bw.Write((byte)m.Func); bw.Write((byte)m.Domain); bw.Write(m.Modified.Value); bw.Write(m.Modifying.Value);
                bw.Write((sbyte)m.Op); bw.Write(m.Filter);
            }
        }
        bw.Write(ds.Groups.Count);
        foreach (var (id, g) in ds.Groups) { bw.Write(id); bw.Write(g.Name); bw.Write(g.Category); }
        bw.Write(ds.Types.Count);
        foreach (var t in ds.Types.Values)
        {
            var r = t.Raw!;
            bw.Write(r.Id); bw.Write(r.Name); bw.Write(r.Group); bw.Write(r.Category); bw.Write(r.Published);
            bw.Write(r.Mass); bw.Write(r.Volume); bw.Write(r.Capacity); bw.Write(r.Radius);
            bw.Write(r.MetaLevel.HasValue); bw.Write(r.MetaLevel ?? 0);
            bw.Write(r.MarketGroup.HasValue); bw.Write(r.MarketGroup ?? 0);
            bw.Write(r.Attrs.Length); foreach (var kv in r.Attrs) { bw.Write(kv.Key); bw.Write(kv.Value); }
            bw.Write(r.Effects.Length); foreach (var e in r.Effects) { bw.Write(e.Id.Value); bw.Write(e.IsDefault); }
        }
        bw.Write(ds.Dbuffs.Count);
        foreach (var (id, b) in ds.Dbuffs)
        {
            bw.Write(id); OptStr(bw, b.Name); OptStr(bw, b.Aggregate); bw.Write((sbyte)b.Op);
            bw.Write(b.Item.Length); foreach (var a in b.Item) bw.Write(a.Value);
            bw.Write(b.Location.Length); foreach (var a in b.Location) bw.Write(a.Value);
            bw.Write(b.LocationGroup.Length); foreach (var (a, g) in b.LocationGroup) { bw.Write(a.Value); bw.Write(g); }
            bw.Write(b.LocationSkill.Length); foreach (var (a, s) in b.LocationSkill) { bw.Write(a.Value); bw.Write(s); }
        }
        bw.Write(ds.Mutaplasmids.Count);
        foreach (var (id, m) in ds.Mutaplasmids)
        {
            bw.Write(id); bw.Write(m.Ranges.Count);
            foreach (var (a, (lo, hi)) in m.Ranges) { bw.Write(a); bw.Write(lo); bw.Write(hi); }
            bw.Write(m.Mapping.Length);
            foreach (var (ins, outp) in m.Mapping) { bw.Write(ins.Length); foreach (var x in ins) bw.Write(x); bw.Write(outp); }
        }
        bw.Write(ds.NamesZh.Count);
        foreach (var (id, n) in ds.NamesZh) { bw.Write(id); bw.Write(n); }
        bw.Write(ds.Categories.Count);
        foreach (var (id, n) in ds.Categories) { bw.Write(id); bw.Write(n); }
        bw.Write(Magic);
    }

    public static Dataset? Read(string file, string key)
    {
        using var br = new BinaryReader(new MemoryStream(File.ReadAllBytes(file)), Encoding.UTF8);
        if (br.ReadString() != Magic || br.ReadString() != key) return null;
        string sha = br.ReadString(); long build = br.ReadInt64(); string? date = OptStr(br);
        int n = br.ReadInt32();
        var attrs = new List<AttrInfo>(n);
        for (int i = 0; i < n; i++)
            attrs.Add(new AttrInfo
            {
                Id = new AttrId(br.ReadInt32()), Name = br.ReadString(), Default = br.ReadDouble(), Stackable = br.ReadBoolean(),
                HighIsGood = br.ReadBoolean(), MinAttr = OptAttr(br), MaxAttr = OptAttr(br), RoundToCentis = br.ReadBoolean(),
            });
        n = br.ReadInt32();
        var effects = new List<EffectInfo>(n);
        for (int i = 0; i < n; i++)
        {
            var id = new EffectId(br.ReadInt32()); var name = br.ReadString(); var cat = (EffectCategory)br.ReadByte();
            var dur = OptAttr(br); var rng = OptAttr(br); var fo = OptAttr(br); var res = OptAttr(br); var fuc = OptAttr(br);
            bool off = br.ReadBoolean(), assist = br.ReadBoolean();
            var mods = new Modifier[br.ReadInt32()];
            for (int k = 0; k < mods.Length; k++)
                mods[k] = new Modifier((ModFunc)br.ReadByte(), (ModDomain)br.ReadByte(), new AttrId(br.ReadInt32()), new AttrId(br.ReadInt32()),
                    (Op)br.ReadSByte(), br.ReadInt32());
            effects.Add(new EffectInfo
            {
                Id = id, Name = name, Category = cat, DurationAttr = dur, RangeAttr = rng, FalloffAttr = fo, ResistanceAttr = res,
                FittingUsageChanceAttr = fuc, IsOffensive = off, IsAssistance = assist, Modifiers = mods,
            });
        }
        n = br.ReadInt32();
        var groups = new Dictionary<int, GroupInfo>(n);
        for (int i = 0; i < n; i++) { int id = br.ReadInt32(); groups[id] = new GroupInfo(br.ReadString(), br.ReadInt32()); }
        n = br.ReadInt32();
        var types = new List<DatasetLoader.RawType>(n);
        for (int i = 0; i < n; i++)
        {
            int id = br.ReadInt32(); string name = br.ReadString(); int group = br.ReadInt32(), cat = br.ReadInt32(); bool pub = br.ReadBoolean();
            double mass = br.ReadDouble(), vol = br.ReadDouble(), cap = br.ReadDouble(), rad = br.ReadDouble();
            bool hasMeta = br.ReadBoolean(); int meta = br.ReadInt32();
            bool hasMg = br.ReadBoolean(); int mg = br.ReadInt32();
            var at = new KeyValuePair<int, double>[br.ReadInt32()];
            for (int k = 0; k < at.Length; k++) at[k] = new(br.ReadInt32(), br.ReadDouble());
            var ef = new EffectRef[br.ReadInt32()];
            for (int k = 0; k < ef.Length; k++) ef[k] = new EffectRef(new EffectId(br.ReadInt32()), br.ReadBoolean());
            types.Add(new DatasetLoader.RawType(id, name, group, cat, pub, mass, vol, cap, rad, hasMeta ? meta : null, at, ef, hasMg ? mg : null));
        }
        n = br.ReadInt32();
        var dbuffs = new Dictionary<int, DbuffInfo>(n);
        for (int i = 0; i < n; i++)
        {
            int id = br.ReadInt32(); string? name = OptStr(br), agg = OptStr(br); var op = (Op)br.ReadSByte();
            var item = new AttrId[br.ReadInt32()]; for (int k = 0; k < item.Length; k++) item[k] = new AttrId(br.ReadInt32());
            var loc = new AttrId[br.ReadInt32()]; for (int k = 0; k < loc.Length; k++) loc[k] = new AttrId(br.ReadInt32());
            var lg = new (AttrId, int)[br.ReadInt32()]; for (int k = 0; k < lg.Length; k++) lg[k] = (new AttrId(br.ReadInt32()), br.ReadInt32());
            var ls = new (AttrId, int)[br.ReadInt32()]; for (int k = 0; k < ls.Length; k++) ls[k] = (new AttrId(br.ReadInt32()), br.ReadInt32());
            dbuffs[id] = new DbuffInfo { Name = name, Aggregate = agg, Op = op, Item = item, Location = loc, LocationGroup = lg, LocationSkill = ls };
        }
        n = br.ReadInt32();
        var mutas = new Dictionary<int, MutaplasmidInfo>(n);
        for (int i = 0; i < n; i++)
        {
            int id = br.ReadInt32(); int c = br.ReadInt32();
            var r = new Dictionary<int, (double, double)>(c);
            for (int k = 0; k < c; k++) { int a = br.ReadInt32(); r[a] = (br.ReadDouble(), br.ReadDouble()); }
            var map = new (int[], int)[br.ReadInt32()];
            for (int k = 0; k < map.Length; k++)
            {
                var ins = new int[br.ReadInt32()];
                for (int x = 0; x < ins.Length; x++) ins[x] = br.ReadInt32();
                map[k] = (ins, br.ReadInt32());
            }
            mutas[id] = new MutaplasmidInfo { Ranges = r, Mapping = map };
        }
        n = br.ReadInt32();
        var zh = new Dictionary<int, string>(n);
        for (int i = 0; i < n; i++) { int id = br.ReadInt32(); zh[id] = br.ReadString(); }
        n = br.ReadInt32();
        var cats = new Dictionary<int, string>(n);
        for (int i = 0; i < n; i++) { int id = br.ReadInt32(); cats[id] = br.ReadString(); }
        if (br.ReadString() != Magic) return null;
        return DatasetLoader.Assemble(build, date, sha, attrs, effects, groups, types, dbuffs, mutas, zh, cats);
    }
}
