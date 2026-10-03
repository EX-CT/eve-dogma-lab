"""Non-damage graphs: lock_time, warp_time, mobility, capacitor, shield_regen, ewar, remote_reps."""
import heapq
import math

import numpy as np

from .common import NAN, range_factor, regen_amount, regen_rate, stack_mult, unerr
from .ctx import ACTIVE, ONLINE, Ctx

AU = 149597870700.0


def _num(params, key, default):
    v = params.get(key, default)
    return default if v is None else float(v)


# ---------------------------------------------------------------- lock time
def lock_time(eng, req, c, xs, ys, params, settings):
    scan = c.g(c.ship, "scanResolution")
    with np.errstate(divide="ignore", invalid="ignore"):
        if scan > 0:
            t = np.minimum(40000.0 / scan / np.arcsinh(xs) ** 2, 1800.0)
        else:
            t = np.full(len(xs), c.g(c.ship, "scanSpeed") / 1000.0)
    t = np.where(xs >= 1, t, NAN)
    return {"time_s": t}


# ---------------------------------------------------------------- warp time
SUBWARP_OFF_GROUPS = ("Propulsion Module", "Mass Entanglers", "Cloaking Device", "Siege Module", "Super Weapon",
                      "Cynosural Field Generator", "Clone Vat Bay", "Jump Portal Generator")


def _subwarp_speed(eng, req, c):
    v = c.memo.get("subwarp")
    if v is not None:
        return v
    r = dict(req)
    mods = []
    for k, m in enumerate(req.get("modules") or []):
        m2 = dict(m)
        if m2.get("state") in ("active", "overheated"):
            ti = eng.ds.tidx(m2.get("type_id"))
            if ti >= 0 and (eng.ds.group_name.get(int(eng.ds.t_group[ti])) or "") in SUBWARP_OFF_GROUPS:
                m2["state"] = "online"
        mods.append(m2)
    r["modules"] = mods
    r["projected"] = []
    sc = eng.cache.get(r, tag="subwarp:")
    v = c.memo["subwarp"] = sc.g(sc.ship, "maxVelocity")
    return v


def warp_time(eng, req, c, xs, ys, params, settings):
    g = c.g
    sub = _subwarp_speed(eng, req, c)
    warp = (g(c.ship, "baseWarpSpeed") or 1.0) * (g(c.ship, "warpSpeedMultiplier") or 1.0)
    need = g(c.ship, "warpCapacitorNeed")
    maxd = 0.0 if not need else g(c.ship, "capacitorCapacity") / (g(c.ship, "mass") * need)
    k_acc, k_dec = warp, min(warp / 3.0, 2.0)
    drop = min(sub / 2.0, 100.0)
    vmax = warp * AU
    min_d = AU + vmax / k_dec
    d = np.asarray(xs, float)
    with np.errstate(divide="ignore", invalid="ignore"):
        short = min_d > d
        v = np.where(short, d * k_acc * k_dec / (k_acc + k_dec), vmax)
        cruise = np.where(short, 0.0, (d - min_d) / vmax)
        t = cruise + np.log(v / k_acc) / k_acc + np.log(v / drop) / k_dec
    t = np.where(d == 0, 0.0, t)
    t = np.where((d < 0) | (d > maxd * AU), NAN, t)
    return {"time_s": t}


# ---------------------------------------------------------------- mobility
def mobility(eng, req, c, xs, ys, params, settings):
    g = c.g
    v, m, a = g(c.ship, "maxVelocity"), g(c.ship, "mass"), g(c.ship, "agility")
    t = np.asarray(xs, float)
    with np.errstate(divide="ignore", invalid="ignore", over="ignore"):
        speed = v * (1 - np.exp((-t * 1000000) / (a * m)))
        out = {"speed_mps": speed}
        if "distance_m" in ys:
            out["distance_m"] = (v * t + (v * a * m * np.exp((-t * 1000000) / (a * m)) / 1000000)) - \
                (v * 0 + (v * a * m * math.exp((-0 * 1000000) / (a * m)) / 1000000))
        out["momentum_kg_mps"] = speed * m
        tm = _num(params, "tgt_mass_kg", 1.3e9) / 10 ** 6
        bm = m / 10 ** 6
        bs = (2 * speed * bm) / (bm + tm)
        out["bump_speed_mps"] = bs
        out["bump_distance_m"] = bs * tm * _num(params, "tgt_inertia", 0.015)
    bad = t < 0
    return {k: np.where(bad, NAN, x) for k, x in out.items() if k in ys}


