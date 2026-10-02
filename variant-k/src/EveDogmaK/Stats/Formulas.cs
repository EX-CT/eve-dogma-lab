using EveDogmaK.Requests;

namespace EveDogmaK.Stats;

/// <summary>Closed-form game formulas (Pyfa-equivalent).</summary>
public static class Formulas
{
    /// <summary>Projected effect strength at distance: 1 inside optimal, 0.5^((d-opt)/falloff)^2 in falloff (0 beyond opt+3*falloff if restricted).</summary>
    public static double RangeFactor(double optimal, double falloff, double? distance, bool restricted)
    {
        if (distance is not double d) return 1.0;
        if (falloff > 0.0)
        {
            if (restricted && d > optimal + 3.0 * falloff) return 0.0;
            return Math.Pow(0.5, Math.Pow(Math.Max(d - optimal, 0.0) / falloff, 2));
        }
        return d <= optimal ? 1.0 : 0.0;
    }

    /// <summary>Lock time in seconds: 40000 / scanRes / asinh(sig)^2, capped at 1800 s.</summary>
    public static double? LockTime(double scanRes, double sig)
    {
        if (scanRes <= 0.0 || sig <= 0.0) return null;
        double a = Math.Asinh(sig);
        return Math.Min(40000.0 / scanRes / (a * a), 1800.0);
    }

    /// <summary>Python round(v, 2): correctly rounded decimal of the exact binary value (IEEE-exact "F2" formatting).</summary>
    public static double PyRound2(double v) =>
        double.IsFinite(v) ? double.Parse(v.ToString("F2", System.Globalization.CultureInfo.InvariantCulture), System.Globalization.CultureInfo.InvariantCulture) : v;

    /// <summary>Pyfa eos.utils.float.floatUnerr: keep 7 significant digits (used for doomsday subcycle counts).</summary>
    public static double FloatUnerr7(double v)
    {
        if (v == 0.0 || !double.IsFinite(v)) return v;
        int rf = 7 - (int)Math.Ceiling(Math.Log10(Math.Abs(v)));
        if (rf >= 0) return double.Parse(v.ToString("F" + rf, System.Globalization.CultureInfo.InvariantCulture), System.Globalization.CultureInfo.InvariantCulture);
        double p = Math.Pow(10, -rf);
        return Math.Round(v / p, MidpointRounding.ToEven) * p;
    }

    /// <summary>Pyfa floatUnerr: round to 9 decimals to kill float noise before floor/ceil.</summary>
    public static double FloatUnerr(double v) => Math.Round(v * 1e9, MidpointRounding.AwayFromZero) / 1e9;

    /// <summary>Spool-up weapons (Pyfa calculateSpoolup): (bonus, cycles, time).</summary>
    public static (double Value, double Cycles, double Time) Spoolup(double max, double step, double cycleS, Spool spool)
    {
        if (max == 0.0 || step == 0.0) return (0, 0, 0);
        double cycles = spool.Type switch
        {
            SpoolType.SpoolScale => Math.Ceiling(FloatUnerr(max * spool.Amount / step)),
            SpoolType.CycleScale => Math.Round(spool.Amount * Math.Ceiling(FloatUnerr(max / step)), MidpointRounding.AwayFromZero),
            SpoolType.Time => Math.Min(Math.Floor(FloatUnerr(spool.Amount / cycleS)), Math.Ceiling(FloatUnerr(max / step))),
            _ => Math.Min(Math.Floor(spool.Amount), Math.Ceiling(FloatUnerr(max / step))),
        };
        double v = Math.Min(cycles * step, max);
        return (v, cycles, cycles * cycleS);
    }

    /// <summary>Peak recharge rate of a capacitor or shield: 10/τ * 0.25 * capacity (τ = full recharge time in s).</summary>
    public static double PeakRecharge(double capacity, double rechargeMs) =>
        rechargeMs > 0.0 ? 10.0 / (rechargeMs / 1000.0) * 0.5 * 0.5 * capacity : 0.0;
}
