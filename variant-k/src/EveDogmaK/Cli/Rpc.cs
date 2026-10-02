using System.Text.Json;
using EveDogmaK.Data;
using EveDogmaK.Eft;
using EveDogmaK.Engine;
using EveDogmaK.Json;
using EveDogmaK.Requests;
using EveDogmaK.Stats;

namespace EveDogmaK.Cli;

/// <summary>JSONL RPC (serve-stdio) and the search/type/meta helpers.</summary>
public static class Rpc
{
    public static JObj Handle(Dataset ds, string line)
    {
        JsonDocument doc;
        try { doc = JsonDocument.Parse(line); }
        catch (JsonException e) { return new JObj { { "id", JNode.Null }, { "error", new JObj { { "code", "BAD_JSON" }, { "message", e.Message } } } }; }
        using (doc)
        {
            var root = doc.RootElement;
            JNode id = root.TryGetProperty("id", out var idEl) ? Raw(idEl) : JNode.Null;
            var p = root.TryGetProperty("params", out var pe) ? pe : default;
            string method = root.TryGetProperty("method", out var me) && me.ValueKind == JsonValueKind.String ? me.GetString()! : "calc";
            JNode result;
            switch (method)
            {
                case "calc":
                    try { result = Calculator.Calc(ds, RequestParser.Fit(p, "")); }
                    catch (RequestException e) { result = new JObj { { "error", new JObj { { "code", "BAD_REQUEST" }, { "message", e.Message } } } }; }
                    break;
                case "eft_parse":
                    try
                    {
                        var text = p.ValueKind == JsonValueKind.Object && p.TryGetProperty("text", out var te) && te.ValueKind == JsonValueKind.String ? te.GetString()! : "";
                        result = RequestWriter.Write(EftFormat.Parse(ds, text));
                    }
                    catch (EftParseException e) { result = Error("EFT_PARSE", e.Message); }
                    break;
                case "eft_export":
                    try
                    {
                        var fit = p.ValueKind == JsonValueKind.Object && p.TryGetProperty("fit", out var fe) ? fe : default;
                        var name = p.ValueKind == JsonValueKind.Object && p.TryGetProperty("name", out var ne) && ne.ValueKind == JsonValueKind.String ? ne.GetString()! : "EXCT fit";
                        result = new JObj { { "text", EftFormat.Export(ds, RequestParser.Fit(fit, ""), name) } };
                    }
                    catch (RequestException e) { result = Error("BAD_REQUEST", e.Message); }
                    break;
                case "search":
                {
                    bool obj = p.ValueKind == JsonValueKind.Object;
                    string query = obj && p.TryGetProperty("query", out var q) && q.ValueKind == JsonValueKind.String ? q.GetString()! : "";
                    int limit = obj && p.TryGetProperty("limit", out var l) && l.TryGetInt32(out var li) && li >= 0 ? li : 20;
                    List<string>? kinds = obj && p.TryGetProperty("kinds", out var k) && k.ValueKind == JsonValueKind.Array
                        ? k.EnumerateArray().Where(x => x.ValueKind == JsonValueKind.String).Select(x => x.GetString()!).ToList() : null;
                    result = Search(ds, query, limit, kinds);
                    break;
                }
                case "type":
                    result = TypeInfo(ds, p.ValueKind == JsonValueKind.Object && p.TryGetProperty("id", out var t)
                        ? (t.ValueKind == JsonValueKind.String ? t.GetString()! : t.GetRawText()) : "");
                    break;
                case "meta": result = Meta(ds); break;
                default: result = new JObj { { "error", new JObj { { "code", "UNKNOWN_METHOD" }, { "message", method } } } }; break;
            }
            return new JObj { { "id", id }, { "result", result } };
        }
    }

    private static JNode Raw(JsonElement e) => e.ValueKind switch
    {
        JsonValueKind.String => e.GetString(),
        JsonValueKind.Number => e.TryGetInt64(out var l) ? (JNode)l : (JNode)e.GetDouble(),
        JsonValueKind.True => true,
        JsonValueKind.False => false,
        _ => JNode.Null,
    };

    private static JObj Error(string code, string message) => new() { { "error", new JObj { { "code", code }, { "message", message } } } };

    /// <summary>Search kinds of the interim spec (contract 1.4.1): category -> kind; category 20 splits implant / booster.</summary>
    private static readonly (string Kind, int Category)[] SearchCategories =
        { ("ship", 6), ("module", 7), ("charge", 8), ("drone", 18), ("fighter", 87), ("implant", 20), ("subsystem", 32), ("skill", 16) };

