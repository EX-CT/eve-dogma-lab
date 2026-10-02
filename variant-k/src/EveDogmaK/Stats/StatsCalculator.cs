using EveDogmaK.Data;
using EveDogmaK.Engine;
using EveDogmaK.Json;
using EveDogmaK.Requests;

namespace EveDogmaK.Stats;

/// <summary>Fit statistics on top of the evaluated dogma graph (Pyfa-equivalent formulas).</summary>
public sealed partial class StatsCalculator
{
    public const string EngineName = "eve-dogma-k 0.1.0";

    private readonly Fit _f;
    private readonly KnownIds _k;
    private readonly Dataset _ds;
    private readonly FitRequest _req;
    private readonly List<int> _modules, _drones, _fighters;

    public StatsCalculator(Fit fit)
    {
        _f = fit; _k = fit.K; _ds = fit.Ds; _req = fit.Request;
        _modules = fit.Items.Where(i => i.Kind == ItemKind.Module).Select(i => i.Index).ToList();
        _drones = fit.Items.Where(i => i.Kind == ItemKind.Drone).Select(i => i.Index).ToList();
        _fighters = fit.Items.Where(i => i.Kind == ItemKind.Fighter).Select(i => i.Index).ToList();
    }

    private double G(int i, AttrId a) => _f.Get(i, a);
    private bool Online(int i) => _f[i].State >= ModuleState.Online;
    private bool Active(int i) => _f[i].State >= ModuleState.Active;
    private string TypeName(int i) => _f[i].Type.Name;

    // ---------------------------------------------------------------- module timing
    private double RawCycleMs(int i)
    {
        double v = Math.Max(G(i, _k.Speed), G(i, _k.Duration));
        foreach (var a in _k.ExtraCycleAttrs) v = Math.Max(v, G(i, a));
        return v;
    }

    private int NumCharges(int i)
    {
        int c = _f[i].Charge;
        if (c < 0) return 0;
        double vol = G(c, KnownIds.Volume), cap = _f.Base(i, KnownIds.Capacity);
        return vol <= 0.0 ? 0 : (int)Math.Floor(Formulas.FloatUnerr(cap / vol));
    }

    private int NumShots(int i)
    {
        int c = _f[i].Charge;
        if (c < 0) return 0;
        int n = NumCharges(i);
        if (n > 0 && _f.Has(i, _k.ChargeRate))
        {
            double r = G(i, _k.ChargeRate);
            return r > 0.0 ? (int)Math.Floor(n / r) : 0;
        }
        if (n > 0 && _f.Has(c, _k.CrystalsGetDamaged))
        {
            if (G(c, _k.CrystalsGetDamaged) == 1.0)
            {
                double hp = G(c, KnownIds.Hp), chance = G(c, _k.CrystalVolatilityChance), dmg = G(c, _k.CrystalVolatilityDamage);
                if (dmg * chance > 0.0) return (int)Math.Floor(n * hp / (dmg * chance));
            }
            return 0;
        }
        return 0;
    }

    /// <summary>Average cycle time in ms (Pyfa getCycleParameters().averageTime), optionally including reloads.</summary>
    private double AvgCycleMs(int i, bool factorReload)
    {
        double active = RawCycleMs(i);
        if (active == 0.0) return 0.0;
        double inactive = G(i, _k.ReactivationDelay);
        int shots = NumShots(i);
        double reload = G(i, _k.ReloadTime);
        if (!factorReload || shots == 0 || inactive >= reload) return active + inactive;
        double early = shots - 1.0;
        return ((active + inactive) * early + (active + reload)) / shots;
    }

    private string WeaponKind(int i) =>
        _f.HasEffect(i, _k.TurretFitted) ? "turret" :
        _f.HasEffect(i, _k.LauncherFitted) ? "missile" :
        _f.HasEffect(i, _k.EmpWave) ? "smartbomb" :
        _f.HasEffect(i, _k.ChainLightning) ? "vorton" : "other";