# ---------------------------------------------------------------- shield regen
def _effectivify(c, amount, layer):
    dp = c.p["damage_pattern"] or {"em": 25.0, "thermal": 25.0, "kinetic": 25.0, "explosive": 25.0}
    tot = sum(dp[k] for k in ("em", "thermal", "kinetic", "explosive"))
    pre = {"shield": "shield", "armor": "armor", "hull": ""}[layer]
    div = 0.0
    for k, nm in (("em", "Em"), ("thermal", "Thermal"), ("kinetic", "Kinetic"), ("explosive", "Explosive")):
        an = f"{pre}{nm}DamageResonance" if pre else f"{nm[0].lower() + nm[1:]}DamageResonance"
        div += dp[k] / tot * c.g(c.ship, an)
    return amount / div


def shield_regen(eng, req, c, xs, ys, params, settings, axis):
    cmax = c.g(c.ship, "shieldCapacity")
    tau = c.g(c.ship, "shieldRechargeRate") / 1000.0
    x = np.asarray(xs, float)
    with np.errstate(divide="ignore", invalid="ignore"):
        if axis == "shield_pct":
            amt = x / 100.0 * cmax
            bad = (x < 0) | (x > 100)
        else:
            s0 = _num(params, "shield_start_pct", 0.0) / 100.0 * cmax
            amt = regen_amount(cmax, tau, s0, x)
            bad = x < 0
        rate = regen_rate(cmax, tau, amt)
    if params.get("effective"):
        amt, rate = _effectivify(c, amt, "shield"), _effectivify(c, rate, "shield")
    out = {"shield_hp": amt, "shield_regen_hp_s": rate}
    return {k: np.where(bad, NAN, out[k]) for k in ys}


# ---------------------------------------------------------------- capacitor
def cap_history(capacity, recharge_ms, drains, start, reload, t_max_ms=3600000.0):
    """Pyfa capSim history (stagger on, no repeat optimisation): sorted [(t_s, cap)] of every cap change.
    Same event model as variant-g's capsim, with the change log kept."""
    tau = recharge_ms / 5.0
    heap = []
    seq = 0
    groups = []
    for dur, need, clip, rl, inj, nost in drains:
        if not reload and not inj:
            clip, rl = 0, 0.0
        if dur <= 0.0:
            continue
        key = (dur, need, clip, rl, inj, nost)
        for gr in groups:
            if gr[0] == key:
                gr[1] += 1
                break
        else:
            groups.append([key, 1])
    for (dur, need, clip, rl, inj, nost), n in groups:
        if inj:
            for _ in range(n):
                heap.append([0.0, dur, need, 0, clip, rl, True, seq])
                seq += 1
            continue
        if not nost:
            if clip == 0:
                dur = math.floor(dur / n)
            else:
                stg = (dur * clip + rl) / (n * clip)
                for k in range(1, n):
                    heap.append([k * stg, dur, need, 0, clip, rl, False, seq])
                    seq += 1
        else:
            need *= n
        heap.append([0.0, dur, need, 0, clip, rl, False, seq])
        seq += 1
    heapq.heapify(heap)
    cap = start
    t_last = 0.0
    awaiting = []
    saved = {}
    exp, sqrt = math.exp, math.sqrt

    def fire_inj(a, t_now):
        nonlocal seq, cap
        _, dur, need, shot, clip, rl, isinj, _s = a
        cap = min(cap - need, capacity)
        saved[t_now] = cap
        t = t_now + dur
        shot += 1
        if clip and shot % clip == 0:
            shot = 0
            t += rl
        heapq.heappush(heap, [t, dur, need, shot, clip, rl, isinj, seq])
        seq += 1

    while heap:
        ev = heapq.heappop(heap)
        t_now = ev[0]
        if t_now >= t_max_ms:
            break
        if t_now > t_last:
            cap = ((1.0 + (sqrt(cap / capacity) - 1.0) * exp((t_last - t_now) / tau)) ** 2) * capacity
        t_last = t_now
        _, dur, need, shot, clip, rl, inj, _s = ev
        if inj and cap - need > capacity:
            awaiting.append(ev)
            continue
        if need > cap and cap < capacity:
            while awaiting and need > cap and capacity > cap:
                want = min(need - cap, capacity - cap)
                good = [a for a in awaiting if -a[2] >= want]
                pick = min(good, key=lambda a: -a[2]) if good else max(awaiting, key=lambda a: -a[2])
                awaiting.remove(pick)
                fire_inj(pick, t_now)
        cap = min(cap - need, capacity)
        saved[t_now] = cap
        if cap < 0.0:
            break
        while awaiting and cap < capacity:
            want = capacity - cap
            good = [a for a in awaiting if -a[2] <= want]
            if not good:
                break
            pick = max(good, key=lambda a: -a[2])
            awaiting.remove(pick)
            fire_inj(pick, t_now)
        t = t_now + dur
        shot += 1
        if clip and shot % clip == 0:
            shot = 0
            t += rl
        heapq.heappush(heap, [t, dur, need, shot, clip, rl, inj, seq])
        seq += 1
    return [(k / 1000.0, max(0.0, saved[k])) for k in sorted(saved)]