    private static string? SearchKind(Dataset ds, Data.TypeInfo t)
    {
        if (t.Category == 20)
            return ds.Groups.TryGetValue(t.Group, out var g) && g.Name.Contains("Booster", StringComparison.Ordinal) ? "booster" : "implant";
        foreach (var (k, c) in SearchCategories) if (c == t.Category) return k;
        return null;
    }

    /// <summary>Interim search: published types of the scored kinds, exact &gt; prefix &gt; substring (English or Chinese, case-insensitive), ties by type id.</summary>
    public static JArr Search(Dataset ds, string q, int limit, List<string>? kinds)
    {
        var ql = q.Trim().ToLowerInvariant();
        int? Rank(Data.TypeInfo t)
        {
            var en = t.Name.ToLowerInvariant();
            var zh = ds.NamesZh.TryGetValue(t.Id, out var z) ? z.ToLowerInvariant() : "";
            if (en == ql || (zh.Length > 0 && zh == ql)) return 0;
            if (en.StartsWith(ql, StringComparison.Ordinal) || (zh.Length > 0 && zh.StartsWith(ql, StringComparison.Ordinal))) return 1;
            if (en.Contains(ql, StringComparison.Ordinal) || (zh.Length > 0 && zh.Contains(ql, StringComparison.Ordinal))) return 2;
            return null;
        }
        var hits = new List<(int Rank, int Id, string Kind, Data.TypeInfo T)>();
        foreach (var t in ds.Types.Values)
        {
            if (!t.Published || SearchKind(ds, t) is not { } kind) continue;
            if (kinds != null && !kinds.Contains(kind)) continue;
            if (Rank(t) is int r) hits.Add((r, t.Id, kind, t));
        }
        string[] matchNames = { "exact", "prefix", "substring" };
        return new JArr(hits.OrderBy(h => h.Rank).ThenBy(h => h.Id).Take(limit).Select(h => (JNode)new JObj
        {
            { "type_id", h.T.Id }, { "name", h.T.Name }, { "name_zh", ds.NamesZh.GetValueOrDefault(h.T.Id) }, { "kind", h.Kind }, { "match", matchNames[h.Rank] },
            { "group", ds.Groups.TryGetValue(h.T.Group, out var g) ? g.Name : null }, { "category_id", h.T.Category },
            { "meta_level", JNode.Of(h.T.MetaLevel) }, { "slot", FitBuilder.InferSlot(h.T) is { } s ? StatsCalculator.SlotName(s) : null },
        }));
    }

    public static JObj TypeInfo(Dataset ds, string key)
    {
        int? id = int.TryParse(key.Trim(), out var v) ? v : ds.TypeByNameLookup(key);
        var t = id is int i ? ds.Type(i) : null;
        if (t == null) return new JObj { { "error", new JObj { { "code", "UNKNOWN_TYPE" }, { "message", key } } } };
        var attrs = new JObj();
        foreach (var (a, val) in t.RawAttrs.Entries()) attrs[ds.AttrName(a)] = val;
        return new JObj
        {
            { "type_id", t.Id }, { "name", t.Name }, { "name_zh", ds.NamesZh.GetValueOrDefault(t.Id) },
            { "group", ds.Groups.TryGetValue(t.Group, out var g) ? g.Name : null }, { "group_id", t.Group }, { "category_id", t.Category },
            { "published", t.Published }, { "mass", t.Mass }, { "volume", t.Volume }, { "capacity", t.Capacity },
            { "slot", FitBuilder.InferSlot(t) is { } s ? StatsCalculator.SlotName(s) : null }, { "attributes", attrs },
            { "effects", new JArr(t.Effects.Select(e => (JNode)new JObj { { "id", e.Id.Value }, { "name", ds.Effect(e.Id)?.Name }, { "default", e.IsDefault } })) },
        };
    }

    public static JObj Meta(Dataset ds) => new()
    {
        { "engine", StatsCalculator.EngineName }, { "schema_version", 1 }, { "sde_build", ds.Build }, { "sde_release_date", ds.ReleaseDate },
        { "dataset_sha256", ds.Sha256 }, { "types", ds.Types.Count }, { "attributes", ds.AttrCount }, { "effects", ds.Effects.Count },
    };
}
