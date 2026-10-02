using System.Globalization;
using EveDogmaK.Data;
using EveDogmaK.Engine;
using EveDogmaK.Requests;

namespace EveDogmaK.Eft;

public sealed class EftParseException(string message) : Exception(message);

/// <summary>
/// EFT fitting text import/export. The export matches Pyfa's <c>service/port/eft.py exportEft</c> byte for byte (all
/// options on, after Pyfa's <c>fill()</c>; contract 1.4.1 ruling 4). The behaviour follows the reference engine's
/// clean-room implementation.
/// </summary>
public static class EftFormat
{
    // ------------------------------------------------------------------ import

    private static string[] Lines(string text)
    {
        var l = text.Split('\n');
        for (int i = 0; i < l.Length; i++) l[i] = l[i].TrimEnd('\r');
        // like Rust str::lines: a trailing newline does not produce an extra empty line
        return text.EndsWith('\n') ? l[..^1] : l;
    }

    /// <summary>Strip a trailing " [N]" mutation reference.</summary>
    private static (string Line, int? Ref) MutationRef(string line)
    {
        var l = line.TrimEnd();
        if (l.EndsWith(']'))
        {
            int p = l.LastIndexOf(" [", StringComparison.Ordinal);
            if (p >= 0 && uint.TryParse(l.AsSpan(p + 2, l.Length - p - 3), NumberStyles.None, CultureInfo.InvariantCulture, out var n))
                return (l[..p].TrimEnd(), (int)n);
        }
        return (l, null);
    }

    private static bool IsMutationHead(string line)
    {
        var t = line.Trim();
        if (!t.StartsWith('[')) return false;
        int e = t.IndexOf(']');
        return e > 0 && uint.TryParse(t.AsSpan(1, e - 1), NumberStyles.None, CultureInfo.InvariantCulture, out _);
    }

    /// <summary>Trailing mutation blocks: "[N] Base Name" / "  Mutaplasmid Name" / "  attr value, attr value".</summary>
    private static (Dictionary<int, Mutation> Mutations, int FirstLine) ParseMutations(Dataset ds, string[] lines)
    {
        var result = new Dictionary<int, Mutation>();
        int first = Array.FindIndex(lines, IsMutationHead);
        if (first < 0) first = lines.Length;
        int i = first;
        while (i < lines.Length)
        {
            var t = lines[i].Trim();
            if (!IsMutationHead(t)) { i++; continue; }
            int e = t.IndexOf(']');
            int n = int.Parse(t.AsSpan(1, e - 1), CultureInfo.InvariantCulture);
            var baseName = t[(e + 1)..].Trim();
            int baseId = ds.TypeByNameLookup(baseName) ?? throw new EftParseException($"unknown mutated base '{baseName}'");
            int? muta = null;
            var attrs = new Dictionary<string, double>();
            i++;
            while (i < lines.Length && !IsMutationHead(lines[i]))
            {
                var l = lines[i].Trim();
                i++;
                if (l.Length == 0) continue;
                if (muta == null)
                {
                    muta = ds.TypeByNameLookup(l) ?? throw new EftParseException($"unknown mutaplasmid '{l}'");
                    continue;
                }
                foreach (var part in l.Split(','))
                {
                    var kv = part.Trim();
                    int sp = kv.LastIndexOf(' ');
                    if (sp < 0) continue;
                    var aid = ds.AttrIdOf(kv[..sp].Trim());
                    if (!aid.IsNone && double.TryParse(kv[(sp + 1)..].Trim(), NumberStyles.Float, CultureInfo.InvariantCulture, out var v))
                        attrs[aid.Value.ToString(CultureInfo.InvariantCulture)] = v;
                }
            }
            result[n] = new Mutation(baseId, muta, attrs.ToList());
        }
        return (result, first);
    }

    /// <summary>The mutated type a base type turns into with this mutaplasmid.</summary>
    private static int MutatedType(Dataset ds, Mutation m)
    {
        if (m.MutaplasmidTypeId is int id && ds.Mutaplasmids.TryGetValue(id, out var mu))
            foreach (var (inputs, output) in mu.Mapping)
                if (Array.IndexOf(inputs, m.BaseTypeId) >= 0) return output;
        return m.BaseTypeId;
    }