def capacitor(eng, req, c, xs, ys, params, settings, axis):
    cmax = c.g(c.ship, "capacitorCapacity")
    tau = c.g(c.ship, "rechargeRate") / 1000.0
    x = np.asarray(xs, float)
    out = {}
    with np.errstate(divide="ignore", invalid="ignore"):
        if axis == "cap_pct":
            amt = x / 100.0 * cmax
            bad = (x < 0) | (x > 100)
            out["cap_gj"] = amt
            out["cap_regen_gj_s"] = regen_rate(cmax, tau, amt)
        else:
            bad = (x < 0) | (x > 3600)
            c0 = _num(params, "cap_start_pct", 100.0) / 100.0 * cmax
            smooth = regen_amount(cmax, tau, c0, x)
            out["cap_regen_gj_s"] = regen_rate(cmax, tau, smooth)
            if "cap_gj" in ys:
                use_sim = params.get("use_capsim", True) is not False
                hist = None
                if use_sim:
                    key = ("caphist", c0)
                    hist = c.memo.get(key)
                    if hist is None:
                        d = c.cap_drains()
                        hist = [] if not d else cap_history(d[0], d[1], d[2], c0, c.p["options"]["factor_reload"])
                        c.memo[key] = hist
                if not hist:
                    out["cap_gj"] = smooth
                else:
                    ht = np.array([h[0] for h in hist])
                    hc = np.array([h[1] for h in hist])
                    k = np.searchsorted(ht, x, side="right") - 1
                    kk = np.maximum(k, 0)
                    t0 = np.where(k >= 0, ht[kk], 0.0)
                    cb = np.where(k >= 0, hc[kk], c0)
                    adv = cmax * (1 + np.exp(5 * -(x - t0) / tau) * (np.sqrt(cb / cmax) - 1)) ** 2
                    val = np.where((k >= 0) & (t0 == x), cb, adv)
                    val = np.where(k == len(ht) - 1, NAN, val)
                    out["cap_gj"] = val
    return {k: np.where(bad, NAN, out[k]) for k in ys}


# ---------------------------------------------------------------- EWAR
GEN_ECM = ("scanGravimetricStrengthBonus", "scanLadarStrengthBonus", "scanMagnetometricStrengthBonus",
           "scanRadarStrengthBonus")
FTR_ECM = ("fighterAbilityECMStrengthGravimetric", "fighterAbilityECMStrengthLadar",
           "fighterAbilityECMStrengthMagnetometric", "fighterAbilityECMStrengthRadar")


def _lock_ok(c, settings, d):
    if settings["ignore_lock_range"]:
        return np.ones(len(d), bool)
    return d <= c.g(c.ship, "maxTargetRange")


