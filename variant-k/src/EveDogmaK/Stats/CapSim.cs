namespace EveDogmaK.Stats;

/// <summary>One capacitor consumer/producer for the simulation.</summary>
public readonly record struct CapDrain(double DurationMs, double CapNeed, int ClipSize, double ReloadMs, bool IsInjector, bool DisableStagger);

public sealed record CapSimResult(bool Stable, double StableLow, double StableHigh, double TimeS, double EveStable, long Iterations);

/// <summary>
/// Event-driven capacitor simulator, behaviour-compatible with Pyfa eos/capSim.py (LGPL); re-implemented from the
/// reference engine. Events are ordered like Pyfa's heapq of [t, duration, capNeed, shot, clip, reload, isInjector].
/// </summary>
public static class CapSim
{
    private sealed class Ev
    {
        public double T, Duration, CapNeed, Reload;
        public int Shot, Clip;
        public bool Inj;
        public long Seq;
        public Ev Clone() => (Ev)MemberwiseClone();
    }

    private sealed class EvOrder : IComparer<Ev>
    {
        public static readonly EvOrder Instance = new();
        public int Compare(Ev? a, Ev? b)
        {
            int c = a!.T.CompareTo(b!.T); if (c != 0) return c;
            c = a.Duration.CompareTo(b.Duration); if (c != 0) return c;
            c = a.CapNeed.CompareTo(b.CapNeed); if (c != 0) return c;
            c = a.Shot.CompareTo(b.Shot); if (c != 0) return c;
            c = a.Clip.CompareTo(b.Clip); if (c != 0) return c;
            c = a.Reload.CompareTo(b.Reload); if (c != 0) return c;
            c = a.Inj.CompareTo(b.Inj); if (c != 0) return c;
            return a.Seq.CompareTo(b.Seq);
        }
    }

    private static long Gcd(long a, long b) => b == 0 ? a : Gcd(b, a % b);