    public static FitRequest Parse(Dataset ds, string text)
    {
        var all = Lines(text);
        var (muts, firstMut) = ParseMutations(ds, all);
        var lines = all.Take(firstMut).Select(l => l.Trim()).Where(l => l.Length > 0).ToList();
        if (lines.Count == 0) throw new EftParseException("empty EFT");
        var h = lines[0].TrimStart('[').TrimEnd(']');
        var shipName = h.Split(',')[0].Trim();
        int ship = ds.TypeByNameLookup(shipName) ?? throw new EftParseException($"unknown ship '{shipName}'");
        int? mode = null;
        var modules = new List<ModuleReq>(); var drones = new List<DroneReq>(); var fighters = new List<FighterReq>();
        var implants = new List<int>(); var boosters = new List<BoosterReq>(); var cargo = new List<CargoReq>();
        var boosterness = ds.AttrIdOf("boosterness");

        foreach (var raw in lines.Skip(1))
        {
            var line = raw;
            if (line.StartsWith("[Empty", StringComparison.Ordinal)) continue;
            bool offline = false;
            if (line.EndsWith("/OFFLINE", StringComparison.Ordinal) || line.EndsWith("/offline", StringComparison.Ordinal))
            {
                line = line[..^"/offline".Length].Trim();
                offline = true;
            }
            (line, var mref) = MutationRef(line);
            Mutation? mutation = null;
            if (mref is int n0) mutation = muts.TryGetValue(n0, out var mm) ? mm : throw new EftParseException($"mutation [{n0}] not defined");

            // "Name xN" => drone / fighter / cargo
            int xpos = line.LastIndexOf(" x", StringComparison.Ordinal);
            if (xpos >= 0 && uint.TryParse(line[(xpos + 2)..].Trim(), NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out var qty))
            {
                var name = line[..xpos].Trim();
                int tid = ds.TypeByNameLookup(name) ?? throw new EftParseException($"unknown item '{name}'");
                if (mutation != null) tid = MutatedType(ds, mutation);
                switch (ds.Type(tid)!.Category)
                {
                    case 18: drones.Add(new DroneReq(tid, (int)qty, (int)qty, mutation)); break;
                    case 87: fighters.Add(new FighterReq(tid, (int)qty, true, null)); break;
                    default: cargo.Add(new CargoReq(tid, (int)qty)); break;
                }
                continue;
            }
            int comma = line.IndexOf(',');
            var itemName = (comma >= 0 ? line[..comma] : line).Trim();
            string? chargeName = comma >= 0 ? line[(comma + 1)..].Trim() : null;
            int type = ds.TypeByNameLookup(itemName) ?? throw new EftParseException($"unknown item '{itemName}'");
            if (mutation != null) type = MutatedType(ds, mutation);
            var t = ds.Type(type)!;
            switch (t.Category)
            {
                case 20:
                    // implants vs boosters: boosters carry the boosterness attribute
                    if (t.RawAttrs.TryGet(boosterness, out _)) boosters.Add(new BoosterReq(type, Array.Empty<int>()));
                    else implants.Add(type);
                    break;
                case 18: drones.Add(new DroneReq(type, 1, 1, mutation)); break;
                case 8: cargo.Add(new CargoReq(type, 1)); break;
                default:
                    if (t.Group == 1306) { mode = type; break; } // T3D mode
                    var slot = FitBuilder.InferSlot(t);
                    int? charge = chargeName == null ? null : ds.TypeByNameLookup(chargeName) ?? throw new EftParseException($"unknown charge '{chargeName}'");
                    bool activeCapable = t.Effects.Any(e => ds.Effect(e.Id)?.Category == EffectCategory.Active)
                        || (t.RawAttrs.TryGet(new AttrId(6), out var cn) && cn != 0.0);
                    var state = offline ? ModuleState.Offline
                        : activeCapable && slot is not (Slot.Rig or Slot.Subsystem) ? ModuleState.Active : ModuleState.Online;
                    modules.Add(new ModuleReq(type, slot, state, charge, mutation, null));
                    break;
            }
        }
        return new FitRequest(ship, mode, null, Array.Empty<KeyValuePair<string, int>>(), null, modules, drones, fighters, implants, boosters,
            cargo, Array.Empty<Buff>(), Array.Empty<FitRequest>(), Array.Empty<ProjectedReq>(), Array.Empty<int>(), null, null, null,
            Array.Empty<Override>(), Options.Default);
    }

    // ------------------------------------------------------------------ export

