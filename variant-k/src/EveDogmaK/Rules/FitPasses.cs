using EveDogmaK.Data;
using EveDogmaK.Engine;
using EveDogmaK.Requests;

namespace EveDogmaK.Rules;

/// <summary>Shared warfare-buff application: a dbuff collection lists item / location / location-group / location-skill targets.</summary>
public static class WarfareBuffs
{
    public static void Apply(Fit fit, int buffId, ModSource src, int sourceItem)
    {
        if (!fit.Ds.Dbuffs.TryGetValue(buffId, out var info)) return;
        const int neverExempt = 0;
        int ship = fit.Ship;
        var t = new List<int>();
        foreach (var a in info.Item) fit.AddModifier(ship, a, info.Op, src, sourceItem, neverExempt);
        foreach (var a in info.Location)
        {
            fit.Targets.Resolve(fit, ship, ModFunc.Location, ModDomain.Ship, 0, t);
            foreach (var x in t) fit.AddModifier(x, a, info.Op, src, sourceItem, neverExempt);
        }
        foreach (var (a, g) in info.LocationGroup)
        {
            fit.Targets.Resolve(fit, ship, ModFunc.LocationGroup, ModDomain.Ship, g, t);
            foreach (var x in t) fit.AddModifier(x, a, info.Op, src, sourceItem, neverExempt);
        }
        foreach (var (a, s) in info.LocationSkill)
        {
            fit.Targets.Resolve(fit, ship, ModFunc.LocationRequiredSkill, ModDomain.Ship, s, t);
            foreach (var x in t) fit.AddModifier(x, a, info.Op, src, sourceItem, neverExempt);
        }
    }
}

/// <summary>Explicit fleet buffs from the request, aggregated per buff id (Maximum, or Minimum for "Minimum" collections).</summary>
public sealed class FleetBuffPass : IFitPass
{
    public string Name => "fleet-buffs";
    public void Run(Fit fit)
    {
        var agg = new SortedDictionary<int, double>();
        foreach (var b in fit.Request.FleetBuffs)
        {
            if (!fit.Ds.Dbuffs.TryGetValue(b.BuffId, out var info)) { fit.Warnings.Add($"unknown warfare buff {b.BuffId}"); continue; }
            agg[b.BuffId] = !agg.TryGetValue(b.BuffId, out var cur) ? b.Value
                : info.Aggregate == "Minimum" ? Math.Min(cur, b.Value) : Math.Max(cur, b.Value);
        }
        foreach (var (id, value) in agg) WarfareBuffs.Apply(fit, id, ModSource.Const(value), fit.Ship);
    }
}

/// <summary>
/// Local command bursts: active modules expose warfareBuffNID / warfareBuffNValue (the warfare charge PostAssigns the id and
/// PostMuls the value onto the module), so both are read, modified, from the module. Explicit fleet buffs take precedence.
/// </summary>
public sealed class CommandBurstPass : IFitPass
{
    public string Name => "command-bursts";
    public void Run(Fit fit)
    {
        var explicitIds = fit.Request.FleetBuffs.Select(b => b.BuffId).ToHashSet();
        int n = fit.Items.Count;
        for (int i = 0; i < n; i++)
        {
            var it = fit[i];
            if (it.Kind != ItemKind.Module || it.State < ModuleState.Active) continue;
            foreach (var (idAttr, valAttr) in fit.K.WarfareBuffs)
            {
                int id = fit.Has(i, idAttr) ? (int)fit.Get(i, idAttr) : 0;
                if (id == 0 || explicitIds.Contains(id)) continue;
                WarfareBuffs.Apply(fit, id, ModSource.FromAttr(i, valAttr), i);
            }
        }
    }
}