def _dcr_ok(c, settings, d):
    if settings["ignore_drone_control_range"]:
        return np.ones(len(d), bool)
    return d <= c.drone_control_range()


def _ewar_sources(c, kind, res):
    """[(strength or (s1, s2), optimal, falloff, needs_lock, needs_dcr)]"""
    g = c.g
    out = []
    for i in c.active_modules():
        e = c.effs(i)
        rng, fo = c.max_range(i) or 0.0, c.falloff(i) or 0.0
        aoe = max(0.0, rng + g(i, "doomsdayAOERange"))
        if kind == "neut":
            cyc = c.cycle(i)
            dur = (cyc.average if cyc is not None else math.inf) / 1000.0
            for en in ("energyNeutralizerFalloff", "structureEnergyNeutralizerFalloff"):
                if en in e:
                    out.append((g(i, "energyNeutralizerAmount") / dur * res, rng, fo, True, False))
            if "energyNosferatuFalloff" in e and g(i, "nosOverride"):
                out.append((g(i, "powerTransferAmount") / dur * res, rng, fo, True, False))
            if "doomsdayAOENeut" in e:
                out.append((g(i, "energyNeutralizerAmount") / dur * res, aoe, fo, False, False))
            continue
        spec = {"web": (("remoteWebifierFalloff", "structureModuleEffectStasisWebifier"), "doomsdayAOEWeb", "speedFactor"),
                "ecm": (("remoteECMFalloff", "structureModuleEffectECM"), "doomsdayAOEECM", None),
                "damp": (("remoteSensorDampFalloff", "structureModuleEffectRemoteSensorDampener"), "doomsdayAOEDamp",
                         "maxTargetRangeBonus"),
                "td": (("shipModuleTrackingDisruptor", "structureModuleEffectWeaponDisruption"), "doomsdayAOETrack",
                       "maxRangeBonus"),
                "gd": (("shipModuleGuidanceDisruptor", "structureModuleEffectWeaponDisruption"), "doomsdayAOETrack",
                       None),
                "tp": (("remoteTargetPaintFalloff", "structureModuleEffectTargetPainter"), "doomsdayAOEPaint",
                       "signatureRadiusBonus")}[kind]
        if kind == "ecm":
            s = max(g(i, a) for a in GEN_ECM) * res
        elif kind == "gd":
            s = (g(i, "missileVelocityBonus") * res, g(i, "explosionDelayBonus") * res)
        else:
            s = g(i, spec[2]) * res
        for en in spec[0]:
            if en in e:
                out.append((s, rng, fo, True, False))
        if spec[1] in e:
            out.append((s, aoe, fo, False, False))
    dspec = {"neut": "entityEnergyNeutralizerFalloff", "web": "remoteWebifierEntity", "ecm": "entityECMFalloff",
             "damp": "remoteSensorDampEntity", "td": "npcEntityWeaponDisruptor", "tp": "remoteTargetPaintEntity"}
    if kind in dspec:
        for i, n in c.active_drones():
            if dspec[kind] not in c.effs(i):
                continue
            if kind == "neut":
                s = g(i, "energyNeutralizerAmount") / (g(i, "energyNeutralizerDuration") / 1000.0) * res
            elif kind == "ecm":
                s = max(g(i, a) for a in GEN_ECM) * res
            else:
                s = g(i, {"web": "speedFactor", "damp": "maxTargetRangeBonus", "td": "maxRangeBonus",
                          "tp": "signatureRadiusBonus"}[kind]) * res
            out.extend([(s, math.inf, 0.0, True, True)] * n)
    fspec = {"neut": "fighterAbilityEnergyNeutralizer", "web": "fighterAbilityStasisWebifier", "ecm": "fighterAbilityECM"}
    if kind in fspec:
        for i, n in c.active_fighters():
            if fspec[kind] not in c.fighter_abilities(i):
                continue
            if kind == "neut":
                dur = g(i, "fighterAbilityEnergyNeutralizerDuration")
                s = g(i, "fighterAbilityEnergyNeutralizerAmount") / (dur / 1000.0) * n * res
            elif kind == "web":
                s = g(i, "fighterAbilityStasisWebifierSpeedPenalty") * n * res
            else:
                s = max(g(i, a) for a in FTR_ECM) * n * res
            out.append((s, math.inf, 0.0, True, False))
    return out