    /// <summary>Python <c>repr(float)</c> of Pyfa's <c>floatUnerr(v)</c> (7 significant digits), as Pyfa prints mutated values.</summary>
    public static string PyFloat(double v)
    {
        if (v != 0.0 && double.IsFinite(v))
        {
            int rf = 7 - (int)Math.Ceiling(Math.Log10(Math.Abs(v)));
            if (rf >= 0) v = double.Parse(v.ToString("F" + rf, CultureInfo.InvariantCulture), CultureInfo.InvariantCulture);
            else { double p = Math.Pow(10, -rf); v = Math.Round(v / p, MidpointRounding.AwayFromZero) * p; }
        }
        if (double.IsPositiveInfinity(v)) return "inf";
        if (double.IsNegativeInfinity(v)) return "-inf";
        double a = Math.Abs(v);
        if (a != 0.0 && (a < 1e-4 || a >= 1e16))
        {
            // Python exponent notation: 1e-05, 1.5e+16
            var s = v.ToString("E16", CultureInfo.InvariantCulture);
            var r = v.ToString("R", CultureInfo.InvariantCulture); // shortest digits
            int ei = s.IndexOf('E');
            int exp = int.Parse(s[(ei + 1)..], CultureInfo.InvariantCulture);
            var digits = r.Replace("-", "").Replace(".", "");
            int rE = digits.IndexOfAny(new[] { 'E', 'e' });
            if (rE >= 0) digits = digits[..rE];
            digits = digits.TrimStart('0').TrimEnd('0');
            if (digits.Length == 0) digits = "0";
            var mant = digits.Length == 1 ? digits : digits[0] + "." + digits[1..];
            return (v < 0 ? "-" : "") + mant + "e" + (exp < 0 ? "-" : "+") + Math.Abs(exp).ToString("00", CultureInfo.InvariantCulture);
        }
        if (v == Math.Truncate(v)) return v.ToString("F1", CultureInfo.InvariantCulture);
        return v.ToString("R", CultureInfo.InvariantCulture);
    }

    /// <summary>Pyfa's drone market-group order (service/port/eft.py DRONE_ORDER).</summary>
    private static int DroneOrder(int? marketGroup) => marketGroup switch
    {
        837 or 1531 => 0, 3881 => 1, 838 or 1532 => 2, 3882 => 3, 839 or 359 => 4, 3883 => 5, 911 or 1533 => 6,
        843 or 1586 => 7, 841 or 1029 => 8, 842 or 1030 => 9, 158 or 358 => 10, 1643 or 1646 => 11, _ => 12,
    };

    private static readonly string[] FighterOrder =
        { "Light Fighter", "Structure Light Fighter", "Heavy Fighter", "Structure Heavy Fighter", "Support Fighter", "Structure Support Fighter" };

