using System.Text.Json;

namespace EveDogmaK.Requests;

public sealed class RequestException : Exception
{
    public string Code { get; }
    public string Path { get; }
    public RequestException(string code, string message, string path) : base(message) { Code = code; Path = path; }
}

/// <summary>
/// Hand-written, reflection-free mapping JsonElement -> typed FitRequest (Native-AOT friendly).
/// Unknown fields are ignored; null means "default" for every optional field.
/// </summary>
public static class RequestParser
{
    public static FitRequest Parse(string json)
    {
        JsonDocument doc;
        try { doc = JsonDocument.Parse(json, new JsonDocumentOptions { MaxDepth = 256 }); }
        catch (JsonException e) { throw new RequestException("BAD_REQUEST", e.Message, ""); }
        using (doc) return Fit(doc.RootElement, "");
    }

    public static FitRequest Fit(JsonElement e, string path)
    {
        if (e.ValueKind != JsonValueKind.Object) throw Bad(path, "expected object");
        var ship = Req(e, "ship", path);
        if (ship.ValueKind != JsonValueKind.Object) throw Bad(path + "/ship", "expected object");
        int shipId = ReqInt(ship, "type_id", path + "/ship");
        int? mode = OptInt(ship, "mode_type_id", path + "/ship");

        int? defLevel = null; var levels = new List<KeyValuePair<string, int>>(); double? sec = null;
        if (Opt(e, "character") is { } ch)
        {
            sec = OptNum(ch, "security_status", path + "/character");
            if (Opt(ch, "skills") is { } sk)
            {
                defLevel = OptInt(sk, "default_level", path + "/character/skills");
                if (Opt(sk, "levels") is { ValueKind: JsonValueKind.Object } lv)
                    foreach (var p in lv.EnumerateObject())
                        levels.Add(new(p.Name, Int(p.Value, path + "/character/skills/levels/" + p.Name)));
            }
        }
        var modules = Arr(e, "modules", path, (x, p) => Module(x, p));
        var drones = Arr(e, "drones", path, (x, p) => Drone(x, p));
        var fighters = Arr(e, "fighters", path, (x, p) => Fighter(x, p));
        var implants = Arr(e, "implants", path, (x, p) => Int(x, p));
        var boosters = Arr(e, "boosters", path, (x, p) => new BoosterReq(ReqInt(x, "type_id", p),
            Opt(x, "side_effects") is { ValueKind: JsonValueKind.Array } se ? se.EnumerateArray().Select(a => Int(a, p)).ToArray() : Array.Empty<int>()));
        var cargo = Arr(e, "cargo", path, (x, p) => new CargoReq(ReqInt(x, "type_id", p), OptInt(x, "quantity", p) ?? 1));
        var buffs = new List<Buff>();
        var boosterFits = new List<FitRequest>();
        if (Opt(e, "fleet") is { } fl)
        {
            buffs = Arr(fl, "buffs", path + "/fleet", (x, p) => new Buff(ReqInt(x, "buff_id", p), ReqNum(x, "value", p)));
            boosterFits = Arr(fl, "booster_fits", path + "/fleet", (x, p) => Fit(x, p));
        }
        var projected = Arr(e, "projected", path, (x, p) => new ProjectedReq(
            ReqStr(x, "kind", p),
            Opt(x, "module") is { ValueKind: JsonValueKind.Object } m ? Module(m, p + "/module") : null,
            Opt(x, "drone") is { ValueKind: JsonValueKind.Object } d ? Drone(d, p + "/drone") : null,
            Opt(x, "fit") is { ValueKind: JsonValueKind.Object } f ? Fit(f, p + "/fit") : null,
            OptInt(x, "amount", p) ?? 1, OptNum(x, "distance_m", p),
            Opt(x, "fighter") is { ValueKind: JsonValueKind.Object } fi ? Fighter(fi, p + "/fighter") : null));
        var envEffects = new List<int>(); string? secLevel = null;
        if (Opt(e, "environment") is { } env)
        {
            envEffects = Arr(env, "effect_type_ids", path + "/environment", (x, p) => Int(x, p));
            secLevel = OptStr(env, "system_security", path + "/environment");
        }
        DamageProfile? dp = Opt(e, "damage_pattern") is { ValueKind: JsonValueKind.Object } dpe ? Profile(dpe, path + "/damage_pattern") : null;
        TargetProfile? tp = Opt(e, "target_profile") is { ValueKind: JsonValueKind.Object } tpe
            ? new TargetProfile(Profile(tpe, path + "/target_profile"), OptNum(tpe, "signature_radius", path), OptNum(tpe, "max_velocity", path), OptNum(tpe, "radius", path))
            : null;
        var overrides = Arr(e, "overrides", path, (x, p) => new Override(ReqInt(x, "type_id", p), ReqInt(x, "attribute_id", p), ReqNum(x, "value", p)));
        var opts = Options.Default;
        if (Opt(e, "options") is { ValueKind: JsonValueKind.Object } o)
        {
            string op = path + "/options";
            var cs = new CapSimOptions(false, false, null);
            if (Opt(o, "cap_sim") is { ValueKind: JsonValueKind.Object } c)
                cs = new CapSimOptions(OptBool(c, "reload", op) ?? false, OptBool(c, "stagger", op) ?? false, OptNum(c, "max_time_s", op));
            opts = new Options(OptBool(o, "nos_no_target_cap", op) ?? false, OptBool(o, "factor_reload", op) ?? false,
                Opt(o, "default_spool") is { ValueKind: JsonValueKind.Object } ds ? SpoolOf(ds, op + "/default_spool") : null,
                OptStr(o, "rah", op), OptStr(o, "include_attributes", op), OptBool(o, "sources", op) ?? false,
                OptBool(o, "validate", op) ?? true, cs);
        }
        return new FitRequest(shipId, mode, defLevel, levels, sec, modules, drones, fighters, implants, boosters, cargo, buffs, boosterFits, projected,
            envEffects, secLevel, dp, tp, overrides, opts);
    }

