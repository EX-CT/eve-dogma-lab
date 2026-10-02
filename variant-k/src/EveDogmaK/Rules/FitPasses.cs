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
        // Pyfa applies buffs stacking-penalised, except the abyssal weather resistance / HP / velocity buffs
        int neverExempt = buffId is 90 or 93 or 94 or 95 or 96 or 98 or 99 ? Fit.ShipCategory : 0;
        int ship = fit.Ship;
        var t = new List<int>();
        foreach (var a in info.Item) fit.AddModifier(ship, a, info.Op, src, sourceItem, neverExempt);
        // AoE cloud / weather buffs also hit drones that require the Drones skill (Pyfa fit.py commandBonus)
        string[] droneAttrs = buffId switch
        {
            79 => new[] { "signatureRadius" },
            90 => new[] { "shieldEmDamageResonance", "armorEmDamageResonance", "emDamageResonance" },
            93 => new[] { "shieldExplosiveDamageResonance", "armorExplosiveDamageResonance", "explosiveDamageResonance" },
            95 => new[] { "shieldThermalDamageResonance", "armorThermalDamageResonance", "thermalDamageResonance" },
            99 => new[] { "shieldKineticDamageResonance", "armorKineticDamageResonance", "kineticDamageResonance" },
            94 => new[] { "shieldCapacity" },
            96 => new[] { "armorHP" },
            97 => new[] { "maxRange", "falloff" },
            98 => new[] { "maxVelocity" },
            _ => Array.Empty<string>(),
        };
        if (droneAttrs.Length > 0)
        {
            const int dronesSkill = 3436;
            foreach (var d in fit.Items.Where(x => x.Kind == ItemKind.Drone && x.RequiresSkill(dronesSkill)).Select(x => x.Index).ToList())
                foreach (var n in droneAttrs)
                {
                    var a = fit.Ds.AttrIdOf(n);
                    if (!a.IsNone) fit.AddModifier(d, a, info.Op, src, sourceItem, neverExempt);
                }
        }
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

/// <summary>
/// Warfare buffs. Candidates per buff id: the fit's own active command bursts (warfareBuffNID / warfareBuffNValue, read
/// modified from the module because the warfare charge PostAssigns the id and PostMuls the value onto it) and the
/// active bursts of every <c>fleet.booster_fits</c> ship (computed on its own). Like Pyfa, only the single strongest
/// (by |value|) source per buff id applies. Explicit <c>fleet.buffs</c> (aggregated Maximum, or Minimum for
/// "Minimum" collections) override both.
/// </summary>
public sealed class WarfareBuffPass : IFitPass
{
    public string Name => "warfare-buffs";

    public void Run(Fit fit)
    {
        var agg = new Dictionary<int, double>();
        foreach (var b in fit.Request.FleetBuffs)
        {
            if (!fit.Ds.Dbuffs.TryGetValue(b.BuffId, out var info)) { fit.Warnings.Add($"unknown warfare buff {b.BuffId}"); continue; }
            agg[b.BuffId] = !agg.TryGetValue(b.BuffId, out var cur) ? b.Value
                : info.Aggregate == "Minimum" ? Math.Min(cur, b.Value) : Math.Max(cur, b.Value);
        }
        var best = new Dictionary<int, (double Value, ModSource Src)>();
        void Offer(int id, double v, ModSource src)
        {
            if (best.TryGetValue(id, out var old) && Math.Abs(old.Value) >= Math.Abs(v)) return;
            best[id] = (v, src);
        }
        foreach (var (id, idx, valAttr, v) in ActiveBursts(fit, agg)) Offer(id, v, ModSource.FromAttr(idx, valAttr));
        // abyssal weather / AoE cloud beacons (Pyfa weather_* / aoe_beacon_* effects): warfareBuff1/2 of the environment
        // item join the same pool (strongest |value| per buff id)
        for (int i = 0; i < fit.Items.Count; i++)
        {
            var it = fit[i];
            if (it.Kind != ItemKind.Beacon) continue;
            bool weather = it.Effects.Any(er => fit.Ds.Effect(er.Id) is { } ei
                && (ei.Name.StartsWith("weather_", StringComparison.Ordinal) || ei.Name.StartsWith("aoe_beacon_", StringComparison.Ordinal)));
            if (!weather) continue;
            foreach (var (idAttr, valAttr) in fit.K.WarfareBuffs.Take(2))
            {
                int id = fit.Has(i, idAttr) ? (int)fit.Get(i, idAttr) : 0;
                if (id == 0 || agg.ContainsKey(id)) continue;
                double v = fit.Get(i, valAttr);
                Offer(id, v, ModSource.Const(v));
            }
        }
        for (int k = 0; k < fit.Request.BoosterFits.Count; k++)
        {
            Fit booster;
            try { booster = FitBuilder.Build(fit.Ds, fit.Request.BoosterFits[k] with { BoosterFits = Array.Empty<FitRequest>() }); }
            catch (EngineException e)
            {
                fit.Warnings.Add($"fleet.booster_fits[{k}]: EngineError {{ code: \"{e.Code}\", message: \"{e.Message}\", path: \"{e.Path}\" }}");
                continue;
            }
            foreach (var (id, _, _, v) in ActiveBursts(booster, agg)) Offer(id, v, ModSource.Const(v));
        }
        foreach (var (id, v) in agg) best[id] = (v, ModSource.Const(v));
        foreach (var id in best.Keys.OrderBy(x => x))
        {
            var src = best[id].Src;
            int target = src.Kind == SourceKind.Attribute ? src.Item : fit.Ship;
            WarfareBuffs.Apply(fit, id, src, target);
        }
        fit.InvalidateCache();
    }

    /// <summary>(buff id, module index, value) of every warfareBuffN slot on active modules, skipping explicitly given ids.</summary>
    private static List<(int Id, int Module, AttrId ValueAttr, double Value)> ActiveBursts(Fit f, Dictionary<int, double> explicitIds)
    {
        var list = new List<(int, int, AttrId, double)>();
        for (int i = 0; i < f.Items.Count; i++)
        {
            var it = f[i];
            if (it.Kind != ItemKind.Module || it.State < ModuleState.Active) continue;
            foreach (var (idAttr, valAttr) in f.K.WarfareBuffs)
            {
                int id = f.Has(i, idAttr) ? (int)f.Get(i, idAttr) : 0;
                if (id == 0 || explicitIds.ContainsKey(id)) continue;
                list.Add((id, i, valAttr, f.Get(i, valAttr)));
            }
        }
        return list;
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