EWAR_Y = {"neut_gj_s": "neut", "web_pct": "web", "ecm_strength": "ecm", "damp_lock_range_pct": "damp",
          "td_optimal_pct": "td", "gd_range_pct": "gd", "tp_sig_pct": "tp"}


EWAR_RESIST_ATTR = {"neut": "energyWarfareResistance", "web": "stasisWebifierResistance", "ecm": "ECMResistance",
                    "damp": "sensorDampenerResistance", "td": "weaponDisruptionResistance",
                    "gd": "weaponDisruptionResistance", "tp": "targetPainterResistance"}


def _clamp01(v):
    return min(1.0, max(0.0, float(v)))


def ewar(eng, req, c, xs, ys, params, settings, tctx=None):
    d = np.asarray(xs, float)
    pres = params.get("resist")
    lock, dcr = _lock_ok(c, settings, d), _dcr_ok(c, settings, d)
    disallow = tctx is not None and bool(tctx.g(tctx.ship, "disallowOffensiveModifiers"))
    out = {}
    for y in ys:
        kind = EWAR_Y[y]
        n = len(d)
        if disallow and kind != "neut":  # Pyfa's ewar handlers return early; neutralizers do not
            out[y] = np.zeros(n)
            continue
        if pres is not None or tctx is None:
            res = 1 - _clamp01(pres or 0)
        else:  # contract 0.2: resist = clamp(1 - T.ship[attr]) with 0 / missing counting as 1
            res = 1 - _clamp01(1 - (tctx.gopt(tctx.ship, EWAR_RESIST_ATTR[kind]) or 1))
        srcs = _ewar_sources(c, kind, res)
        if kind in ("neut", "ecm"):
            tot = np.zeros(n)
            for s, o, f, nl, nd in srcs:
                ok = (lock | (not nl)) & (dcr | (not nd))
                tot += np.where(ok, s * range_factor(o, f, d), 0.0)
            out[y] = tot
            continue
        rows, rows2 = [], []
        for s, o, f, nl, nd in srcs:
            ok = (lock | (not nl)) & (dcr | (not nd))
            rf = range_factor(o, f, d)
            if kind == "gd":
                rows.append(np.where(ok, 1 + s[0] * rf / 100, 1.0))
                rows2.append(np.where(ok, 1 + s[1] * rf / 100, 1.0))
            else:
                rows.append(np.where(ok, 1 + s * rf / 100, 1.0))
        m = stack_mult(rows) if rows else np.ones(n)
        if kind == "gd":
            m2 = stack_mult(rows2) if rows2 else np.ones(n)
            out[y] = (1 - m * m2) * 100
        elif kind == "tp":
            out[y] = (m - 1) * 100
        else:
            out[y] = (1 - m) * 100
    bad = d < 0
    return {k: np.where(bad, NAN, v) for k, v in out.items()}


# ---------------------------------------------------------------- remote reps
RR_GROUPS = {"Remote Armor Repairer": "armor", "Ancillary Remote Armor Repairer": "armor",
             "Mutadaptive Remote Armor Repairer": "armor", "Remote Hull Repairer": "hull",
             "Remote Shield Booster": "shield", "Ancillary Remote Shield Booster": "shield",
             "Remote Capacitor Transmitter": "capacitor"}


def _spool_mult(c, i, kind, amount):
    from evedogma_g.stats import spoolup
    mx, step = c.g(i, "repairMultiplierBonusMax"), c.g(i, "repairMultiplierBonusPerCycle")
    if not mx or not step:
        return 1.0
    sp = c.meta[i]["spool"]
    if sp is not None and kind != "cycles_forced":
        spool = sp
    elif kind == "cycles_forced":
        spool = {"type": "cycles", "amount": amount}
    else:
        spool = {"type": "spool_scale", "amount": 1.0}
    return 1.0 + spoolup(mx, step, c.raw_cycle_ms(i) / 1000.0, spool)


