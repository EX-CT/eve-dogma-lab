using System.Globalization;
using EveDogmaK.Data;
using EveDogmaK.Engine;
using EveDogmaK.Json;
using EveDogmaK.Requests;

namespace EveDogmaK.Stats;

public sealed partial class StatsCalculator
{
    private static string F(double v) => v.ToString("R", CultureInfo.InvariantCulture);
    private static string F2(double v) => v.ToString("F2", CultureInfo.InvariantCulture);

    /// <summary>Fitting rules (contract violation codes). Fitting problems are reported, never thrown.</summary>
    private JArr Validate(double cpu, double pg, double calib, double bw)
    {
        int ship = _f.Ship;
        var v = new JArr();
        void Push(string code, string msg, int? idx) =>
            v.Add(new JObj { { "code", code }, { "message", msg }, { "module_index", JNode.Of(idx) } });

        if (cpu > G(ship, _k.CpuOutput) + 1e-9) Push("CPU_OVERLOAD", $"CPU used {F2(cpu)} > output {F2(G(ship, _k.CpuOutput))}", null);
        if (pg > G(ship, _k.PowerOutput) + 1e-9) Push("POWER_OVERLOAD", $"Powergrid used {F2(pg)} > output {F2(G(ship, _k.PowerOutput))}", null);
        if (calib > G(ship, _k.UpgradeCapacity) + 1e-9) Push("CALIBRATION_OVERLOAD", $"Calibration used {F(calib)} > {F(G(ship, _k.UpgradeCapacity))}", null);
        if (bw > G(ship, _k.DroneBandwidth) + 1e-9) Push("DRONE_BANDWIDTH", $"Drone bandwidth used {F(bw)} > {F(G(ship, _k.DroneBandwidth))}", null);

        foreach (var (slot, attr) in new[] { (Slot.High, _k.HiSlots), (Slot.Mid, _k.MedSlots), (Slot.Low, _k.LowSlots), (Slot.Rig, _k.RigSlots), (Slot.Subsystem, _k.MaxSubSystems), (Slot.Service, _k.ServiceSlots) })
        {
            double used = _modules.Count(i => _f[i].Slot == slot);
            if (used > G(ship, attr)) Push("SLOTS_EXCEEDED", $"{slot} slots used {F(used)} > {F(G(ship, attr))}", null);
        }
        double t = _modules.Count(i => _f.HasEffect(i, _k.TurretFitted));
        if (t > G(ship, _k.TurretSlotsLeft)) Push("TURRET_HARDPOINTS", $"turrets {F(t)} > hardpoints {F(G(ship, _k.TurretSlotsLeft))}", null);
        double l = _modules.Count(i => _f.HasEffect(i, _k.LauncherFitted));
        if (l > G(ship, _k.LauncherSlotsLeft)) Push("LAUNCHER_HARDPOINTS", $"launchers {F(l)} > hardpoints {F(G(ship, _k.LauncherSlotsLeft))}", null);

        var shipT = _f[ship].Type;
        var fittedGroup = new Dictionary<int, int>(); var fittedType = new Dictionary<int, int>();
        var activeGroup = new Dictionary<int, int>(); var onlineGroup = new Dictionary<int, int>();
        static void Inc(Dictionary<int, int> d, int k) => d[k] = d.GetValueOrDefault(k) + 1;
        foreach (var i in _modules)
        {
            var it = _f[i];
            int? idx = it.RequestIndex;
            var mt = it.Type;
            string name = mt.Name;
            if (it.Slot == null) Push("NOT_FITTABLE", $"{name} is not a fittable module", idx);
            var gr = _k.CanFitShipGroups.Select(a => mt.RawAttrs.Get(a)).Where(x => x != null).Select(x => (int)x!.Value).Where(x => x != 0).ToList();
            var ty = _k.CanFitShipTypes.Select(a => mt.RawAttrs.Get(a)).Where(x => x != null).Select(x => (int)x!.Value).Where(x => x != 0).ToList();
            if ((gr.Count > 0 || ty.Count > 0) && !gr.Contains(shipT.Group) && !ty.Contains(shipT.Id))
                Push("SHIP_RESTRICTION", $"{name} cannot be fitted to {shipT.Name}", idx);
            if (it.Slot == Slot.Rig)
            {
                double rs = mt.RawAttrs.Get(_k.RigSize) ?? 0.0, srs = G(ship, _k.RigSize);
                if (rs != 0.0 && rs != srs) Push("RIG_SIZE", $"{name} rig size {F(rs)} != ship rig size {F(srs)}", idx);
            }
            Inc(fittedGroup, it.Group); Inc(fittedType, it.TypeId);
            if (it.State >= ModuleState.Online) Inc(onlineGroup, it.Group);
            if (it.State >= ModuleState.Active) Inc(activeGroup, it.Group);
            (double Lim, int N)? Check(AttrId a, Dictionary<int, int> map, int key)
            {
                if (mt.RawAttrs.Get(a) is not double lim) return null;
                int n = map.GetValueOrDefault(key);
                return lim > 0.0 && n > lim ? (lim, n) : null;
            }
            if (Check(_k.MaxGroupFitted, fittedGroup, it.Group) is var (l1, n1)) Push("MAX_GROUP_FITTED", $"{name}: {n1} fitted of group, max {F(l1)}", idx);
            if (Check(_k.MaxTypeFitted, fittedType, it.TypeId) is var (l2, n2)) Push("MAX_TYPE_FITTED", $"{name}: {n2} fitted, max {F(l2)}", idx);
            if (Check(_k.MaxGroupOnline, onlineGroup, it.Group) is var (l3, n3)) Push("MAX_GROUP_ONLINE", $"{name}: {n3} online of group, max {F(l3)}", idx);
            if (Check(_k.MaxGroupActive, activeGroup, it.Group) is var (l4, n4)) Push("MAX_GROUP_ACTIVE", $"{name}: {n4} active of group, max {F(l4)}", idx);
            if (it.Charge >= 0)
            {
                var ct = _f[it.Charge].Type;
                var cg = _k.ChargeGroups.Select(a => mt.RawAttrs.Get(a)).Where(x => x != null).Select(x => (int)x!.Value).Where(x => x != 0).ToList();
                if (!cg.Contains(ct.Group)) Push("CHARGE_GROUP", $"{ct.Name} cannot be loaded into {name}", idx);
                if (mt.RawAttrs.Get(_k.ChargeSize) is double a && ct.RawAttrs.Get(_k.ChargeSize) is double b && a != b)
                    Push("CHARGE_SIZE", $"{ct.Name} size {F(b)} != launcher size {F(a)}", idx);
                if (ct.Volume > mt.Capacity && mt.Capacity > 0.0) Push("CHARGE_CAPACITY", $"{ct.Name} does not fit into {name}", idx);
            }
        }
        // skills
        var have = new Dictionary<int, double>();
        foreach (var it in _f.Items)
            if (it.Kind == ItemKind.Skill) have[it.TypeId] = _f.Base(it.Index, KnownIds.SkillLevel);
        var missing = new List<(int Skill, double Need, int By)>();
        foreach (var it in _f.Items)
        {
            if (it.Kind is not (ItemKind.Ship or ItemKind.Module or ItemKind.Charge or ItemKind.Drone or ItemKind.Fighter or ItemKind.Implant or ItemKind.Booster)) continue;
            var ty = it.Type;
            for (int x = 0; x < 6; x++)
            {
                int s = (int)(ty.RawAttrs.Get(_k.RequiredSkill[x]) ?? 0.0);
                if (s == 0) continue;
                double need = ty.RawAttrs.Get(_k.RequiredSkillLevel[x]) ?? 1.0;
                if (have.GetValueOrDefault(s) < need && !missing.Any(m => m.Skill == s && m.Need >= need)) missing.Add((s, need, it.TypeId));
            }
        }
        foreach (var (s, need, by) in missing)
            Push("MISSING_SKILL", $"{_ds.Type(s)?.Name ?? "?"} {F(need)} required by {_ds.Type(by)!.Name}", null);
        return v;
    }
}
