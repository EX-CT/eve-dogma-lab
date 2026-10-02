"""Fit statistics on top of evaluated attributes (Pyfa-equivalent formulas, same output as the reference)."""
import math

from . import capsim
from .engine import ACTIVE, ONLINE
from .request import STATE_NAMES

DMG_KEYS = ("em", "thermal", "kinetic", "explosive")


def float_unerr(v):
    return _rnd(v * 1e9) / 1e9


def _rnd(x):
    return math.floor(x + 0.5) if x >= 0 else -math.floor(-x + 0.5)


def spoolup(mx, step, cycle_s, spool):
    if mx == 0.0 or step == 0.0:
        return 0.0
    kind, amount = spool["type"], spool["amount"]
    if kind == "spool_scale":
        cycles = math.ceil(float_unerr(mx * amount / step))
    elif kind == "cycle_scale":
        cycles = _rnd(amount * math.ceil(float_unerr(mx / step)))
    elif kind == "time":
        cycles = min(math.floor(float_unerr(amount / cycle_s)), math.ceil(float_unerr(mx / step)))
    else:
        cycles = min(math.floor(amount), math.ceil(float_unerr(mx / step)))
    return min(cycles * step, mx)


def lock_time(scan_res, sig):
    if scan_res <= 0.0 or sig <= 0.0:
        return None
    return min(40000.0 / scan_res / math.asinh(sig) ** 2, 1800.0)


def dmg(em=0.0, th=0.0, ki=0.0, ex=0.0):
    return [em, th, ki, ex]


def djson(d):
    return {"em": d[0], "thermal": d[1], "kinetic": d[2], "explosive": d[3], "total": d[0] + d[1] + d[2] + d[3]}


def tidy(v):
    if isinstance(v, float):
        if math.isfinite(v):
            r = _rnd(v * 1e6) / 1e6
            return float(r)
        return v
    if isinstance(v, dict):
        return {k: tidy(x) for k, x in v.items()}
    if isinstance(v, list):
        return [tidy(x) for x in v]
    return v