def _rr_mod_amount(c, i):
    """{delay_ms: hp (shield+armor+hull; capacitor dropped)} base amounts of a remote repper, or None"""
    t = RR_GROUPS.get(c.group(i))
    if t is None or t == "capacitor":
        return None
    g = c.g
    if t == "hull":
        a = g(i, "structureDamageAmount")
    elif t == "armor":
        mult = 1.0
        if c.group(i) == "Ancillary Remote Armor Repairer" and c.charge(i) is not None:
            mult = g(i, "chargedArmorDamageMultiplier") if c.has(i, "chargedArmorDamageMultiplier") else 1.0
        a = g(i, "armorDamageAmount") * mult
    else:
        a = g(i, "shieldBonus")
    if not a:
        return None
    return (0.0 if t == "shield" else c.raw_cycle_ms(i)), a


def _rr_drone_amount(c, i, n):
    g = c.g
    out = {}
    sh, ar, hu = g(i, "shieldBonus"), g(i, "armorDamageAmount"), g(i, "structureDamageAmount")
    if sh:
        out[0.0] = sh * n
    if ar or hu:
        out[c.drone_cycle_ms(i)] = (ar + hu) * n
    return out


def _rr_time_cache(c, anc_reload, tmax):
    key = ("rrtime", anc_reload)
    tc = c.memo.get(key)
    if tc is not None and tc[0] >= tmax:
        return tc[1]
    keys = []  # per key: (application kind, [(t_change, rps)], [(t, amount)])
    for i in c.active_modules():
        base = _rr_mod_amount(c, i)
        if base is None:
            continue
        e = c.effs(i)
        anc_s, anc_a = "shipModuleAncillaryRemoteShieldBooster" in e, "shipModuleAncillaryRemoteArmorRepairer" in e
        cyc = c.cycle(i, anc_reload if (anc_s or anc_a) else True)
        if cyc is None:
            continue
        cur, nonstop, without, until = 0.0, 0, 0, c.num_shots(i)
        segs, amts = [], []
        for act, ina, rl in cyc.iter():
            without += 1
            mult = _spool_mult(c, i, "cycles_forced", nonstop)
            delay, amount = base
            amount *= mult
            if anc_a and c.charge(i) is not None and not anc_reload and without > until:
                amount = amount / (c.g(i, "chargedArmorDamageMultiplier") if c.has(i, "chargedArmorDamageMultiplier") else 1.0)
            if amount > 0:
                amts.append((cur + delay / 1000.0, amount))
                segs.append((cur, cur + act / 1000.0, amount / (act / 1000.0 + 0.0) if act else 0.0))
            nonstop = 0 if ina > 0 else nonstop + 1
            if rl:
                without = 0
            if cur > tmax:
                break
            cur += act / 1000.0 + ina / 1000.0
        keys.append(("mod", i, segs, amts))
    for i, n in c.active_drones():
        amts0 = _rr_drone_amount(c, i, n)
        if not amts0:
            continue
        cyc = c.drone_cycle_ms(i)
        if not cyc:
            continue
        cur = 0.0
        segs, amts = [], []
        tot = sum(amts0.values())
        while True:
            for dl, a in amts0.items():
                amts.append((cur + dl / 1000.0, a))
            segs.append((cur, cur + cyc / 1000.0, tot / (cyc / 1000.0)))
            if cur > tmax:
                break
            cur += cyc / 1000.0
        keys.append(("drone", i, segs, amts))
    out = []
    from evedogma_g.stats import float_unerr
    for kind, i, segs, amts in keys:
        pts = []  # (t, rps) change points
        prev_v, prev_end = None, None
        for s, e_, v in segs:
            if not pts:
                pts.append((s, v))
            elif float_unerr(prev_end) < float_unerr(s):
                pts.append((prev_end, 0.0))
                pts.append((s, v))
            elif v != prev_v:
                pts.append((s, v))
            prev_v, prev_end = v, e_
        dd = {}
        for t, a in amts:
            dd[t] = a  # Pyfa keeps one amount per (key, time)
        at = sorted(dd)
        out.append((kind, i, pts, np.array(at), np.cumsum([dd[t] for t in at]) if at else np.array([])))
    c.memo[key] = (tmax, out)
    return out