    private (Damage Volley, string Kind) ModuleVolley(int i)
    {
        var it = _f[i];
        string kind = WeaponKind(i);
        int src = it.Charge >= 0 ? it.Charge : i;
        double mult = _f.Has(i, _k.DamageMultiplier) ? G(i, _k.DamageMultiplier) : 1.0;
        // missile damage is scaled by the pilot's missileDamageMultiplier (BCS etc. modify the character)
        if (kind == "missile" && it.Charge >= 0) mult *= G(_f.Char, _k.MissileDamageMultiplier);
        var d = _k.Damage;
        return (new Damage(G(src, d[0]) * mult, G(src, d[1]) * mult, G(src, d[2]) * mult, G(src, d[3]) * mult), kind);
    }

    /// <summary>
    /// Pyfa missileMaxRangeData (eos/saveddata/module.py, LGPL; re-implemented from the reference engine): flight time plus
    /// ship-radius bonus, acceleration phase, floor/ceil blend of whole seconds, FoF range limit, centre-to-surface.
    /// </summary>
    private double? MissileRange(int charge)
    {
        double vel = G(charge, _k.MaxVelocity);
        if (vel <= 0.0) return null;
        double radius = G(_f.Ship, _k.ShipRadius);
        double ft = Formulas.FloatUnerr(G(charge, _k.ExplosionDelay) / 1000.0 + radius / vel);
        double accelCap = G(charge, KnownIds.Mass) * G(charge, _k.Agility) / 1e6;
        double RangeAt(double t) { double acc = Math.Min(t, accelCap); return vel / 2.0 * acc + vel * (t - acc); }
        double lt = Math.Floor(ft), ht = Math.Ceiling(ft);
        double lr = RangeAt(lt), hr = RangeAt(ht);
        if (_f.HasEffect(charge, _k.FofMissileLaunching))
        {
            double lim = G(charge, _k.MaxFofTargetRange);
            if (lim > 0.0) { lr = Math.Min(lr, lim); hr = Math.Min(hr, lim); }
        }
        lr = Math.Max(lr - radius, 0.0);
        hr = Math.Max(hr - radius, 0.0);
        double hc = ft - lt;
        return lr * (1.0 - hc) + hr * hc;
    }

    /// <summary>Incoming remote repairs, Pyfa __getAppliedRr diminishing-returns formula: HP/s per layer.</summary>
    private double[] AppliedRemoteRepairs()
    {
        var lists = new[] { new List<(double A, double C)>(), new List<(double, double)>(), new List<(double, double)>() };
        foreach (var e in _f.Incoming)
            if (e is IncomingRepair r)
            {
                double dur = G(r.Item, _k.Duration) / 1000.0;
                if (dur > 0.0) lists[r.Layer].Add((G(r.Item, r.Amount) * r.Mult * r.Factor, dur));
            }
        var outp = new double[3];
        for (int l = 0; l < 3; l++)
        {
            double total = 0;
            foreach (var (a, c) in lists[l]) total += a / Math.Truncate(c);
            double sum = 0;
            foreach (var (a, c) in lists[l])
            {
                double rrps = a / Math.Truncate(c);
                double m = 7000.0 + rrps * 20.0;
                double q = (rrps + m) / (total + m) - 1.0;
                sum += (1.0 - q * q) * a / c;
            }
            outp[l] = sum;
        }
        return outp;
    }

