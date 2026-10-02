using System.Text.Json;
using EveDogmaK.Data;
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
                case "search":
                    result = Search(ds, p.ValueKind == JsonValueKind.Object && p.TryGetProperty("query", out var q) ? q.GetString() ?? "" : "",
                        p.ValueKind == JsonValueKind.Object && p.TryGetProperty("limit", out var l) && l.TryGetInt32(out var li) ? li : 20);
                    break;
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
        JsonValueKind.Number => e.TryGetInt64(out var l) ? l : e.GetDouble(),
        JsonValueKind.True => true,
        JsonValueKind.False => false,
        _ => JNode.Null,
    };

    public static JArr Search(Dataset ds, string q, int limit)
    {
        var ql = q.ToLowerInvariant();
        var hits = ds.Types.Values.Where(t => t.Published &&
                (t.Name.ToLowerInvariant().Contains(ql) || (ds.NamesZh.TryGetValue(t.Id, out var z) && z.Contains(q))))
            .OrderBy(t => !t.Name.ToLowerInvariant().StartsWith(ql)).ThenBy(t => t.Name.Length).ThenBy(t => t.Name, StringComparer.Ordinal)
            .Take(limit);
        return new JArr(hits.Select(t => (JNode)new JObj
        {
            { "type_id", t.Id }, { "name", t.Name }, { "name_zh", ds.NamesZh.GetValueOrDefault(t.Id) },
            { "group", ds.Groups.TryGetValue(t.Group, out var g) ? g.Name : null }, { "category_id", t.Category },
            { "meta_level", JNode.Of(t.MetaLevel) }, { "slot", FitBuilder.InferSlot(t) is { } s ? StatsCalculator.SlotName(s) : null },
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
