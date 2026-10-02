using EveDogmaK.Data;
using EveDogmaK.Requests;

namespace EveDogmaK.Engine;

/// <summary>
/// The object graph of one calculation (ship, character, skills, modules, charges, drones, ...)
/// with lazily evaluated, memoised dogma attributes. Built by <see cref="FitBuilder"/>, read by the stats layer.
/// Single-threaded and per-request: no state survives the calculation.
/// </summary>
public sealed partial class Fit
{
    /// <summary>Source categories exempt from stacking penalties: Ship, Charge, Skill, Implant, Subsystem, Structure.</summary>
    public static readonly int[] StackingExemptCategories = { 6, 8, 16, 20, 32, 65 };
    public const int ShipCategory = 6;

    public Dataset Ds { get; }
    public KnownIds K { get; }
    public FitRequest Request { get; }
    public List<Item> Items { get; } = new(512);
    public int Ship { get; internal set; }
    public int Char { get; internal set; }
    public bool IsStructure { get; internal set; }
    public List<string> Warnings { get; } = new();
    /// <summary>Incoming remote reps / neuts / cap transfers from projected items.</summary>
    public List<IncomingEffect> Incoming { get; } = new();

    private int _generation;

    internal Fit(Dataset ds, FitRequest req) { Ds = ds; K = ds.Known; Request = req; }

    public Item this[int i] => Items[i];

    /// <summary>Forget every memoised value (used between RAH adaptation rounds).</summary>
    public void InvalidateCache() => _generation++;

    // ------------------------------------------------------------------ attribute access
    public bool Has(int item, AttrId a) => Items[item].Nodes.ContainsKey(a.Value) || Items[item].BaseAttrs.TryGet(a, out _);

    public double Base(int item, AttrId a)
    {
        var it = Items[item];
        if (it.Nodes.TryGetValue(a.Value, out var n)) return n.Base;
        return it.BaseAttrs.TryGet(a, out var b) ? b : Ds.AttrDefault(a);
    }

    public double Get(int item, AttrId a)
    {
        var it = Items[item];
        if (!it.Nodes.TryGetValue(a.Value, out var n))
        {
            if (!it.BaseAttrs.TryGet(a, out var b)) return Ds.AttrDefault(a);
            n = new AttrNode(b);
            it.Nodes[a.Value] = n;
        }
        return Eval(item, a, n);
    }

    public double? GetOpt(int item, AttrId a) => Has(item, a) ? Get(item, a) : null;

    /// <summary>Set (or replace) an attribute's base value, e.g. skill level, overrides.</summary>
    internal void SetBase(int item, AttrId a, double v) => Items[item].Nodes[a.Value] = new AttrNode(v);

    internal AttrNode NodeFor(int item, AttrId a)
    {
        var it = Items[item];
        if (!it.Nodes.TryGetValue(a.Value, out var n))
        {
            n = new AttrNode(it.BaseAttrs.TryGet(a, out var b) ? b : Ds.AttrDefault(a));
            it.Nodes[a.Value] = n;
        }
        return n;
    }

    /// <summary>
    /// Attach a modifier to (target, attr). Stacking penalty applies when the target attribute is non-stackable
    /// and the source category is not exempt (ship, charge, skill, implant, subsystem, structure).
    /// </summary>
    public void AddModifier(int target, AttrId attr, Op op, ModSource src, int sourceItem, int sourceCategory)
    {
        bool stackable = Ds.Attr(attr)?.Stackable ?? true;
        bool penalized = !stackable && Array.IndexOf(StackingExemptCategories, sourceCategory) < 0;
        var n = NodeFor(target, attr);
        (n.Mods ??= new List<AppliedModifier>(4)).Add(new AppliedModifier(op, penalized, src, sourceItem));
    }

    // ------------------------------------------------------------------ evaluation
    private double SourceValue(in ModSource s)
    {
        switch (s.Kind)
        {
            case SourceKind.Attribute: return Get(s.Item, s.Attr);
            case SourceKind.Constant: return s.Number;
            case SourceKind.Propulsion:
            {
                double m = Get(s.Target, KnownIds.Mass);
                return m == 0.0 ? 1.0 : 1.0 + Get(s.Item, s.Attr) / 100.0 * Get(s.Item, s.Attr2) / m;
            }
            default:
            {
                double f = s.Number;
                if (!s.Attr2.IsNone) f *= Get(s.Target, s.Attr2);
                double v = Get(s.Item, s.Attr);
                return s.Multiplicative ? (v - 1.0) * f + 1.0 : v * f;
            }
        }
    }