def _rr_application(c, settings, d):
    """{(kind, item): factor array}"""
    lock = _lock_ok(c, settings, d) if d is not None else None
    app = {}
    for i in c.active_modules():
        if _rr_mod_amount(c, i) is None:
            continue
        if d is None:
            app[("mod", i)] = np.ones(1)
        else:
            app[("mod", i)] = unerr(np.where(lock, range_factor(c.max_range(i) or 0.0, c.falloff(i) or 0.0, d), 0.0))
    dcr = _dcr_ok(c, settings, d) if d is not None else None
    for i, n in c.active_drones():
        if not _rr_drone_amount(c, i, n):
            continue
        app[("drone", i)] = np.ones(1) if d is None else np.where(lock & dcr, 1.0, 0.0)
    return app


def _point_lookup(times, t):
    """index of the last entry with unerr(time) <= unerr(t) (vectorised over t), -1 when none"""
    if len(times) == 0:
        return np.full(len(t), -1)
    tu = unerr(np.asarray(times, float))
    return np.searchsorted(tu, unerr(t), side="right") - 1


def remote_reps(eng, req, c, xs, ys, params, settings, axis, tctx=None):
    x = np.asarray(xs, float)
    anc = params.get("anc_reload", True) is not False
    n = len(x)
    if axis == "distance_m":
        d, tq = x, params.get("time_s")
        bad = x < 0
    else:
        d, tq = params.get("distance_m"), None
        d = None if d is None else np.full(n, float(d))
        bad = (x < 0) | (x > 2500)
        tq = x
    app = _rr_application(c, settings, d)

    def A(k):
        a = app.get(k)
        return np.zeros(n) if a is None else (a if len(a) == n else np.full(n, a[0]))

    out = {}
    if tq is not None:
        tarr = np.full(n, float(tq)) if np.isscalar(tq) else tq
        tmax = float(np.max(np.where(bad, 0, tarr))) if n else 0.0
        cache = _rr_time_cache(c, anc, tmax)
        if "rps" in ys:
            tot = np.zeros(n)
            for kind, i, pts, at, cum in cache:
                if not pts:
                    continue
                k = _point_lookup([p[0] for p in pts], tarr)
                vals = np.array([p[1] for p in pts])
                tot += np.where(k >= 0, vals[np.maximum(k, 0)], 0.0) * A((kind, i))
            out["rps"] = tot
        if "total" in ys:
            tot = np.zeros(n)
            for kind, i, pts, at, cum in cache:
                if len(at) == 0:
                    continue
                k = _point_lookup(at, tarr)
                tot += np.where(k >= 0, cum[np.maximum(k, 0)], 0.0) * A((kind, i))
            out["total"] = tot
    else:
        tot = np.zeros(n)
        for i in c.active_modules():
            base = _rr_mod_amount(c, i)
            if base is None:
                continue
            e = c.effs(i)
            anc_any = "shipModuleAncillaryRemoteShieldBooster" in e or "shipModuleAncillaryRemoteArmorRepairer" in e
            cyc = c.cycle(i, anc if anc_any else None)
            if cyc is None or cyc.average == 0:
                continue
            rps = base[1] * _spool_mult(c, i, "default", None) / (cyc.average / 1000.0)
            tot += rps * A(("mod", i))
        for i, nn in c.active_drones():
            am = _rr_drone_amount(c, i, nn)
            cyc = c.drone_cycle_ms(i)
            if not am or not cyc:
                continue
            tot += sum(am.values()) / (cyc / 1000.0) * A(("drone", i))
        out["rps"] = tot
        out["total"] = np.full(n, NAN)
    if tctx is not None:  # contract 0.2 target fit: remoteRepairImpedance, disallowAssistance
        if tctx.g(tctx.ship, "disallowAssistance"):
            imp = 0.0
        else:
            imp = tctx.gopt(tctx.ship, "remoteRepairImpedance")
            imp = 1.0 if imp is None else float(imp)
        out = {k: v * imp for k, v in out.items()}
    return {k: np.where(bad, NAN, out[k]) for k in ys}