    private static ModuleReq Module(JsonElement x, string p) => new(
        ReqInt(x, "type_id", p),
        OptStr(x, "slot", p) is { } s ? s switch
        {
            "high" => Slot.High, "mid" => Slot.Mid, "low" => Slot.Low, "rig" => Slot.Rig, "subsystem" => Slot.Subsystem,
            "service" => Slot.Service, _ => throw Bad(p + "/slot", $"unknown variant `{s}`"),
        } : null,
        OptStr(x, "state", p) is { } st ? st switch
        {
            "offline" => ModuleState.Offline, "online" => ModuleState.Online, "active" => ModuleState.Active,
            "overheated" => ModuleState.Overheated, _ => throw Bad(p + "/state", $"unknown variant `{st}`"),
        } : null,
        OptInt(x, "charge_type_id", p),
        Opt(x, "mutation") is { ValueKind: JsonValueKind.Object } m ? MutationOf(m, p + "/mutation") : null,
        Opt(x, "spool") is { ValueKind: JsonValueKind.Object } sp ? SpoolOf(sp, p + "/spool") : null);

    private static FighterReq Fighter(JsonElement x, string p) => new(ReqInt(x, "type_id", p), OptInt(x, "quantity", p),
        OptBool(x, "active", p) ?? true, Opt(x, "abilities") is { ValueKind: JsonValueKind.Array } ab ? ab.EnumerateArray().Select(a => Int(a, p)).ToArray() : null);

    private static DroneReq Drone(JsonElement x, string p) => new(ReqInt(x, "type_id", p), OptInt(x, "quantity", p) ?? 1,
        OptInt(x, "active", p), Opt(x, "mutation") is { ValueKind: JsonValueKind.Object } m ? MutationOf(m, p + "/mutation") : null);