    public static CapSimResult Simulate(double capacity, double rechargeMs, IReadOnlyList<CapDrain> drains, double startFrac,
        bool reload, bool stagger, double tMaxMs)
    {
        double tau = rechargeMs / 5.0;
        var heap = new PriorityQueue<Ev, Ev>(EvOrder.Instance);
        void Push(Ev e) => heap.Enqueue(e, e);
        long seq = 0;
        long period = 1;
        bool disablePeriod = false;
        var groups = new List<(CapDrain D, int N)>();
        foreach (var d0 in drains)
        {
            var d = d0;
            if (!reload && !d.IsInjector) d = d with { ClipSize = 0, ReloadMs = 0 };
            if (d.DurationMs <= 0.0) continue;
            int gi = groups.FindIndex(g => g.D == d);
            if (gi >= 0) groups[gi] = (groups[gi].D, groups[gi].N + 1);
            else groups.Add((d, 1));
        }
        foreach (var (d0, n) in groups)
        {
            var d = d0;
            if (d.ClipSize > 0) disablePeriod = true;
            if (d.IsInjector)
            {
                for (int i = 0; i < n; i++)
                    Push(new Ev { T = 0, Duration = d.DurationMs, CapNeed = d.CapNeed, Clip = d.ClipSize, Reload = d.ReloadMs, Inj = true, Seq = seq++ });
                continue;
            }
            double duration = d.DurationMs, capNeed = d.CapNeed;
            if (stagger && !d.DisableStagger)
            {
                if (d.ClipSize == 0) duration = Math.Floor(duration / n);
                else
                {
                    double st = (duration * d.ClipSize + d.ReloadMs) / ((double)n * d.ClipSize);
                    for (int i = 1; i < n; i++)
                        Push(new Ev { T = i * st, Duration = duration, CapNeed = capNeed, Clip = d.ClipSize, Reload = d.ReloadMs, Seq = seq++ });
                }
            }
            else capNeed *= n;
            long dur = (long)Math.Max(Math.Round(duration, MidpointRounding.AwayFromZero), 1.0);
            period = period / Gcd(period, dur) * dur;
            Push(new Ev { T = 0, Duration = duration, CapNeed = capNeed, Clip = d.ClipSize, Reload = d.ReloadMs, Seq = seq++ });
        }
        double periodMs = disablePeriod || period > tMaxMs ? tMaxMs : period;

        double capMax = capacity, cap = capacity * startFrac;
        double capWrap = cap, capLowest = cap, capLowestPre = cap, tWrap = periodMs, tLast = 0.0;
        long iterations = 0;
        var awaiting = new List<Ev>();
        List<(double, double)> awaitingWrap = new();
        bool ranOut = false;
        Ev? lastEv = null;

        static List<(double, double)> Key(List<Ev> v) =>
            v.Select(e => (e.Duration, e.CapNeed)).OrderBy(x => x.Item1).ThenBy(x => x.Item2).ToList();

        void Fire(Ev inj, double tNow)
        {
            inj.T = tNow + inj.Duration;
            inj.Shot++;
            if (inj.Clip > 0 && inj.Shot % inj.Clip == 0) { inj.Shot = 0; inj.T += inj.Reload; }
            inj.Seq = seq++;
            Push(inj);
        }

        while (heap.TryDequeue(out var ev, out _))
        {
            double tNow = ev.T;
            if (tNow >= tMaxMs) { lastEv = ev; break; }
            if (tNow > tLast && capMax > 0.0 && tau > 0.0)
            {
                double x = Math.Sqrt(Math.Max(cap / capMax, 0.0));
                double y = 1.0 + (x - 1.0) * Math.Exp((tLast - tNow) / tau);
                cap = y * y * capMax;
            }
            if (tNow != tLast)
            {
                if (cap < capLowestPre) capLowestPre = cap;
                if (tNow == tWrap)
                {
                    var k = Key(awaiting);
                    if (cap >= capWrap && k.SequenceEqual(awaitingWrap)) { lastEv = ev; break; }
                    capWrap = Math.Round(cap * 10.0, MidpointRounding.AwayFromZero) / 10.0;
                    awaitingWrap = k;
                    tWrap += periodMs;
                }
            }
            tLast = tNow;
            iterations++;
            if (iterations > 5_000_000) { lastEv = ev; break; }
            if (ev.Inj && cap - ev.CapNeed > capMax) { awaiting.Add(ev); continue; }
            if (ev.CapNeed > cap && cap < capMax)
            {
                while (awaiting.Count > 0 && ev.CapNeed > cap && capMax > cap)
                {
                    double need = Math.Min(ev.CapNeed - cap, capMax - cap);
                    int pick = -1;
                    for (int i = 0; i < awaiting.Count; i++) // smallest injector that covers the need
                        if (-awaiting[i].CapNeed >= need && (pick < 0 || -awaiting[i].CapNeed < -awaiting[pick].CapNeed)) pick = i;
                    if (pick < 0)
                        for (int i = 0; i < awaiting.Count; i++) // otherwise the biggest (last max wins, like max_by)
                            if (pick < 0 || -awaiting[i].CapNeed >= -awaiting[pick].CapNeed) pick = i;
                    var inj = awaiting[pick];
                    awaiting.RemoveAt(pick);
                    cap = Math.Min(cap - inj.CapNeed, capMax);
                    Fire(inj, tNow);
                }
            }
            cap = Math.Min(cap - ev.CapNeed, capMax);
            if (cap < capLowest)
            {
                if (cap < 0.0) { ranOut = true; lastEv = ev; break; }
                capLowest = cap;
            }
            while (awaiting.Count > 0 && cap < capMax)
            {
                double need = capMax - cap;
                int pick = -1;
                for (int i = 0; i < awaiting.Count; i++) // biggest injector that does not overfill (last max wins)
                    if (-awaiting[i].CapNeed <= need && (pick < 0 || -awaiting[i].CapNeed >= -awaiting[pick].CapNeed)) pick = i;
                if (pick < 0) break;
                var inj = awaiting[pick];
                awaiting.RemoveAt(pick);
                cap = Math.Min(cap - inj.CapNeed, capMax);
                Fire(inj, tNow);
            }
            Fire(ev, tNow);
        }
        // EVE's own stability estimate
        double avgDrain = 0;
        foreach (var (e, _) in heap.UnorderedItems) avgDrain += e.CapNeed / e.Duration;
        if (lastEv != null) avgDrain += lastEv.CapNeed / lastEv.Duration;
        double inner = -(2.0 * avgDrain * tau - capMax) / capMax;
        double eveStable = inner >= 0.0 && capMax > 0.0 ? 0.25 * Math.Pow(1.0 + Math.Sqrt(inner), 2) : 0.0;
        bool stable = !ranOut;
        return new CapSimResult(stable,
            stable && capMax > 0.0 ? capLowest / capMax : 0.0,
            stable && capMax > 0.0 ? capLowestPre / capMax : 0.0,
            tLast / 1000.0, eveStable, iterations);
    }
}