    // ---------------------------------------------------------------- top level
    public JObj Compute()
    {
        var res = Resources(out double cpu, out double pg, out double calib, out double bw);
        var ship = _f.Ship;
        var st = _f[ship].Type;
        var outp = new JObj
        {
            { "meta", new JObj { { "schema_version", 1 }, { "engine", EngineName }, { "sde_build", _ds.Build }, { "dataset_sha256", _ds.Sha256 } } },
            { "ship", new JObj { { "type_id", st.Id }, { "name", st.Name }, { "group", _ds.Groups.TryGetValue(st.Group, out var g) ? g.Name : null } } },
            { "resources", res },
            { "offense", Offense() },
            { "defense", Defense() },
            { "capacitor", Capacitor(out var moduleRows) },
            { "navigation", Navigation() },
            { "targeting", Targeting() },
            { "drones", new JObj
                {
                    { "active", _drones.Sum(i => (long)_f[i].ActiveCount) },
                    { "max_active", G(_f.Char, _k.MaxActiveDrones) },
                    { "control_range_m", G(_f.Char, _k.DroneControlDistance) },
                } },
            { "modules", moduleRows },
        };
        if (_req.Options.Validate) outp["violations"] = Validate(cpu, pg, calib, bw);
        if (_f.Warnings.Count > 0) outp["warnings"] = new JArr(_f.Warnings.Select(w => (JNode)w));
        switch (_req.Options.IncludeAttributes)
        {
            case "ship":
                outp["attributes"] = new JObj { { "ship", DumpAttrs(ship) } };
                break;
            case "all":
                outp["attributes"] = new JObj
                {
                    { "ship", DumpAttrs(ship) }, { "character", DumpAttrs(_f.Char) },
                    { "modules", new JArr(_modules.Select(i => (JNode)new JObj
                        {
                            { "module_index", JNode.Of(_f[i].RequestIndex) }, { "type_id", _f[i].TypeId }, { "attributes", DumpAttrs(i) },
                            { "charge", _f[i].Charge >= 0 ? DumpAttrs(_f[i].Charge) : JNode.Null },
                        })) },
                    { "drones", new JArr(_drones.Select(i => (JNode)new JObj { { "drone_index", JNode.Of(_f[i].RequestIndex) }, { "attributes", DumpAttrs(i) } })) },
                };
                break;
        }
        return outp;
    }

    public JObj DumpAttrs(int i)
    {
        var it = _f[i];
        var keys = new SortedSet<int>(it.Nodes.Keys);
        foreach (var (a, _) in it.BaseAttrs.Entries()) keys.Add(a.Value);
        var o = new JObj();
        foreach (var k in keys) o[_ds.AttrName(new AttrId(k))] = G(i, new AttrId(k));
        return o;
    }

    private static JObj Usage(double used, double total) => new() { { "used", used }, { "total", total } };