/// <summary>
/// Reactive Armor Hardener adaptation (no modifierInfo in the SDE). Simulates RAH cycles against the incoming damage pattern
/// (after the ship's other armor resists) until the resist profile loops, averages the loop, and applies the averaged
/// resonances as a stacking-penalised PreMul. Same algorithm as Pyfa/eos (LGPL; re-implemented from the reference engine).
/// options.rah = "disable" applies the unadapted resonances instead.
/// </summary>
public sealed class ReactiveArmorHardenerPass : IFitPass
{
    public string Name => "reactive-armor-hardener";
    private static readonly int[] TieOrder = { 0, 3, 2, 1 }; // in-game tie order: em, explosive, kinetic, thermal

    public void Run(Fit fit)
    {
        var k = fit.K;
        if (k.AdaptiveArmorHardener.IsNone) return;
        var attrs = k.ArmorResonance;
        var rahs = fit.Items.Where(it => it.Kind == ItemKind.Module && it.State >= ModuleState.Active && it.HasEffect(k.AdaptiveArmorHardener))
            .Select(it => it.Index).ToList();
        bool disable = fit.Request.Options.Rah == "disable";
        var dp = fit.Request.DamagePattern ?? DamageProfile.Uniform;
        int ship = fit.Ship;
        foreach (var m in rahs)
        {
            fit.InvalidateCache();
            var res = attrs.Select(a => fit.Get(m, a)).ToArray();
            if (!disable) res = Adapt(fit, m, ship, attrs, dp, res);
            int cat = fit[m].Category;
            for (int x = 0; x < 4; x++)
            {
                if (!disable) fit.AddModifier(m, attrs[x], Op.PostAssign, ModSource.Const(res[x]), m, cat);
                fit.AddModifier(ship, attrs[x], Op.PreMul, ModSource.Const(res[x]), m, cat);
            }
        }
        fit.InvalidateCache();
    }

    private static double[] Adapt(Fit fit, int m, int ship, AttrId[] attrs, DamageProfile dp, double[] res)
    {
        var baseDmg = Enumerable.Range(0, 4).Select(x => dp[x] * fit.Get(ship, attrs[x])).ToArray();
        double shift = fit.Get(m, fit.K.ResistanceShiftAmount) / 100.0;
        var cycles = new List<double[]>();
        int loopStart = -20;
        for (int iter = 0; iter < 50; iter++)
        {
            var t = TieOrder.Select(x => (Idx: x, Taken: baseDmg[x] * res[x], Res: res[x])).OrderBy(x => x.Taken).ToArray(); // stable
            double c0, c1, c2, c3;
            if (t[2].Taken == 0.0)
            {
                c0 = 1.0 - t[0].Res; c1 = 1.0 - t[1].Res; c2 = 1.0 - t[2].Res; c3 = -(c0 + c1 + c2);
            }
            else if (t[1].Taken == 0.0)
            {
                c0 = 1.0 - t[0].Res; c1 = 1.0 - t[1].Res; c2 = -(c0 + c1) / 2.0; c3 = c2;
            }
            else
            {
                c0 = Math.Min(shift, 1.0 - t[0].Res); c1 = Math.Min(shift, 1.0 - t[1].Res); c2 = -(c0 + c1) / 2.0; c3 = c2;
            }
            res[t[0].Idx] = t[0].Res + c0; res[t[1].Idx] = t[1].Res + c1; res[t[2].Idx] = t[2].Res + c2; res[t[3].Idx] = t[3].Res + c3;
            int found = cycles.FindIndex(v => Enumerable.Range(0, 4).All(x => Math.Abs(res[x] - v[x]) <= 1e-6));
            if (found >= 0) { loopStart = found; break; }
            cycles.Add((double[])res.Clone());
        }
        int start = loopStart >= 0 ? loopStart : Math.Max(cycles.Count - 20, 0);
        var loop = cycles.Skip(start).ToList();
        if (loop.Count > 0)
            for (int x = 0; x < 4; x++)
                res[x] = Math.Round(loop.Sum(v => v[x]) / loop.Count * 1000.0, MidpointRounding.AwayFromZero) / 1000.0;
        return res;
    }
}