    private static Mutation MutationOf(JsonElement m, string p)
    {
        var attrs = new List<KeyValuePair<string, double>>();
        if (Opt(m, "attributes") is { ValueKind: JsonValueKind.Object } a)
            foreach (var kv in a.EnumerateObject()) attrs.Add(new(kv.Name, Num(kv.Value, p + "/attributes/" + kv.Name)));
        attrs.Sort((x, y) => string.CompareOrdinal(x.Key, y.Key)); // BTreeMap order, like the reference
        return new Mutation(ReqInt(m, "base_type_id", p), OptInt(m, "mutaplasmid_type_id", p), attrs);
    }

    private static Spool SpoolOf(JsonElement s, string p) => new(ReqStr(s, "type", p) switch
    {
        "spool_scale" => SpoolType.SpoolScale, "cycle_scale" => SpoolType.CycleScale, "time" => SpoolType.Time, "cycles" => SpoolType.Cycles,
        var o => throw Bad(p + "/type", $"unknown variant `{o}`"),
    }, ReqNum(s, "amount", p));

    private static DamageProfile Profile(JsonElement e, string p) =>
        new(OptNum(e, "em", p) ?? 0, OptNum(e, "thermal", p) ?? 0, OptNum(e, "kinetic", p) ?? 0, OptNum(e, "explosive", p) ?? 0);

    // ---- primitive helpers
    private static RequestException Bad(string path, string msg) => new("BAD_REQUEST", msg, path);
    private static JsonElement? Opt(JsonElement e, string k) =>
        e.ValueKind == JsonValueKind.Object && e.TryGetProperty(k, out var v) && v.ValueKind != JsonValueKind.Null ? v : null;
    private static JsonElement Req(JsonElement e, string k, string p) => Opt(e, k) ?? throw Bad(p, $"missing field `{k}`");
    private static int Int(JsonElement v, string p)
    {
        if (v.ValueKind == JsonValueKind.Number && v.TryGetInt64(out var l) && l >= int.MinValue && l <= int.MaxValue) return (int)l;
        throw Bad(p, $"invalid type: expected integer, got {v.ValueKind}");
    }
    private static double Num(JsonElement v, string p) =>
        v.ValueKind == JsonValueKind.Number ? v.GetDouble() : throw Bad(p, $"invalid type: expected number, got {v.ValueKind}");
    private static int ReqInt(JsonElement e, string k, string p) => Int(Req(e, k, p), p + "/" + k);
    private static double ReqNum(JsonElement e, string k, string p) => Num(Req(e, k, p), p + "/" + k);
    private static string ReqStr(JsonElement e, string k, string p) => OptStr(e, k, p) ?? throw Bad(p, $"missing field `{k}`");
    private static int? OptInt(JsonElement e, string k, string p) => Opt(e, k) is { } v ? Int(v, p + "/" + k) : null;
    private static double? OptNum(JsonElement e, string k, string p) => Opt(e, k) is { } v ? Num(v, p + "/" + k) : null;
    private static bool? OptBool(JsonElement e, string k, string p) => Opt(e, k) is { } v
        ? v.ValueKind is JsonValueKind.True or JsonValueKind.False ? v.GetBoolean() : throw Bad(p + "/" + k, "invalid type: expected boolean")
        : null;
    private static string? OptStr(JsonElement e, string k, string p) => Opt(e, k) is { } v
        ? v.ValueKind == JsonValueKind.String ? v.GetString() : throw Bad(p + "/" + k, "invalid type: expected string")
        : null;
    private static List<T> Arr<T>(JsonElement e, string k, string p, Func<JsonElement, string, T> f)
    {
        var list = new List<T>();
        if (Opt(e, k) is not { } a) return list;
        if (a.ValueKind != JsonValueKind.Array) throw Bad(p + "/" + k, "invalid type: expected sequence");
        int i = 0;
        foreach (var x in a.EnumerateArray()) { list.Add(f(x, $"{p}/{k}/{i}")); i++; }
        return list;
    }
}