    // ---------------------------------------------------------------- resources
    private JObj Resources(out double cpuUsed, out double pgUsed, out double calibUsed, out double bwUsed)
    {
        int ship = _f.Ship;
        double Sum(AttrId a, Func<int, bool> pred) { double s = 0; foreach (var i in _modules) if (pred(i)) s += G(i, a); return s; }
        cpuUsed = Sum(_k.Cpu, Online);
        pgUsed = Sum(_k.Power, Online);
        calibUsed = Sum(_k.UpgradeCost, i => _f[i].Slot == Slot.Rig);
        double bw = 0, bay = 0, fbay = 0, cargo = 0;
        foreach (var i in _drones) bw += G(i, _k.DroneBandwidthUsed) * _f[i].ActiveCount;
        foreach (var i in _drones) bay += G(i, KnownIds.Volume) * _f[i].Quantity;
        foreach (var i in _fighters) fbay += G(i, KnownIds.Volume) * _f[i].Quantity;
        foreach (var c in _req.Cargo) cargo += (_ds.Type(c.TypeId)?.Volume ?? 0.0) * c.Quantity;
        bwUsed = bw;
        double Count(Slot s) => _modules.Count(i => _f[i].Slot == s);
        string FighterClass(int i) => G(i, _k.FighterSquadronIsHeavy) > 0.0 ? "heavy" : G(i, _k.FighterSquadronIsSupport) > 0.0 ? "support" : "light";
        double ClassUsed(string c) => _fighters.Count(i => _f[i].ActiveCount > 0 && FighterClass(i) == c);
        return new JObj
        {
            { "cpu", Usage(cpuUsed, G(ship, _k.CpuOutput)) },
            { "power", Usage(pgUsed, G(ship, _k.PowerOutput)) },
            { "calibration", Usage(calibUsed, G(ship, _k.UpgradeCapacity)) },
            { "drone_bandwidth", Usage(bw, G(ship, _k.DroneBandwidth)) },
            { "drone_bay", Usage(bay, G(ship, _k.DroneCapacity)) },
            { "fighter_bay", Usage(fbay, G(ship, _k.FighterCapacity)) },
            { "cargo", Usage(cargo, G(ship, KnownIds.Capacity)) },
            { "slots", new JObj
                {
                    { "high", Usage(Count(Slot.High), G(ship, _k.HiSlots)) },
                    { "mid", Usage(Count(Slot.Mid), G(ship, _k.MedSlots)) },
                    { "low", Usage(Count(Slot.Low), G(ship, _k.LowSlots)) },
                    { "rig", Usage(Count(Slot.Rig), G(ship, _k.RigSlots)) },
                    { "subsystem", Usage(Count(Slot.Subsystem), G(ship, _k.MaxSubSystems)) },
                    { "service", Usage(Count(Slot.Service), G(ship, _k.ServiceSlots)) },
                } },
            { "hardpoints", new JObj
                {
                    { "turret", Usage(_modules.Count(i => _f.HasEffect(i, _k.TurretFitted)), G(ship, _k.TurretSlotsLeft)) },
                    { "launcher", Usage(_modules.Count(i => _f.HasEffect(i, _k.LauncherFitted)), G(ship, _k.LauncherSlotsLeft)) },
                } },
            { "fighter_tubes", new JObj
                {
                    { "total", Usage(_fighters.Count(i => _f[i].ActiveCount > 0), G(ship, _k.FighterTubes)) },
                    { "light", Usage(ClassUsed("light"), G(ship, _k.FighterLightSlots)) },
                    { "support", Usage(ClassUsed("support"), G(ship, _k.FighterSupportSlots)) },
                    { "heavy", Usage(ClassUsed("heavy"), G(ship, _k.FighterHeavySlots)) },
                } },
        };
    }