    public static string Export(Dataset ds, FitRequest req, string name)
    {
        string N(int id) => ds.Type(id)?.Name ?? id.ToString(CultureInfo.InvariantCulture);
        double Attr(int id, string a) => ds.Type(id) is { } t && t.Attrs.TryGet(ds.AttrIdOf(a), out var v) ? v : 0.0;
        GroupInfo? GroupOf(int id) => ds.Type(id) is { } t && ds.Groups.TryGetValue(t.Group, out var g) ? g : null;
        // slot totals after modifiers (subsystems, structure rigs, ...)
        Fit? totals = null;
        try { totals = FitBuilder.Build(ds, req); } catch (EngineException) { }
        long Total(string a) => totals == null ? 0 : (long)totals.Get(totals.Ship, ds.AttrIdOf(a));
        var muts = new List<Mutation>();
        var sections = new List<string>();

        // 1: modules by rack
        var racks = new List<string>();
        foreach (var (slot, label, attr) in new[]
        {
            (Slot.Low, "Low", "lowSlots"), (Slot.Mid, "Med", "medSlots"), (Slot.High, "High", "hiSlots"),
            (Slot.Rig, "Rig", "rigSlots"), (Slot.Subsystem, "Subsystem", "maxSubSystems"), (Slot.Service, "Service", "serviceSlots"),
        })
        {
            var lines = new List<string>();
            foreach (var m in req.Modules.Where(m => (m.Slot ?? (ds.Type(m.TypeId) is { } t ? FitBuilder.InferSlot(t) : null)) == slot))
            {
                var l = m.Mutation != null ? N(m.Mutation.BaseTypeId) : N(m.TypeId);
                var tag = "";
                if (m.Mutation is { MutaplasmidTypeId: not null }) { muts.Add(m.Mutation); tag = $" [{muts.Count}]"; }
                if (m.ChargeTypeId is int c) l += ", " + N(c);
                if (m.State == ModuleState.Offline) l += " /offline";
                lines.Add(l + tag);
            }
            for (long free = Total(attr) - lines.Count; free > 0; free--) lines.Add($"[Empty {label} slot]");
            if (lines.Count > 0) racks.Add(string.Join("\n", lines));
        }
        if (racks.Count > 0) sections.Add(string.Join("\n\n", racks));

        // 2: drones, fighters
        var minions = new List<string>();
        int DroneBase(DroneReq d) => d.Mutation?.BaseTypeId ?? d.TypeId;
        bool DroneMutated(DroneReq d) => d.Mutation?.MutaplasmidTypeId != null;
        var drones = req.Drones
            .OrderBy(d => DroneOrder(ds.Type(DroneBase(d))?.MarketGroup)).ThenBy(DroneMutated)
            .ThenBy(d => DroneMutated(d) ? ds.Type(d.TypeId)?.Name ?? "" : N(d.TypeId), StringComparer.Ordinal)
            .ToList();
        var dl = new List<string>();
        foreach (var d in drones)
        {
            var tag = "";
            if (DroneMutated(d)) { muts.Add(d.Mutation!); tag = $" [{muts.Count}]"; }
            dl.Add($"{N(DroneBase(d))} x{d.Quantity}{tag}");
        }
        if (dl.Count > 0) minions.Add(string.Join("\n", dl));
        var fl = req.Fighters
            .OrderBy(f => { int k = Array.IndexOf(FighterOrder, GroupOf(f.TypeId)?.Name ?? ""); return k < 0 ? FighterOrder.Length : k; })
            .ThenBy(f => N(f.TypeId), StringComparer.Ordinal)
            .Select(f =>
            {
                int max = (int)(uint)Attr(f.TypeId, "fighterSquadronMaxSize");
                int q = f.Quantity is int qq ? (qq >= max ? max : qq) : max;
                return $"{N(f.TypeId)} x{q}";
            }).ToList();
        if (fl.Count > 0) minions.Add(string.Join("\n", fl));
        if (minions.Count > 0) sections.Add(string.Join("\n\n", minions));

        // 3: implants (by implantness), boosters (by boosterness)
        var character = new List<string>();
        var imps = req.Implants.OrderBy(i => Attr(i, "implantness")).ToList();
        if (imps.Count > 0) character.Add(string.Join("\n", imps.Select(N)));
        var boos = req.Boosters.Select(b => b.TypeId).OrderBy(i => Attr(i, "boosterness")).ToList();
        if (boos.Count > 0) character.Add(string.Join("\n", boos.Select(N)));
        if (character.Count > 0) sections.Add(string.Join("\n\n", character));

        // 4: cargo by (category name, group name, type name)
        var cargo = req.Cargo
            .OrderBy(c => GroupOf(c.TypeId) is { } g && ds.Categories.TryGetValue(g.Category, out var cn) ? cn : "", StringComparer.Ordinal)
            .ThenBy(c => GroupOf(c.TypeId)?.Name ?? "", StringComparer.Ordinal)
            .ThenBy(c => N(c.TypeId), StringComparer.Ordinal).ToList();
        if (cargo.Count > 0) sections.Add(string.Join("\n", cargo.Select(c => $"{N(c.TypeId)} x{c.Quantity}")));

        // 5: mutation details
        if (muts.Count > 0)
        {
            var blocks = muts.Select((m, k) =>
            {
                var kv = m.Attributes
                    .Select(a => (Name: int.TryParse(a.Key, NumberStyles.None, CultureInfo.InvariantCulture, out var id) && ds.Attr(new AttrId(id)) is { } ai ? ai.Name : a.Key, a.Value))
                    .OrderBy(x => x.Name, StringComparer.Ordinal);
                var attrs = string.Join(", ", kv.Select(x => $"{x.Name} {PyFloat(x.Value)}"));
                return $"[{k + 1}] {N(m.BaseTypeId)}\n  {N(m.MutaplasmidTypeId!.Value)}\n  {attrs}";
            });
            sections.Add(string.Join("\n", blocks));
        }
        return $"[{N(req.ShipTypeId)}, {name}]\n\n{string.Join("\n\n\n", sections)}";
    }
}