    private static readonly double[] PenaltyFactor = Enumerable.Range(0, 64).Select(i => Math.Exp(-(double)(i * i) / 7.1289)).ToArray();
    private static readonly Op[] OpOrder =
        { Op.PreAssign, Op.PreMul, Op.PreDiv, Op.ModAdd, Op.ModSub, Op.PostMul, Op.PostDiv, Op.PostPercent, Op.PostAssign };

    private double Eval(int item, AttrId attrId, AttrNode a)
    {
        if (a.Generation == _generation) return a.Value;
        if (a.Busy) return a.Base; // cycle guard
        a.Busy = true;
        var info = Ds.Attr(attrId);
        double val = a.Base;
        var mods = a.Mods;
        if (mods is { Count: > 0 })
        {
            int n = mods.Count;
            Span<double> vals = n <= 128 ? stackalloc double[n] : new double[n];
            for (int i = 0; i < n; i++) vals[i] = SourceValue(mods[i].Source);
            Span<double> pos = n <= 128 ? stackalloc double[n] : new double[n];
            Span<double> neg = n <= 128 ? stackalloc double[n] : new double[n];
            bool highIsGood = info?.HighIsGood ?? true;
            foreach (var op in OpOrder)
            {
                bool any = false; int np = 0, nn = 0;
                bool hasAssign = false; double assign = 0;
                for (int i = 0; i < n; i++)
                {
                    var m = mods[i];
                    if (m.Op != op) continue;
                    any = true;
                    double v = vals[i];
                    switch (op)
                    {
                        case Op.PreAssign:
                        case Op.PostAssign:
                            assign = !hasAssign ? v : highIsGood ? Math.Max(assign, v) : Math.Min(assign, v);
                            hasAssign = true;
                            break;
                        case Op.ModAdd: val += v; break;
                        case Op.ModSub: val -= v; break;
                        default:
                            double mult = op switch
                            {
                                Op.PreMul or Op.PostMul => v,
                                Op.PreDiv or Op.PostDiv => v == 0.0 ? 1.0 : 1.0 / v,
                                Op.PostPercent => 1.0 + v / 100.0,
                                _ => 1.0,
                            };
                            if (m.Penalized)
                            {
                                if (mult > 1.0) pos[np++] = mult;
                                else if (mult < 1.0) neg[nn++] = mult;
                            }
                            else val *= mult;
                            break;
                    }
                }
                if (!any) continue;
                if (hasAssign) val = assign;
                val = ApplyPenalized(val, pos[..np]);
                val = ApplyPenalized(val, neg[..nn]);
            }
        }
        if (info != null)
        {
            if (info.MinAttr is { } mn) val = Math.Max(val, Get(item, mn));
            if (info.MaxAttr is { } mx) val = Math.Min(val, Get(item, mx));
            if (info.RoundToCentis) val = EveDogmaK.Stats.Formulas.PyRound2(val);
        }
        a.Busy = false;
        a.Value = val;
        a.Generation = _generation;
        return val;
    }

    /// <summary>Stacking penalty: strongest first, i-th multiplier weighted by exp(-(i/2.67)^2).</summary>
    private static double ApplyPenalized(double val, Span<double> list)
    {
        // stable insertion sort, descending by |m-1| (same order as a stable sort in the reference)
        for (int i = 1; i < list.Length; i++)
        {
            double x = list[i]; double kx = Math.Abs(x - 1.0); int j = i - 1;
            while (j >= 0 && Math.Abs(list[j] - 1.0) < kx) { list[j + 1] = list[j]; j--; }
            list[j + 1] = x;
        }
        for (int i = 0; i < list.Length; i++)
            val *= 1.0 + (list[i] - 1.0) * (i < PenaltyFactor.Length ? PenaltyFactor[i] : 0.0);
        return val;
    }

    /// <summary>Effective state for effect activation (charges follow their module, drones their active count).</summary>
    public ModuleState EffectiveState(int i)
    {
        var it = Items[i];
        return it.Kind switch
        {
            ItemKind.Charge => it.Parent >= 0 ? Items[it.Parent].State : ModuleState.Online,
            ItemKind.Ship or ItemKind.Character or ItemKind.Skill or ItemKind.Implant or ItemKind.Booster or ItemKind.Mode or ItemKind.Beacon
                => ModuleState.Online,
            ItemKind.Drone or ItemKind.Fighter => it.ActiveCount > 0 ? ModuleState.Active : ModuleState.Offline,
            _ => it.State,
        };
    }

    public bool HasEffect(int i, EffectId e) => !e.IsNone && Items[i].HasEffect(e);
}

public sealed partial class Fit
{
    private TargetIndex? _targetIndex;
    /// <summary>Location/group/skill index over the (complete) item list; built on first use.</summary>
    public TargetIndex Targets => _targetIndex ??= new TargetIndex(this);
}