    // ---------------------------------------------------------------- offense
    private JObj Offense()
    {
        var tp = _req.TargetProfile ?? TargetProfile.Default;
        var defaultSpool = _req.Options.DefaultSpool ?? new Spool(SpoolType.SpoolScale, 1.0);
        bool factorReload = _req.Options.FactorReload;
        var weapons = new JArr();
        Damage wVol = Damage.Zero, wDps = Damage.Zero;
        foreach (var i in _modules)
        {
            if (!Active(i)) continue;
            var (baseVol, kind) = ModuleVolley(i);
            if (baseVol.Total == 0.0) continue;
            double cyc = AvgCycleMs(i, factorReload);
            double raw = RawCycleMs(i);
            var spool = _f[i].Spool ?? defaultSpool;
            var (sp, _, _) = Formulas.Spoolup(G(i, _k.DamageMultiplierBonusMax), G(i, _k.DamageMultiplierBonusPerCycle), raw / 1000.0, spool);
            var vol = baseVol.Scale(1.0 + sp);
            var dps = cyc > 0.0 ? vol.Scale(1000.0 / cyc) : Damage.Zero;
            wVol += vol; // Pyfa reports spooled volley
            wDps += dps;
            var it = _f[i];
            var w = new JObj
            {
                { "module_index", JNode.Of(it.RequestIndex) }, { "type_id", it.TypeId }, { "name", TypeName(i) }, { "kind", kind },
                { "charge_type_id", it.Charge >= 0 ? _f[it.Charge].TypeId : JNode.Null },
                { "volley", vol.ToJson() }, { "dps", dps.ToJson() }, { "cycle_time_ms", cyc },
            };
            if (kind == "turret")
            {
                w["optimal_m"] = G(i, _k.MaxRange); w["falloff_m"] = G(i, _k.Falloff); w["tracking"] = G(i, _k.TrackingSpeed);
            }
            else if (kind == "missile" && it.Charge >= 0)
            {
                int c = it.Charge;
                if (MissileRange(c) is double range) w["range_m"] = range;
                w["explosion_radius"] = G(c, _k.AoeCloudSize);
                w["explosion_velocity"] = G(c, _k.AoeVelocity);
            }
            else if (kind == "smartbomb") w["range_m"] = G(i, _k.EmpFieldRange);
            if (sp > 0.0) { w["spool_multiplier"] = 1.0 + sp; w["volley_unspooled"] = baseVol.ToJson(); }
            weapons.Add(w);
        }
        Damage dVol = Damage.Zero, dDps = Damage.Zero;
        var droneOut = new JArr();
        foreach (var i in _drones)
        {
            double n = _f[i].ActiveCount;
            if (n == 0.0) continue;
            double mult = _f.Has(i, _k.DamageMultiplier) ? G(i, _k.DamageMultiplier) : 1.0;
            var d = _k.Damage;
            var v = new Damage(G(i, d[0]), G(i, d[1]), G(i, d[2]), G(i, d[3])).Scale(mult * n);
            double cyc = RawCycleMs(i);
            if (v.Total == 0.0 || cyc == 0.0) continue;
            var dps = v.Scale(1000.0 / cyc);
            dVol += v; dDps += dps;
            droneOut.Add(new JObj { { "drone_index", JNode.Of(_f[i].RequestIndex) }, { "type_id", _f[i].TypeId }, { "name", TypeName(i) }, { "count", n }, { "volley", v.ToJson() }, { "dps", dps.ToJson() } });
        }
        Damage fVol = Damage.Zero, fDps = Damage.Zero;
        var fighterOut = new JArr();
        foreach (var i in _fighters)
        {
            double n = _f[i].ActiveCount;
            if (n == 0.0) continue;
            Damage fv = Damage.Zero, fd = Damage.Zero;
            foreach (var (eff, prefix) in new[] { (_k.FighterAttackM, "fighterAbilityAttackMissile"), (_k.FighterMissiles, "fighterAbilityMissiles") })
            {
                int idx = _f[i].Effects.FindIndex(e => e.Id == eff);
                if (eff.IsNone || idx < 0) continue;
                bool used = _f[i].FighterAbilities is { } l ? Array.IndexOf(l, eff.Value) >= 0 : _f[i].Effects[idx].IsDefault;
                if (!used) continue;
                AttrId A(string s) => _ds.AttrIdOf(prefix + s);
                double m = G(i, A("DamageMultiplier"));
                if (m == 0.0) m = 1.0;
                var v = new Damage(G(i, A("DamageEM")), G(i, A("DamageTherm")), G(i, A("DamageKin")), G(i, A("DamageExp"))).Scale(m * n);
                double dur = G(i, A("Duration"));
                fv += v;
                if (dur > 0.0) fd += v.Scale(1000.0 / dur);
            }
            if (fv.Total > 0.0)
            {
                fVol += fv; fDps += fd;
                fighterOut.Add(new JObj { { "fighter_index", JNode.Of(_f[i].RequestIndex) }, { "type_id", _f[i].TypeId }, { "name", TypeName(i) }, { "squadron_size", n }, { "volley", fv.ToJson() }, { "dps", fd.ToJson() } });
            }
        }
        var tVol = wVol + dVol + fVol;
        var tDps = wDps + dDps + fDps;
        return new JObj
        {
            { "weapons", weapons }, { "drones", droneOut }, { "fighters", fighterOut },
            { "total", new JObj
                {
                    { "weapon_dps", wDps.Total }, { "weapon_volley", wVol.Total }, { "drone_dps", dDps.Total }, { "drone_volley", dVol.Total },
                    { "fighter_dps", fDps.Total }, { "fighter_volley", fVol.Total }, { "dps", tDps.ToJson() }, { "volley", tVol.ToJson() },
                } },
            { "vs_target_profile", new JObj { { "dps", tDps.Versus(tp.Resists) }, { "volley", tVol.Versus(tp.Resists) } } },
        };
    }