class FitStats:
    def __init__(self, batch, fit, vals):
        self.b, self.fit, self.v, self.ds = batch, fit, vals, batch.ds
        self.meta = batch.meta

    # helpers
    def g(self, i, name):
        return self.v.get(i, self.ds.a(name))

    def has_eff(self, i, names):
        en = self.ds.effect_name
        return any(en.get(e) in names for e, _ in self.meta[i]["effects"])

    def raw_cycle_ms(self, i):
        v = max(self.g(i, "speed"), self.g(i, "duration"))
        for n in ("durationHighisGood", "durationSensorDampeningBurstProjector", "durationTargetIlluminationBurstProjector",
                  "durationECMJammerBurstProjector", "durationWeaponDisruptionBurstProjector"):
            a = self.ds.a(n)
            if a:
                v = max(v, self.v.get(i, a))
        return v

    def num_charges(self, i):
        c = self.meta[i]["charge"]
        if c is None:
            return 0
        vol = self.v.get(c, 161)
        cap = self.v.base(i, 38)
        return 0 if vol <= 0.0 else int(math.floor(float_unerr(cap / vol)))

    def num_shots(self, i):
        c = self.meta[i]["charge"]
        if c is None:
            return 0
        n = self.num_charges(i)
        cr = self.ds.a("chargeRate")
        if n > 0 and self.v.has(i, cr):
            r = self.v.get(i, cr)
            return int(math.floor(n / r)) if r > 0.0 else 0
        cgd = self.ds.a("crystalsGetDamaged")
        if n > 0 and self.v.has(c, cgd):
            if self.v.get(c, cgd) == 1.0:
                hp = self.v.get(c, 9)
                chance = self.g(c, "crystalVolatilityChance")
                dm = self.g(c, "crystalVolatilityDamage")
                if dm * chance > 0.0:
                    return int(math.floor((n * hp) / (dm * chance)))
            return 0
        return 0

    def avg_cycle_ms(self, i, factor_reload):
        active = self.raw_cycle_ms(i)
        if active == 0.0:
            return 0.0
        inactive = self.g(i, "moduleReactivationDelay")
        shots = self.num_shots(i)
        reload = self.g(i, "reloadTime")
        if not factor_reload or shots == 0 or inactive >= reload:
            return active + inactive
        return ((active + inactive) * (shots - 1.0) + (active + reload)) / shots

    def module_volley(self, i):
        if self.has_eff(i, ("turretFitted",)):
            kind = "turret"
        elif self.has_eff(i, ("launcherFitted",)):
            kind = "missile"
        elif self.has_eff(i, ("empWave",)):
            kind = "smartbomb"
        elif self.has_eff(i, ("ChainLightning",)):
            kind = "vorton"
        else:
            kind = "other"
        c = self.meta[i]["charge"]
        src = c if c is not None else i
        dm = self.ds.a("damageMultiplier")
        mult = self.v.get(i, dm) if self.v.has(i, dm) else 1.0
        if kind == "missile" and c is not None:
            mult *= self.g(self.fit.char, "missileDamageMultiplier")
        d = [self.g(src, n) * mult for n in ("emDamage", "thermalDamage", "kineticDamage", "explosiveDamage")]
        return d, kind

    def compute(self):
        ds, fit, v, meta = self.ds, self.fit, self.v, self.meta
        req = fit.req
        opts = req["options"]
        g = self.g
        ship, ch = fit.ship, fit.char
        factor_reload = opts["factor_reload"]
        mods = fit.modules
        st = lambda i: meta[i]["state"]  # noqa: E731
        name = lambda i: ds.t_name[meta[i]["ti"]]  # noqa: E731

        # ---------------- resources
        cpu_a, pow_a = ds.a("cpu"), ds.a("power")
        cpu_used = sum(v.get(i, cpu_a) for i in mods if st(i) >= ONLINE)
        pg_used = sum(v.get(i, pow_a) for i in mods if st(i) >= ONLINE)
        calib_used = sum(g(i, "upgradeCost") for i in mods if meta[i]["slot"] == "rig")
        drones, fighters = fit.drones, fit.fighters
        bw_used = sum(g(i, "droneBandwidthUsed") * meta[i]["active_count"] for i in drones)
        bay_used = sum(v.get(i, 161) * meta[i]["quantity"] for i in drones)
        fbay_used = sum(v.get(i, 161) * meta[i]["quantity"] for i in fighters)
        cargo_used = 0.0
        for c in req["cargo"]:
            ti = ds.tidx(c["type_id"])
            cargo_used += (float(ds.t_vol[ti]) if ti >= 0 else 0.0) * c["quantity"]
        count_slot = lambda s: float(sum(1 for i in mods if meta[i]["slot"] == s))  # noqa: E731
        turrets = [i for i in mods if self.has_eff(i, ("turretFitted",))]
        launchers = [i for i in mods if self.has_eff(i, ("launcherFitted",))]
        usage = lambda u, t: {"used": float(u), "total": float(t)}  # noqa: E731

        def fclass(i):
            if g(i, "fighterSquadronIsHeavy") > 0.0:
                return "heavy"
            if g(i, "fighterSquadronIsSupport") > 0.0:
                return "support"
            return "light"

        act_f = [i for i in fighters if meta[i]["active_count"] > 0]
        cls_used = lambda c: float(sum(1 for i in act_f if fclass(i) == c))  # noqa: E731
        resources = {
            "cpu": usage(cpu_used, g(ship, "cpuOutput")), "power": usage(pg_used, g(ship, "powerOutput")),
            "calibration": usage(calib_used, g(ship, "upgradeCapacity")),
            "drone_bandwidth": usage(bw_used, g(ship, "droneBandwidth")),
            "drone_bay": usage(bay_used, g(ship, "droneCapacity")),
            "fighter_bay": usage(fbay_used, g(ship, "fighterCapacity")),
            "cargo": usage(cargo_used, v.get(ship, 38)),
            "slots": {"high": usage(count_slot("high"), g(ship, "hiSlots")), "mid": usage(count_slot("mid"), g(ship, "medSlots")),
                      "low": usage(count_slot("low"), g(ship, "lowSlots")), "rig": usage(count_slot("rig"), g(ship, "rigSlots")),
                      "subsystem": usage(count_slot("subsystem"), g(ship, "maxSubSystems")),
                      "service": usage(count_slot("service"), g(ship, "serviceSlots"))},
            "hardpoints": {"turret": usage(len(turrets), g(ship, "turretSlotsLeft")),
                           "launcher": usage(len(launchers), g(ship, "launcherSlotsLeft"))},
            "fighter_tubes": {"total": usage(len(act_f), g(ship, "fighterTubes")),
                              "light": usage(cls_used("light"), g(ship, "fighterLightSlots")),
                              "support": usage(cls_used("support"), g(ship, "fighterSupportSlots")),
                              "heavy": usage(cls_used("heavy"), g(ship, "fighterHeavySlots"))},
        }

        # ---------------- offense
        tp = req["target_profile"] or {"em": 0.0, "thermal": 0.0, "kinetic": 0.0, "explosive": 0.0,
                                        "signature_radius": None, "max_velocity": None, "radius": None}
        tp_res = [tp["em"], tp["thermal"], tp["kinetic"], tp["explosive"]]
        default_spool = opts["default_spool"] or {"type": "spool_scale", "amount": 1.0}
        weapons = []
        w_vol, w_dps = dmg(), dmg()
        for i in mods:
            if st(i) < ACTIVE:
                continue
            base, kind = self.module_volley(i)
            if sum(base) == 0.0:
                continue
            cyc = self.avg_cycle_ms(i, factor_reload)
            raw = self.raw_cycle_ms(i)
            spool = meta[i]["spool"] or default_spool
            sp = spoolup(g(i, "damageMultiplierBonusMax"), g(i, "damageMultiplierBonusPerCycle"), raw / 1000.0, spool)
            vs = [x * (1.0 + sp) for x in base]
            dps = [x * (1000.0 / cyc) for x in vs] if cyc > 0.0 else dmg()
            w_vol = [a + b for a, b in zip(w_vol, vs)]
            w_dps = [a + b for a, b in zip(w_dps, dps)]
            c = meta[i]["charge"]
            w = {"module_index": meta[i]["req_index"], "type_id": meta[i]["type_id"], "name": name(i), "kind": kind,
                 "charge_type_id": meta[c]["type_id"] if c is not None else None,
                 "volley": djson(vs), "dps": djson(dps), "cycle_time_ms": cyc}
            if kind == "turret":
                w["optimal_m"] = g(i, "maxRange")
                w["falloff_m"] = g(i, "falloff")
                w["tracking"] = g(i, "trackingSpeed")
            elif kind == "missile":
                if c is not None:
                    vel = g(c, "maxVelocity")
                    if vel > 0.0:
                        # Pyfa missile range: flight time + ship radius, acceleration phase, floor/ceil blend,
                        # FoF limit, centre-to-surface
                        radius = g(ship, "radius")
                        ft = float_unerr(g(c, "explosionDelay") / 1000.0 + radius / vel)
                        accel_cap = g(c, "mass") * g(c, "agility") / 1e6

                        def range_at(t):
                            acc = min(t, accel_cap)
                            return vel / 2.0 * acc + vel * (t - acc)

                        lt, ht = math.floor(ft), math.ceil(ft)
                        lr, hr = range_at(lt), range_at(ht)
                        if self.has_eff(c, ("fofMissileLaunching",)):
                            lim = g(c, "maxFOFTargetRange")
                            if lim > 0.0:
                                lr, hr = min(lr, lim), min(hr, lim)
                        lr, hr = max(lr - radius, 0.0), max(hr - radius, 0.0)
                        hc = ft - lt
                        w["range_m"] = lr * (1.0 - hc) + hr * hc
                    w["explosion_radius"] = g(c, "aoeCloudSize")
                    w["explosion_velocity"] = g(c, "aoeVelocity")
            elif kind == "smartbomb":
                w["range_m"] = g(i, "empFieldRange")
            if sp > 0.0:
                w["spool_multiplier"] = 1.0 + sp
                w["volley_unspooled"] = djson(base)
            weapons.append(w)
        d_vol, d_dps, drone_out = dmg(), dmg(), []
        dm_a = ds.a("damageMultiplier")
        for i in drones:
            n = float(meta[i]["active_count"])
            if n == 0.0:
                continue
            mult = v.get(i, dm_a) if v.has(i, dm_a) else 1.0
            vv = [g(i, k) * (mult * n) for k in ("emDamage", "thermalDamage", "kineticDamage", "explosiveDamage")]
            cyc = self.raw_cycle_ms(i)
            if sum(vv) == 0.0 or cyc == 0.0:
                continue
            dps = [x * (1000.0 / cyc) for x in vv]
            d_vol = [a + b for a, b in zip(d_vol, vv)]
            d_dps = [a + b for a, b in zip(d_dps, dps)]
            drone_out.append({"drone_index": meta[i]["req_index"], "type_id": meta[i]["type_id"], "name": name(i),
                              "count": n, "volley": djson(vv), "dps": djson(dps)})
        f_vol, f_dps, fighter_out = dmg(), dmg(), []
        for i in fighters:
            n = float(meta[i]["active_count"])
            if n == 0.0:
                continue
            fv, fd = dmg(), dmg()
            for eff, prefix in (("fighterAbilityAttackM", "fighterAbilityAttackMissile"), ("fighterAbilityMissiles", "fighterAbilityMissiles")):
                eid = ds.e(eff)
                found = [d for e, d in meta[i]["effects"] if e == eid]
                if not found:
                    continue
                ab = meta[i]["fighter_abilities"]
                used = (eid in ab) if ab is not None else found[0]
                if not used:
                    continue
                m = g(i, prefix + "DamageMultiplier")
                m = 1.0 if m == 0.0 else m
                vv = [g(i, prefix + s) * (m * n) for s in ("DamageEM", "DamageTherm", "DamageKin", "DamageExp")]
                dur = g(i, prefix + "Duration")
                fv = [a + b for a, b in zip(fv, vv)]
                if dur > 0.0:
                    fd = [a + b * (1000.0 / dur) for a, b in zip(fd, vv)]
            if sum(fv) > 0.0:
                f_vol = [a + b for a, b in zip(f_vol, fv)]
                f_dps = [a + b for a, b in zip(f_dps, fd)]
                fighter_out.append({"fighter_index": meta[i]["req_index"], "type_id": meta[i]["type_id"], "name": name(i),
                                    "squadron_size": n, "volley": djson(fv), "dps": djson(fd)})
        t_vol = [a + b + c for a, b, c in zip(w_vol, d_vol, f_vol)]
        t_dps = [a + b + c for a, b, c in zip(w_dps, d_dps, f_dps)]
        vsr = lambda d: sum(x * (1.0 - r) for x, r in zip(d, tp_res))  # noqa: E731
        offense = {"weapons": weapons, "drones": drone_out, "fighters": fighter_out,
                   "total": {"weapon_dps": sum(w_dps), "weapon_volley": sum(w_vol), "drone_dps": sum(d_dps),
                             "drone_volley": sum(d_vol), "fighter_dps": sum(f_dps), "fighter_volley": sum(f_vol),
                             "dps": djson(t_dps), "volley": djson(t_vol)},
                   "vs_target_profile": {"dps": vsr(t_dps), "volley": vsr(t_vol)}}

        # ---------------- defense
        dp = req["damage_pattern"] or {"em": 25.0, "thermal": 25.0, "kinetic": 25.0, "explosive": 25.0}
        dpl = [dp["em"], dp["thermal"], dp["kinetic"], dp["explosive"]]
        dp_tot = max(sum(dpl), 1e-12)

        def layer(prefix):
            if not prefix:
                names = ["emDamageResonance", "thermalDamageResonance", "kineticDamageResonance", "explosiveDamageResonance"]
            else:
                names = [f"{prefix}{k}DamageResonance" for k in ("Em", "Thermal", "Kinetic", "Explosive")]
            return [g(ship, n) for n in names]

        def effectivify(amount, r):
            div = sum(p * x for p, x in zip(dpl, r)) / dp_tot
            return amount if div == 0.0 else amount / div

        rs, ra, rh = layer("shield"), layer("armor"), layer("")
        hp_s, hp_a, hp_h = g(ship, "shieldCapacity"), g(ship, "armorHP"), v.get(ship, 9)
        e_s, e_a, e_h = effectivify(hp_s, rs), effectivify(hp_a, ra), effectivify(hp_h, rh)
        rj = lambda r: dict(zip(DMG_KEYS, r))  # noqa: E731
        shield_rep = armor_rep = hull_rep = 0.0
        for i in mods:
            if st(i) < ACTIVE:
                continue
            dur = g(i, "duration") / 1000.0
            if dur <= 0.0:
                continue
            if self.has_eff(i, ("shieldBoosting", "fueledShieldBoosting")):
                shield_rep += g(i, "shieldBonus") / dur
            if self.has_eff(i, ("armorRepair",)):
                armor_rep += g(i, "armorDamageAmount") / dur
            if self.has_eff(i, ("fueledArmorRepair",)):
                c = meta[i]["charge"]
                paste = c is not None and ds.t_name[meta[c]["ti"]] == "Nanite Repair Paste"
                armor_rep += g(i, "armorDamageAmount") * (3.0 if paste else 1.0) / dur
            if self.has_eff(i, ("structureRepair",)):
                hull_rep += g(i, "structureDamageAmount") / dur
        # incoming remote repairs (Pyfa's diminishing-returns formula for stacked remote reps)
        lists = ([], [], [])
        for ps in fit.proj_special:
            if ps[0] == "rep":
                _, item, layer, amount, mult, factor = ps
                dur = g(item, "duration") / 1000.0
                if dur > 0.0:
                    lists[layer].append((v.get(item, amount) * mult * factor, dur))

        def applied(lst):
            total = sum(a_ / math.trunc(c_) for a_, c_ in lst)
            out_ = 0.0
            for a_, c_ in lst:
                rrps = a_ / math.trunc(c_)
                m_ = 7000.0 + rrps * 20.0
                out_ += (1.0 - (((rrps + m_) / (total + m_)) - 1.0) ** 2) * a_ / c_
            return out_

        shield_rep += applied(lists[0])
        armor_rep += applied(lists[1])
        hull_rep += applied(lists[2])
        srr = g(ship, "shieldRechargeRate") / 1000.0
        passive = 10.0 / srr * 0.5 * 0.5 * hp_s if srr > 0.0 else 0.0
        defense = {
            "hp": {"shield": hp_s, "armor": hp_a, "hull": hp_h, "total": hp_s + hp_a + hp_h},
            "resonance": {"shield": rj(rs), "armor": rj(ra), "hull": rj(rh)},
            "ehp": {"shield": e_s, "armor": e_a, "hull": e_h, "total": e_s + e_a + e_h},
            "damage_pattern": dict(dp),
            "tank": {"raw": {"passive_shield": passive, "shield_repair": shield_rep, "armor_repair": armor_rep, "hull_repair": hull_rep},
                     "effective": {"passive_shield": effectivify(passive, rs), "shield_repair": effectivify(shield_rep, rs),
                                   "armor_repair": effectivify(armor_rep, ra), "hull_repair": effectivify(hull_rep, rh)}},
        }

        # ---------------- capacitor
        cap = g(ship, "capacitorCapacity")
        rr = g(ship, "rechargeRate")
        peak = 10.0 / (rr / 1000.0) * 0.5 * 0.5 * cap if rr > 0.0 else 0.0
        drains, rows = [], []
        cap_used = cap_added = 0.0
        for i in mods:
            cap_need = g(i, "capacitorNeed")
            is_inj = ds.group_name.get(meta[i]["group"]) == "Capacitor Booster"
            if is_inj:
                c = meta[i]["charge"]
                cap_need = -(g(c, "capacitorBonus") if c is not None else 0.0)
            if self.has_eff(i, ("energyNosferatuFalloff",)) and not opts["nos_no_target_cap"]:
                cap_need = -g(i, "powerTransferAmount")
            cyc_raw = self.raw_cycle_ms(i)
            full = cyc_raw + g(i, "moduleReactivationDelay")
            row = {"module_index": meta[i]["req_index"], "type_id": meta[i]["type_id"], "name": name(i),
                   "slot": meta[i]["slot"], "state": STATE_NAMES[st(i)], "cpu": v.get(i, cpu_a), "power": v.get(i, pow_a)}
            if cyc_raw > 0.0:
                row["cycle_time_ms"] = cyc_raw
            if st(i) >= ACTIVE and cap_need != 0.0 and full > 0.0:
                avg = self.avg_cycle_ms(i, factor_reload)
                use = cap_need / (avg / 1000.0) if avg > 0.0 else 0.0
                if use > 0.0:
                    cap_used += use
                else:
                    cap_added -= use
                row["cap_use_gj_s"] = use
                drains.append((float(math.trunc(full)), cap_need, self.num_shots(i), g(i, "reloadTime"), is_inj,
                               self.has_eff(i, ("turretFitted",))))
            rows.append(row)
        # incoming neuts / nos / cap transfers: extra simulation drains after the fit's own modules
        sig_now = g(ship, "signatureRadius")
        for ps in fit.proj_special:
            if ps[0] == "drain":
                _, item, amount, duration, factor, resist, sign = ps
                need = v.get(item, amount) * factor * sign
                if resist:
                    need *= v.get(ship, resist)
                sres = g(item, "energyNeutralizerSignatureResolution")
                if sres != 0.0:
                    need *= min(sig_now / sres, 1.0)
                dur = v.get(item, duration)
                if need != 0.0 and dur > 0.0:
                    drains.append((float(math.trunc(dur)), need, 0, 0.0, False, False))
        capj = {"capacity": cap, "recharge_time_s": rr / 1000.0, "peak_recharge_gj_s": peak, "use_gj_s": cap_used,
                "injected_gj_s": cap_added, "delta_gj_s": peak + cap_added - cap_used}
        if not drains:
            capj["stable"] = True
            capj["stable_percent"] = 100.0
        else:
            o = opts["cap_sim"]
            mt = o["max_time_s"] if o["max_time_s"] is not None else 6.0 * 3600.0
            r = capsim.simulate(cap, rr, drains, 1.0, o["reload"] or factor_reload, True, mt * 1000.0)
            stv = (r["stable_low"] + r["stable_high"]) / 2.0
            capj["stable"] = bool(r["stable"] and stv > 0.0)
            if r["stable"] and stv > 0.0:
                capj["stable_percent"] = min(stv * 100.0, 100.0)
            else:
                capj["depletes_in_s"] = r["t_s"]
            capj["eve_stable_percent"] = r["eve_stable"] * 100.0
            capj["sim_iterations"] = r["iterations"]

        # ---------------- navigation
        maxv = g(ship, "maxVelocity")
        limit = g(ship, "speedLimit")
        mass = v.get(ship, 4)
        agility = g(ship, "agility")
        bw = g(ship, "baseWarpSpeed") or 1.0
        wm = g(ship, "warpSpeedMultiplier") or 1.0
        warp_need = g(ship, "warpCapacitorNeed")
        sig = g(ship, "signatureRadius")
        navigation = {"max_velocity": limit if (limit > 0.0 and maxv > limit) else maxv,
                      "align_time_s": -math.log(0.25) * agility * mass / 1e6, "mass": mass, "agility": agility,
                      "signature_radius": sig, "warp_speed_au_s": bw * wm,
                      "max_warp_distance_au": cap / (mass * warp_need) if warp_need > 0.0 and mass > 0.0 else 0.0,
                      "warp_scramble_status": g(ship, "warpScrambleStatus")}

        # ---------------- targeting
        best = ("none", 0.0)
        for n, at in (("radar", "scanRadarStrength"), ("ladar", "scanLadarStrength"),
                      ("magnetometric", "scanMagnetometricStrength"), ("gravimetric", "scanGravimetricStrength")):
            x = g(ship, at)
            if x > best[1]:
                best = (n, x)
        scan_res = g(ship, "scanResolution")
        lt = lambda s: lock_time(scan_res, s)  # noqa: E731
        targeting = {"max_targets": min(g(ship, "maxLockedTargets"), max(g(ch, "maxLockedTargets"), 0.0)),
                     "max_range_m": g(ship, "maxTargetRange"), "scan_resolution": scan_res,
                     "sensor_strength": best[1], "sensor_type": best[0],
                     "probe_size": max(sig / best[1], 1.08) if best[1] > 0.0 else None,
                     "lock_time_s": {"sig_25m": lt(25.0), "sig_40m": lt(40.0), "sig_125m": lt(125.0), "sig_400m": lt(400.0),
                                     "sig_target_profile": lt(tp["signature_radius"]) if tp.get("signature_radius") is not None else None}}
        drones_j = {"active": sum(meta[i]["active_count"] for i in drones),
                    "max_active": g(ch, "maxActiveDrones"), "control_range_m": g(ch, "droneControlDistance")}
        sti = meta[ship]["ti"]
        out = {"meta": {"schema_version": 1, "engine": ENGINE, "sde_build": ds.build, "dataset_sha256": ds.sha256},
               "ship": {"type_id": meta[ship]["type_id"], "name": ds.t_name[sti], "group": ds.group_name.get(meta[ship]["group"])},
               "resources": resources, "offense": offense, "defense": defense, "capacitor": capj,
               "navigation": navigation, "targeting": targeting, "drones": drones_j, "modules": rows}
        if opts["validate"]:
            out["violations"] = self.validate(cpu_used, pg_used, calib_used, bw_used)
        if fit.warnings:
            out["warnings"] = list(fit.warnings)
        inc = opts["include_attributes"]
        if inc == "ship":
            out["attributes"] = {"ship": self.dump(ship)}
        elif inc == "all":
            out["attributes"] = {
                "ship": self.dump(ship), "character": self.dump(ch),
                "modules": [{"module_index": meta[i]["req_index"], "type_id": meta[i]["type_id"], "attributes": self.dump(i),
                             "charge": self.dump(meta[i]["charge"]) if meta[i]["charge"] is not None else None} for i in mods],
                "drones": [{"drone_index": meta[i]["req_index"], "attributes": self.dump(i)} for i in drones]}
        return tidy(out)

    def dump(self, i):
        ev = self.v.ev
        lo = __import__("numpy").searchsorted(ev.keys, i << 14)
        hi = __import__("numpy").searchsorted(ev.keys, (i + 1) << 14)
        out = {}
        for k, x in zip(ev.keys[lo:hi].tolist(), ev.val[lo:hi].tolist()):
            a = k & 0x3FFF
            out[self.ds.attr_name.get(a, str(a))] = x
        return dict(sorted(out.items()))

    def validate(self, cpu, pg, calib, bw):
        ds, meta, g = self.ds, self.meta, self.g
        ship = self.fit.ship
        out = []

        def push(code, msg, idx):
            out.append({"code": code, "message": msg, "module_index": idx})

        if cpu > g(ship, "cpuOutput") + 1e-9:
            push("CPU_OVERLOAD", f"CPU used {cpu:.2f} > output {g(ship, 'cpuOutput'):.2f}", None)
        if pg > g(ship, "powerOutput") + 1e-9:
            push("POWER_OVERLOAD", f"Powergrid used {pg:.2f} > output {g(ship, 'powerOutput'):.2f}", None)
        if calib > g(ship, "upgradeCapacity") + 1e-9:
            push("CALIBRATION_OVERLOAD", f"Calibration used {_rf(calib)} > {_rf(g(ship, 'upgradeCapacity'))}", None)
        if bw > g(ship, "droneBandwidth") + 1e-9:
            push("DRONE_BANDWIDTH", f"Drone bandwidth used {_rf(bw)} > {_rf(g(ship, 'droneBandwidth'))}", None)
        mods = self.fit.modules
        for slot, attr, label in (("high", "hiSlots", "High"), ("mid", "medSlots", "Mid"), ("low", "lowSlots", "Low"),
                                  ("rig", "rigSlots", "Rig"), ("subsystem", "maxSubSystems", "Subsystem"),
                                  ("service", "serviceSlots", "Service")):
            used = float(sum(1 for i in mods if meta[i]["slot"] == slot))
            if used > g(ship, attr):
                push("SLOTS_EXCEEDED", f"{label} slots used {_rf(used)} > {_rf(g(ship, attr))}", None)
        t = float(sum(1 for i in mods if self.has_eff(i, ("turretFitted",))))
        if t > g(ship, "turretSlotsLeft"):
            push("TURRET_HARDPOINTS", f"turrets {_rf(t)} > hardpoints {_rf(g(ship, 'turretSlotsLeft'))}", None)
        l_ = float(sum(1 for i in mods if self.has_eff(i, ("launcherFitted",))))
        if l_ > g(ship, "launcherSlotsLeft"):
            push("LAUNCHER_HARDPOINTS", f"launchers {_rf(l_)} > hardpoints {_rf(g(ship, 'launcherSlotsLeft'))}", None)
        sti = meta[ship]["ti"]
        ship_group, ship_id, ship_name = meta[ship]["group"], meta[ship]["type_id"], ds.t_name[sti]
        g_attrs = [a for a in (ds.a(f"canFitShipGroup{k:02d}") for k in range(1, 21)) if a]
        t_attrs = [a for a in (ds.a(f"canFitShipType{k}") for k in range(1, 12)) if a]
        fitted_group, fitted_type, active_group, online_group = {}, {}, {}, {}
        for i in mods:
            it = meta[i]
            idx = it["req_index"]
            ti = it["ti"]
            nm = ds.t_name[ti]
            ta = lambda a: ds.type_attr(ti, a)  # noqa: E731
            if it["slot"] is None:
                push("NOT_FITTABLE", f"{nm} is not a fittable module", idx)
            gr = [int(x) for x in (ta(a) for a in g_attrs) if x is not None and int(x) != 0]
            ty = [int(x) for x in (ta(a) for a in t_attrs) if x is not None and int(x) != 0]
            if (gr or ty) and ship_group not in gr and ship_id not in ty:
                push("SHIP_RESTRICTION", f"{nm} cannot be fitted to {ship_name}", idx)
            if it["slot"] == "rig":
                rsz = ta(ds.a("rigSize")) or 0.0
                srs = g(ship, "rigSize")
                if rsz != 0.0 and rsz != srs:
                    push("RIG_SIZE", f"{nm} rig size {_rf(rsz)} != ship rig size {_rf(srs)}", idx)
            fitted_group[it["group"]] = fitted_group.get(it["group"], 0) + 1
            fitted_type[it["type_id"]] = fitted_type.get(it["type_id"], 0) + 1
            if it["state"] >= ONLINE:
                online_group[it["group"]] = online_group.get(it["group"], 0) + 1
            if it["state"] >= ACTIVE:
                active_group[it["group"]] = active_group.get(it["group"], 0) + 1
            for attr, mp, key, code, word in (("maxGroupFitted", fitted_group, it["group"], "MAX_GROUP_FITTED", "fitted of group"),
                                              ("maxTypeFitted", fitted_type, it["type_id"], "MAX_TYPE_FITTED", "fitted"),
                                              ("maxGroupOnline", online_group, it["group"], "MAX_GROUP_ONLINE", "online of group"),
                                              ("maxGroupActive", active_group, it["group"], "MAX_GROUP_ACTIVE", "active of group")):
                lim = ta(ds.a(attr))
                if lim is None:
                    continue
                n = mp.get(key, 0)
                if lim > 0.0 and n > lim:
                    push(code, f"{nm}: {n} {word}, max {_rf(lim)}", idx)
            c = it["charge"]
            if c is not None:
                cti = meta[c]["ti"]
                cnm = ds.t_name[cti]
                cg = [int(x) for x in (ta(ds.a(f"chargeGroup{k}")) for k in range(1, 6)) if x is not None and int(x) != 0]
                if int(ds.t_group[cti]) not in cg:
                    push("CHARGE_GROUP", f"{cnm} cannot be loaded into {nm}", idx)
                ms = ta(ds.a("chargeSize"))
                cs = ds.type_attr(cti, ds.a("chargeSize"))
                if ms is not None and cs is not None and ms != cs:
                    push("CHARGE_SIZE", f"{cnm} size {_rf(cs)} != launcher size {_rf(ms)}", idx)
                if float(ds.t_vol[cti]) > float(ds.t_cap[ti]) and float(ds.t_cap[ti]) > 0.0:
                    push("CHARGE_CAPACITY", f"{cnm} does not fit into {nm}", idx)
        have = self.fit.skill_levels
        missing = []
        from .engine import SHIP as K_SHIP, MODULE, CHARGE, DRONE, FIGHTER, IMPLANT, BOOSTER
        req_attrs = [(ds.a(f"requiredSkill{k}"), ds.a(f"requiredSkill{k}Level")) for k in range(1, 7)]
        for i in self.fit.items:
            it = meta[i]
            if it["kind"] not in (K_SHIP, MODULE, CHARGE, DRONE, FIGHTER, IMPLANT, BOOSTER):
                continue
            ti = it["ti"]
            for sa, la in req_attrs:
                s = int(ds.type_attr(ti, sa, 0.0))
                if s == 0:
                    continue
                need = ds.type_attr(ti, la, 1.0)
                if have.get(s, 0.0) < need and not any(m[0] == s and m[1] >= need for m in missing):
                    missing.append((s, need, it["type_id"]))
        for s, need, by in missing:
            sti_ = ds.tidx(s)
            sn = ds.t_name[sti_] if sti_ >= 0 else "?"
            push("MISSING_SKILL", f"{sn} {_rf(need)} required by {ds.t_name[ds.tidx(by)]}", None)
        return out


def _rf(x):
    """format a float like Rust's Display for f64 (no trailing .0)"""
    if isinstance(x, float) and x.is_integer() and abs(x) < 1e16:
        return str(int(x))
    return repr(x)


ENGINE = "eve-dogma-g 0.1.0"