    // ---------------------------------------------------------------- defense
    private JObj Defense()
    {
        int ship = _f.Ship;
        var dp = _req.DamagePattern ?? DamageProfile.Uniform;
        double dpTot = Math.Max(dp.Total, 1e-12);
        double[] Layer(AttrId[] a) => new[] { G(ship, a[0]), G(ship, a[1]), G(ship, a[2]), G(ship, a[3]) };
        double Effective(double amount, double[] r)
        {
            double div = (dp.Em * r[0] + dp.Thermal * r[1] + dp.Kinetic * r[2] + dp.Explosive * r[3]) / dpTot;
            return div == 0.0 ? amount : amount / div;
        }
        var rs = Layer(_k.ShieldResonance); var ra = Layer(_k.ArmorResonance); var rh = Layer(_k.HullResonance);
        double hpS = G(ship, _k.ShieldCapacity), hpA = G(ship, _k.ArmorHp), hpH = G(ship, KnownIds.Hp);
        double eS = Effective(hpS, rs), eA = Effective(hpA, ra), eH = Effective(hpH, rh);
        static JObj ResJson(double[] r) => new() { { "em", r[0] }, { "thermal", r[1] }, { "kinetic", r[2] }, { "explosive", r[3] } };
        double shieldRep = 0, armorRep = 0, hullRep = 0;
        foreach (var i in _modules)
        {
            if (!Active(i)) continue;
            double dur = G(i, _k.Duration) / 1000.0;
            if (dur <= 0.0) continue;
            if (_f.HasEffect(i, _k.ShieldBoosting) || _f.HasEffect(i, _k.FueledShieldBoosting)) shieldRep += G(i, _k.ShieldBonus) / dur;
            if (_f.HasEffect(i, _k.ArmorRepair)) armorRep += G(i, _k.ArmorDamageAmount) / dur;
            if (_f.HasEffect(i, _k.FueledArmorRepair))
            {
                // ancillary armor repairer: x3 with Nanite Repair Paste loaded
                bool paste = _f[i].Charge >= 0 && _f[_f[i].Charge].Type.Name == "Nanite Repair Paste";
                armorRep += G(i, _k.ArmorDamageAmount) * (paste ? 3.0 : 1.0) / dur;
            }
            if (_f.HasEffect(i, _k.StructureRepair)) hullRep += G(i, _k.StructureDamageAmount) / dur;
        }
        var rr = AppliedRemoteRepairs();
        shieldRep += rr[0]; armorRep += rr[1]; hullRep += rr[2];
        double passive = Formulas.PeakRecharge(hpS, G(ship, _k.ShieldRechargeRate));
        return new JObj
        {
            { "hp", new JObj { { "shield", hpS }, { "armor", hpA }, { "hull", hpH }, { "total", hpS + hpA + hpH } } },
            { "resonance", new JObj { { "shield", ResJson(rs) }, { "armor", ResJson(ra) }, { "hull", ResJson(rh) } } },
            { "ehp", new JObj { { "shield", eS }, { "armor", eA }, { "hull", eH }, { "total", eS + eA + eH } } },
            { "damage_pattern", new JObj { { "em", dp.Em }, { "thermal", dp.Thermal }, { "kinetic", dp.Kinetic }, { "explosive", dp.Explosive } } },
            { "tank", new JObj
                {
                    { "raw", new JObj { { "passive_shield", passive }, { "shield_repair", shieldRep }, { "armor_repair", armorRep }, { "hull_repair", hullRep } } },
                    { "effective", new JObj
                        {
                            { "passive_shield", Effective(passive, rs) }, { "shield_repair", Effective(shieldRep, rs) },
                            { "armor_repair", Effective(armorRep, ra) }, { "hull_repair", Effective(hullRep, rh) },
                        } },
                } },
        };
    }

    // ---------------------------------------------------------------- capacitor
    private JObj Capacitor(out JArr moduleRows)
    {
        int ship = _f.Ship;
        bool factorReload = _req.Options.FactorReload;
        double cap = G(ship, _k.CapacitorCapacity);
        double rr = G(ship, _k.RechargeRate);
        double peak = Formulas.PeakRecharge(cap, rr);
        var drains = new List<CapDrain>();
        double used = 0, added = 0;
        moduleRows = new JArr();
        foreach (var i in _modules)
        {
            var it = _f[i];
            double capNeed = G(i, _k.CapacitorNeed);
            bool isInjector = _ds.Groups.TryGetValue(it.Group, out var grp) && grp.Name == "Capacitor Booster";
            if (isInjector) capNeed = -(it.Charge >= 0 ? G(it.Charge, _k.CapacitorBonus) : 0.0);
            // local nosferatu counts as cap income (assumes the target has cap), like Pyfa
            if (_f.HasEffect(i, _k.EnergyNosferatuFalloff) && !_req.Options.NosNoTargetCap) capNeed = -G(i, _k.PowerTransferAmount);
            double cycRaw = RawCycleMs(i);
            double full = cycRaw + G(i, _k.ReactivationDelay);
            var row = new JObj
            {
                { "module_index", JNode.Of(it.RequestIndex) }, { "type_id", it.TypeId }, { "name", TypeName(i) },
                { "slot", it.Slot is { } s ? SlotName(s) : null }, { "state", StateName(it.State) },
                { "cpu", G(i, _k.Cpu) }, { "power", G(i, _k.Power) },
            };
            if (cycRaw > 0.0) row["cycle_time_ms"] = cycRaw;
            if (Active(i) && capNeed != 0.0 && full > 0.0)
            {
                double avg = AvgCycleMs(i, factorReload);
                double use = avg > 0.0 ? capNeed / (avg / 1000.0) : 0.0;
                if (use > 0.0) used += use; else added -= use;
                row["cap_use_gj_s"] = use;
                drains.Add(new CapDrain(Math.Truncate(full), capNeed, NumShots(i), G(i, _k.ReloadTime), isInjector, _f.HasEffect(i, _k.TurretFitted)));
            }
            moduleRows.Add(row);
        }
        // incoming neuts / nos / cap transfers (Pyfa fit.addDrain): no stagger, after the fit's own modules
        double sigNow = G(ship, _k.SignatureRadius);
        foreach (var e in _f.Incoming)
            if (e is IncomingCapacitor d)
            {
                double need = G(d.Item, d.Amount) * d.Factor * d.Sign;
                if (!d.Resist.IsNone) need *= G(ship, d.Resist);
                double sres = G(d.Item, _k.EnergyNeutralizerSignatureResolution);
                if (sres != 0.0) need *= Math.Min(sigNow / sres, 1.0);
                double dur = G(d.Item, d.Duration);
                if (need != 0.0 && dur > 0.0) drains.Add(new CapDrain(Math.Truncate(dur), need, 0, 0.0, false, false));
            }
        var o = new JObj
        {
            { "capacity", cap }, { "recharge_time_s", rr / 1000.0 }, { "peak_recharge_gj_s", peak },
            { "use_gj_s", used }, { "injected_gj_s", added }, { "delta_gj_s", peak + added - used },
        };
        if (drains.Count == 0)
        {
            o["stable"] = true;
            o["stable_percent"] = 100.0;
        }
        else
        {
            var cs = _req.Options.CapSim;
            var r = CapSim.Simulate(cap, rr, drains, 1.0, cs.Reload || factorReload, true, (cs.MaxTimeS ?? 6.0 * 3600.0) * 1000.0);
            double st = (r.StableLow + r.StableHigh) / 2.0;
            bool stable = r.Stable && st > 0.0;
            o["stable"] = stable;
            if (stable) o["stable_percent"] = Math.Min(st * 100.0, 100.0);
            else o["depletes_in_s"] = r.TimeS;
            o["eve_stable_percent"] = r.EveStable * 100.0;
            o["sim_iterations"] = r.Iterations;
        }
        return o;
    }

    public static string SlotName(Slot s) => s switch
    {
        Slot.High => "high", Slot.Mid => "mid", Slot.Low => "low", Slot.Rig => "rig", Slot.Subsystem => "subsystem", _ => "service",
    };
    public static string StateName(ModuleState s) => s switch
    {
        ModuleState.Offline => "offline", ModuleState.Online => "online", ModuleState.Active => "active", _ => "overheated",
    };

    // ---------------------------------------------------------------- navigation & targeting
    private JObj Navigation()
    {
        int ship = _f.Ship;
        double maxv = G(ship, _k.MaxVelocity), limit = G(ship, _k.SpeedLimit);
        double maxSpeed = limit > 0.0 && maxv > limit ? limit : maxv;
        double mass = G(ship, KnownIds.Mass), agility = G(ship, _k.Agility);
        double baseWarp = G(ship, _k.BaseWarpSpeed); if (baseWarp == 0.0) baseWarp = 1.0;
        double warpMult = G(ship, _k.WarpSpeedMultiplier); if (warpMult == 0.0) warpMult = 1.0;
        double warpNeed = G(ship, _k.WarpCapacitorNeed);
        double cap = G(ship, _k.CapacitorCapacity);
        return new JObj
        {
            { "max_velocity", maxSpeed }, { "align_time_s", -Math.Log(0.25) * agility * mass / 1e6 }, { "mass", mass }, { "agility", agility },
            { "signature_radius", G(ship, _k.SignatureRadius) }, { "warp_speed_au_s", baseWarp * warpMult },
            { "max_warp_distance_au", warpNeed > 0.0 && mass > 0.0 ? cap / (mass * warpNeed) : 0.0 },
            { "warp_scramble_status", G(ship, _k.WarpScrambleStatus) },
        };
    }

    private JObj Targeting()
    {
        int ship = _f.Ship;
        var tp = _req.TargetProfile ?? TargetProfile.Default;
        var strengths = new[] { ("radar", _k.ScanRadarStrength), ("ladar", _k.ScanLadarStrength), ("magnetometric", _k.ScanMagnetometricStrength), ("gravimetric", _k.ScanGravimetricStrength) };
        (string Name, double V) best = ("none", 0.0);
        foreach (var (n, a) in strengths) { double v = G(ship, a); if (v > best.V) best = (n, v); }
        double scanRes = G(ship, _k.ScanResolution);
        double sig = G(ship, _k.SignatureRadius);
        JNode Lt(double s) => JNode.Of(Formulas.LockTime(scanRes, s));
        double shipTargets = G(ship, _k.MaxLockedTargets), charTargets = G(_f.Char, _k.MaxLockedTargets);
        return new JObj
        {
            { "max_targets", Math.Min(shipTargets, Math.Max(charTargets, 0.0)) },
            { "max_range_m", G(ship, _k.MaxTargetRange) }, { "scan_resolution", scanRes },
            { "sensor_strength", best.V }, { "sensor_type", best.Name },
            { "probe_size", best.V > 0.0 ? Math.Max(sig / best.V, 1.08) : JNode.Null },
            { "lock_time_s", new JObj
                {
                    { "sig_25m", Lt(25) }, { "sig_40m", Lt(40) }, { "sig_125m", Lt(125) }, { "sig_400m", Lt(400) },
                    { "sig_target_profile", tp.SignatureRadius is double s ? Lt(s) : JNode.Null },
                } },
        };
    }
}
