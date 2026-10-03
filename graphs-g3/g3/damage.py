# SPDX-License-Identifier: LGPL-3.0-or-later
"""Damage graph (Pyfa "Damage Stats") and application profile, vectorised over the sample points.

Per source fit (memoised on its context): the damage dealers with their dps / volley maps and, lazily, the
cycle-by-cycle time cache. Per request: the target state per point (tackled speed, painted signature) and the
per-dealer application arrays; the series are sums of dealer value × application."""
import math

import numpy as np

from evedogma_g.stats import float_unerr, spoolup

from .common import NAN, range_factor, stack_mult, unerr
from .ctx import ACTIVE, INF, Ctx, Cycle, GraphError

SUPER_SINGLE = {"superWeaponAmarr", "superWeaponCaldari", "superWeaponGallente", "superWeaponMinmatar"}
DD_LOCK = SUPER_SINGLE | {"lightningWeapon"}
DD_WARN = {"doomsdayBeamDOT", "doomsdaySlash", "doomsdayConeDOT", "debuffLance"}
FTR_PREFIX = {"fighterAbilityMissiles": "fighterAbilityMissiles", "fighterAbilityLaunchBomb": "fighterAbilityLaunchBomb",
              "fighterAbilityAttackM": "fighterAbilityAttackMissile",
              "fighterAbilityEnergyNeutralizer": "fighterAbilityEnergyNeutralizer",
              "fighterAbilityStasisWebifier": "fighterAbilityStasisWebifier",
              "fighterAbilityWarpDisruption": "fighterAbilityWarpDisruption", "fighterAbilityECM": "fighterAbilityECM",
              "fighterAbilityEvasiveManeuvers": "fighterAbilityEvasiveManeuvers"}
FTR_HAS_CHARGES = {"fighterAbilityMissiles", "fighterAbilityLaunchBomb"}
FTR_SHOTS = {1: 0, 2: 12, 4: 6, 5: 3}
FTR_REARM = {1: 0, 2: 4000, 4: 6000, 5: 20000}


# ---------------------------------------------------------------- damage values
class Dmg:
    """em/th/ki/ex vector + breacher ticks {tick_key: [(absolute, relative)]} (Pyfa DmgTypes)"""
    __slots__ = ("v", "b")

    def __init__(self, v=(0.0, 0.0, 0.0, 0.0), b=None):
        self.v = tuple(v)
        self.b = b or {}

    def scaled(self, k):
        return Dmg(tuple(x * k for x in self.v), {t: [(a * k, r * k) for a, r in L] for t, L in self.b.items()})

    def plus(self, o):
        b = {t: list(L) for t, L in self.b.items()}
        for t, L in o.b.items():
            b.setdefault(t, []).extend(L)
        return Dmg(tuple(x + y for x, y in zip(self.v, o.v)), b)

    def total(self, hp=INF):
        pure = sum(max((min(a, r * hp) for a, r in L), default=0.0) for L in self.b.values())
        return sum(self.v) + pure

    def raw_total(self):
        return sum(self.v) + sum(max((a for a, r in L), default=0.0) for L in self.b.values())

    def key(self):
        return (tuple(float_unerr(x) for x in self.v), tuple(sorted(self.b)))


# ---------------------------------------------------------------- dealers of a fit
class Dealer:
    __slots__ = ("key", "kind", "item", "eff", "prefix", "dps", "volley", "cycle", "vparams", "breacher", "spool")


def _spool_mult(c, i, forced_cycles=None):
    mx, step = c.g(i, "damageMultiplierBonusMax"), c.g(i, "damageMultiplierBonusPerCycle")
    if not mx or not step:
        return 1.0
    if forced_cycles is not None:
        sp = {"type": "cycles", "amount": forced_cycles}
    else:
        sp = c.meta[i]["spool"] or {"type": "spool_scale", "amount": 1.0}
    return 1.0 + spoolup(mx, step, c.raw_cycle_ms(i) / 1000.0, sp)


def _module_base_volleys(c, i):
    """{delay_ms: Dmg} unspooled (Pyfa Module.getVolleyParameters base)"""
    g = c.g
    ch = c.charge(i)
    if ch is not None and "dotMissileLaunching" in c.effs(ch):
        n = math.floor(g(ch, "dotDuration") / 1000)
        info = (g(ch, "dotMaxDamagePerTick"), g(ch, "dotMaxHPPercentagePerTick") / 100)
        return {1 + k: Dmg(b={1 + k: [info]}) for k in range(n)}, True
    vec, kind = c.st.module_volley(i)
    e = c.effs(i)
    if e & DD_LOCK:
        delay = g(i, "damageDelayDuration")
    elif e & DD_WARN:
        delay = g(i, "doomsdayWarningDuration")
    else:
        delay = 0.0
    dur, sub = g(i, "doomsdayDamageDuration"), g(i, "doomsdayDamageCycleTime")
    n = math.floor(float_unerr(dur / sub)) if dur != 0 and sub != 0 and "doomsdaySlash" not in e else 1
    return {delay + sub * k: Dmg(vec) for k in range(n)}, False


def _ftr_cycle_ms(c, i, ab):
    p = FTR_PREFIX.get(ab)
    return c.g(i, p + "Duration") if p else 0.0


def _ftr_shots(c, i, ab):
    if ab not in FTR_HAS_CHARGES:
        return 0
    return FTR_SHOTS.get(int(c.g(i, "fighterSquadronRole")), 0) or 0


def _ftr_reload(c, i, ab, spent=None):
    shots = _ftr_shots(c, i, ab)
    spent = shots if spent is None else max(shots, spent)
    rearm = (FTR_REARM.get(int(c.g(i, "fighterSquadronRole")), 0) or 0) if ab in FTR_HAS_CHARGES else 0
    return c.g(i, "fighterRefuelingTime") + rearm * spent


def _ftr_all_abilities(c, i):
    en = c.ds.effect_name
    return [en.get(e) for e, _ in sorted(c.meta[i]["effects"]) if (en.get(e) or "").startswith("fighterAbility")]


def _ftr_cycles(c, i, factor_reload):
    """Pyfa Fighter.getCycleParametersPerEffect: {ability: Cycle}"""
    abil = _ftr_all_abilities(c, i)
    cyc = {a: _ftr_cycle_ms(c, i, a) for a in abil}
    if not factor_reload:
        return {a: Cycle(((cyc[a], 0.0, INF, False),), 1) for a in abil if cyc[a] > 0}
    limited = [a for a in abil if _ftr_shots(c, i, a) > 0 and cyc[a] > 0]
    if not limited:
        return {a: Cycle(((cyc[a], 0.0, INF, False),), 1) for a in abil if cyc[a] > 0}
    valid = [a for a in abil if cyc[a] > 0]
    most = min(limited, key=lambda a: cyc[a] * _ftr_shots(c, i, a))
    to_refuel = cyc[most] * _ftr_shots(c, i, most)
    until = {most: (_ftr_shots(c, i, most), None)}
    for a in valid:
        if a == most:
            continue
        full = int(float_unerr(to_refuel / cyc[a]))
        extra = float_unerr(to_refuel - full * cyc[a])
        until[a] = (full, None if extra == 0 else extra)
    refuel = max(_ftr_reload(c, i, a, until[a][0] + (1 if until[a][1] is not None else 0)) for a in valid)
    out = {}
    for a in valid:
        reg, extra = until[a]
        seq = []
        if extra is not None:
            if reg > 0:
                seq.append((cyc[a], 0.0, reg, False))
            seq.append((extra, refuel, 1, True))
        else:
            if reg - 1 > 0:
                seq.append((cyc[a], 0.0, reg - 1, False))
            seq.append((cyc[a], refuel, 1, True))
        out[a] = Cycle(tuple(seq), INF)
    return out


def _ftr_volley(c, i, ab, n):
    g = c.g
    p = FTR_PREFIX.get(ab)
    if p is None:
        return Dmg()
    deals = c.has(i, p + "DamageMultiplier") or c.charge(i) is not None
    if not deals:
        return Dmg()
    if p == "fighterAbilityLaunchBomb":
        ch = c.charge(i)
        v = [g(ch, a) if ch is not None else 0.0 for a in ("emDamage", "thermalDamage", "kineticDamage", "explosiveDamage")]
    else:
        v = [g(i, p + s) for s in ("DamageEM", "DamageTherm", "DamageKin", "DamageExp")]
    m = n * (g(i, p + "DamageMultiplier") if c.has(i, p + "DamageMultiplier") else 1.0)
    return Dmg([x * m for x in v])


def _ftr_optimized_cycles(c, i, n, factor_reload, active):
    inf = {a: Cycle(((_ftr_cycle_ms(c, i, a), 0.0, INF, False),), 1) for a in _ftr_all_abilities(c, i)
           if _ftr_shots(c, i, a) == 0 and _ftr_cycle_ms(c, i, a) > 0}
    rel = _ftr_cycles(c, i, factor_reload)

    def tot(cm):
        s = 0.0
        for a in active:
            if a in cm:
                v = _ftr_volley(c, i, a, n)
                if v.raw_total():
                    s += v.raw_total() / (cm[a].average / 1000.0)
        return s
    return inf if tot(inf) >= tot(rel) else rel


def dealers(c):
    d = c.memo.get("dealers")
    if d is not None:
        return d
    g = c.g
    fr = c.p["options"]["factor_reload"]
    out = []
    for i in c.active_modules():
        base, br = _module_base_volleys(c, i)
        if not any(v.raw_total() > 0 for v in base.values()):
            continue
        D = Dealer()
        D.key, D.item, D.breacher = ("mod", i), i, br
        e = c.effs(i)
        grp = c.group(i)
        if "ChainLightning" in e:
            D.kind = "vorton"
        elif "turretFitted" in e:
            D.kind = "turret"
        elif "launcherFitted" in e or c.meta[i]["type_id"] == 32461:
            D.kind = "missile"
        elif grp in ("Smart Bomb", "Structure Area Denial Module"):
            D.kind = "smartbomb"
        elif grp == "Missile Launcher Bomb":
            D.kind = "bomb"
        elif grp == "Structure Guided Bomb Launcher":
            D.kind = "guided_bomb"
        elif grp in ("Super Weapon", "Structure Doomsday Weapon"):
            D.kind = "doomsday"
        elif br:
            D.kind = "breacher"
        else:
            D.kind = "none"
        if br and D.kind == "missile":
            D.kind = "breacher"
        D.vparams = base
        sm = _spool_mult(c, i)
        vols = {k: v.scaled(sm) if sm != 1.0 else v for k, v in base.items()}
        D.volley = vols[min(vols)]
        cyc = c.cycle(i, fr)
        if cyc is None or cyc.average == 0:
            D.dps = Dmg()
        elif br:
            D.dps = D.volley
        else:
            s = Dmg()
            for v in vols.values():
                s = s.plus(v)
            D.dps = s.scaled(1 / (cyc.average / 1000))
        out.append(D)
    for i, n in c.active_drones():
        dm = g(i, "damageMultiplier") if c.has(i, "damageMultiplier") else 1.0
        src = c.charge(i) if c.charge(i) is not None else i
        names = ("emDamage", "thermalDamage", "kineticDamage", "explosiveDamage")
        if not any(c.has(i, a) or (c.charge(i) is not None and c.has(c.charge(i), a)) for a in names):
            continue
        vol = Dmg([g(src, a) * n * dm for a in names])
        if not vol.raw_total() > 0:
            continue
        D = Dealer()
        D.key, D.item, D.kind, D.breacher = ("drone", i), i, "drone", False
        D.volley = vol
        cyc = c.drone_cycle_ms(i)
        D.dps = vol.scaled(1 / (cyc / 1000)) if cyc else Dmg()
        D.vparams = {0: vol}
        out.append(D)
    for i, n in c.active_fighters():
        active = [a for a in c.fighter_abilities(i)]
        cm = _ftr_optimized_cycles(c, i, n, fr, active)
        for ab in active:
            vol = _ftr_volley(c, i, ab, n)
            if not vol.raw_total() > 0:
                continue
            D = Dealer()
            D.key, D.item, D.kind, D.eff, D.prefix, D.breacher = ("ftr", i, ab), i, "fighter", ab, FTR_PREFIX[ab], False
            D.volley = vol
            D.dps = vol.scaled(1 / (cm[ab].average / 1000)) if ab in cm else Dmg()
            D.vparams = {0: vol}
            out.append(D)
    c.memo["dealers"] = out
    return out


# ---------------------------------------------------------------- time cache
def time_cache(c, tmax):
    tc = c.memo.get("timecache")
    if tc is not None and tc[0] >= tmax:
        return tc[1]
    res = {}
    for D in dealers(c):
        i = D.item
        segs, dmg = [], {}
        segmemo, keep = {}, []  # keep: volley lists referenced by id() in segmemo keys stay alive
        if D.kind == "drone":
            cyc_ms = c.drone_cycle_ms(i)
            cyc = Cycle(((cyc_ms, 0.0, INF, False),), 1) if cyc_ms else None
        elif D.kind == "fighter":
            n = c.meta[i]["active_count"]
            cm = _ftr_optimized_cycles(c, i, n, True, list(c.fighter_abilities(i)))
            cyc = cm.get(D.eff)
        elif D.breacher:
            cyc = Cycle(((1000.0, 0.0, INF, False),), 1)
        else:
            cyc = c.cycle(i, True)
        if cyc is None:
            res[D.key] = ([], [], [])
            continue
        cur, nonstop = 0.0, 0
        # _spool_mult is 1.0 for every cycle unless both spool attributes are set
        spooling = D.key[0] == "mod" and bool(c.g(i, "damageMultiplierBonusMax")
                                             and c.g(i, "damageMultiplierBonusPerCycle"))
        rawnz = {}
        for act, ina, rl in cyc.iter():
            if D.kind == "mod":
                pass
            if spooling:
                sm = _spool_mult(c, i, nonstop)
                vols = {k: (v.scaled(sm) if sm != 1.0 else v) for k, v in D.vparams.items()}
            else:
                vols = D.vparams
            cv = []
            for k in sorted(vols) if False else vols:
                v = vols[k]
                cv.append(v)
                t = cur + k / 1000
                if D.breacher:
                    t += 1
                nz = rawnz.get(id(v))
                if nz is None:
                    nz = v.raw_total() != 0
                    if not spooling:
                        rawnz[id(v)] = nz
                if nz:
                    vv = Dmg(v.v, {t + kk: L for kk, L in v.b.items()})
                    dmg[t] = vv
                if D.breacher:
                    break
            ts, tf = cur, cur + act / 1000
            if D.breacher:
                ts, tf = ts + 1, tf + 1
            if cv:
                # identical (volleys, duration) pairs give identical segment values: build them once
                mk = (tuple(map(id, cv)), tf - ts)
                sv = segmemo.get(mk)
                if sv is None:
                    s = Dmg()
                    for v in cv:
                        s = s.plus(v)
                    if s.raw_total() > 0:
                        best = max(cv, key=lambda v: v.raw_total())
                        sd = s.scaled(1 / (tf - ts))
                        sv = (sd, best, sd.key(), best.key())
                    else:
                        sv = False
                    segmemo[mk] = sv
                    keep.append(cv)
                if sv:
                    segs.append((ts, tf, sv[0], sv[1], sv[2], sv[3]))
            if D.key[0] == "mod":
                nonstop = 0 if ina > 0 else nonstop + 1
            if cur > tmax:
                break
            cur += act / 1000 + ina / 1000
        # change points
        pts = []
        prev, prev_end = None, None
        for ts, tf, dps, vol, kd, kv in segs:
            if not pts:
                pts.append((ts, dps, vol))
            elif float_unerr(prev_end) < float_unerr(ts):
                pts.append((prev_end, Dmg(), Dmg()))
                pts.append((ts, dps, vol))
            elif kd != prev[0] or kv != prev[1]:
                pts.append((ts, dps, vol))
            prev, prev_end = (kd, kv), tf
        dts = sorted(dmg)
        res[D.key] = (pts, dts, [dmg[t] for t in dts])
    c.memo["timecache"] = (tmax, res)
    return res


def _lookup(times, t):
    if len(times) == 0:
        return np.full(len(t), -1)
    return np.searchsorted(unerr(np.asarray(times, float)), unerr(t), side="right") - 1


# ---------------------------------------------------------------- target
class Target:
    def __init__(self, eng, req, settings):
        t = req.get("target")
        self.fit = None
        self.layer = None
        if t is None:
            t = {"profile": {}}
        if not isinstance(t, dict):
            raise GraphError("BAD_REQUEST", "target must be an object", "/target")
        if t.get("fit") is not None:
            self.fit = eng.cache.get(t["fit"], path="/target/fit")
            self.fit_req = t["fit"]
            self.eng = eng
            g, s = self.fit.g, self.fit.ship
            self.vmax = g(s, "maxVelocity")
            self.sig = g(s, "signatureRadius")
            self.radius = g(s, "radius")
            mode = t.get("resist_mode") or "auto"
            self.res = self._resists(mode)
            st = self.fit.stats()
            hp = st["defense"]["hp"] if "hp" in st.get("defense", {}) else None
            self.hp = _fit_hp(self.fit)
        else:
            p = t.get("profile") or {}
            self.vmax = float(p.get("max_velocity") or 0.0)
            sg = p.get("signature_radius")
            self.sig = INF if sg is None else float(sg)
            self.radius = float(p.get("radius") or 0.0)
            self.res = tuple(float(p.get(k) or 0.0) for k in ("em", "thermal", "kinetic", "explosive"))
            hp = p.get("hp")
            self.hp = INF if hp is None else float(hp)

    def _resists(self, mode):
        f = self.fit
        g, s = f.g, f.ship

        def lay(pre):
            names = ("Em", "Thermal", "Kinetic", "Explosive")
            if pre:
                return tuple(1 - g(s, f"{pre}{n}DamageResonance") for n in names)
            return tuple(1 - g(s, f"{n[0].lower() + n[1:]}DamageResonance") for n in names)
        if mode == "shield":
            return lay("shield")
        if mode == "armor":
            return lay("armor")
        if mode == "hull":
            return lay("")
        hpd = _fit_hp_layers(f)
        if mode == "weighted_average":
            sh, ar, hu = lay("shield"), lay("armor"), lay("")
            tot = sum(hpd)
            out = []
            for k in range(4):
                ehp = hpd[0] / (1 - sh[k]) + hpd[1] / (1 - ar[k]) + hpd[2] / (1 - hu[k])
                out.append(1 - tot / ehp)
            return tuple(out)
        # auto
        shp, ahp, hhp = hpd
        lays = (lay("shield"), lay("armor"), lay(""))
        ehp = [h / (sum(0.25 * (1 - r) for r in L)) if True else 0 for h, L in zip(hpd, lays)]
        tot = sum(ehp)
        rf = [e / h if h else 1.0 for e, h in zip(ehp, hpd)]
        st = f.stats()
        tank = st["defense"]["tank"]
        raw = tank.get("raw") or tank.get("peak") or {}
        reps = [raw.get("shield_repair", 0.0), raw.get("armor_repair", 0.0), raw.get("hull_repair", 0.0)]
        regen = raw.get("passive_shield", 0.0)
        best = max(rf)
        sc = [100 * (e / tot) ** 1.5 + 25 * (r / best) ** 1.5 + 10000 * rp * r / tot for e, r, rp in zip(ehp, rf, reps)]
        sc[0] += 5000 * regen * rf[0] / tot
        m = max(sc)
        k = sc.index(m)
        self.layer = ("shield", "armor", "hull")[k]
        return lays[k]

    def disallow(self):
        return self.fit is not None and bool(self.fit.g(self.fit.ship, "disallowOffensiveModifiers"))

    def _variant(self, ignore_scram):
        if not ignore_scram or self.fit is None:
            return self.fit
        v = getattr(self, "_unscram", None)
        if v is None:
            v = self._unscram = self._build_variant()
        return v

    def _build_variant(self):
        req = dict(self.fit_req)
        mods = []
        for m in req.get("modules") or []:
            m2 = dict(m)
            if m2.get("state") in ("active", "overheated"):
                ti = self.eng.ds.tidx(m2.get("type_id"))
                if ti >= 0:
                    effs = {self.eng.ds.effect_name.get(e) for e, _ in self.eng.ds.t_effects[ti]}
                    if effs & {"moduleBonusMicrowarpdrive", "microJumpDrive", "microJumpPortalDrive"}:
                        m2["state"] = "online"
            mods.append(m2)
        req["modules"] = mods
        return self.eng.cache.get(req, tag="unscram:", path="/target/fit")

    def scrammables(self):
        if self.fit is None:
            return False
        for i in self.fit.active_modules():
            if self.fit.effs(i) & {"moduleBonusMicrowarpdrive", "microJumpDrive", "microJumpPortalDrive"}:
                return True
        return False

    def extended(self, attr, rows, ignore_scram):
        """attribute value with extra stacking-penalised multipliers (rows: list of (array mult, res attr id))"""
        base = {"maxVelocity": self.vmax, "signatureRadius": self.sig}[attr]
        if self.fit is None:
            m = stack_mult([r for r, _ in rows]) if rows else None
            return base if m is None else base * m
        f = self._variant(ignore_scram) if np.any(ignore_scram) else self.fit
        val = f.g(f.ship, attr)
        if not rows:
            return val
        adj = []
        for mult, rid in rows:
            rn = self.fit.ds.attr_name.get(rid) if rid else None
            rm = f.g(f.ship, rn) if rn else None
            if rm is None or rm == 1:
                adj.append(mult)
            else:
                adj.append((mult - 1) * rm + 1)
        own = _own_penalized(f, attr)
        if not own:
            return val * stack_mult(adj)
        # Pyfa getModifiedItemAttrExtended: the extra multipliers join the target's own stacking-penalised
        # multipliers of the attribute (one "default" group), e.g. TPs + core-defense-field-extender sig drawbacks
        n = len(adj[0])
        own_rows = [np.full(n, m) for m in own]
        return val / stack_mult([r[:1] for r in own_rows])[0] * stack_mult(own_rows + list(adj))


def _own_penalized(f, attr):
    """the fit ship's own stacking-penalised multipliers on `attr` (from the dogma engine's modifier table)"""
    key = ("ownpen", attr)
    if key in f.memo:
        return f.memo[key]
    from evedogma_g.dataset import ATTR_BITS, ATTR_MASK
    out = []
    hit = f.b.__dict__.get("_mtab")
    aid = f.ds.attr_by_name.get(attr)
    if hit is not None and aid is not None:
        M = hit[2]
        sel = np.nonzero((M["tgt"] == f.ship) & (M["attr"] == aid) & (M["pen"] != 0))[0]
        for j in sel.tolist():
            op, kind = int(M["op"][j]), int(M["kind"][j])
            if op not in (0, 1, 4, 5, 6):
                continue

            def val(k):
                k = int(k)
                return f.v.get(k >> ATTR_BITS, k & ATTR_MASK)
            if kind == 1:  # constant
                sv = float(M["const"][j])
            elif kind == 0:  # attribute
                sv = val(M["a"][j])
            elif kind == 3:  # projected (the target fit's own `projected` entries)
                fac = float(M["factor"][j]) * (val(M["c"][j]) if int(M["c"][j]) >= 0 else 1.0)
                pv = val(M["a"][j])
                sv = (pv - 1.0) * fac + 1.0 if M["mul"][j] else pv * fac
            else:
                continue
            m = sv if op in (0, 4) else (1.0 / sv if sv else 1.0) if op in (1, 5) else 1.0 + sv / 100.0
            if m != 1.0:
                out.append(m)
    f.memo[key] = out
    return out


def _fit_hp_layers(f):
    g, s = f.g, f.ship
    return g(s, "shieldCapacity"), g(s, "armorHP"), g(s, "hp")


def _fit_hp(f):
    return sum(_fit_hp_layers(f))


def _resist_id(c, i, effect_name):
    ds = c.ds
    eid = ds.effect_by_name.get(effect_name)
    rid = None
    if eid is not None:
        try:
            rid = ds.eff_info[eid].get("resistance_attr")
        except (KeyError, IndexError):
            rid = None
    if rid:
        return int(rid)
    p = FTR_PREFIX.get(effect_name)
    if p and c.meta[i]["kind"] == 6:
        r = int(c.g(i, p + "ResistanceID")) or int(c.g(i, p + "RemoteResistanceID")) or None
        return r
    return int(c.g(i, "remoteResistanceID")) or None


# ---------------------------------------------------------------- projected (webs / TPs / scram)
def _proj_data(c):
    d = c.memo.get("projdata")
    if d is not None:
        return d
    g = c.g
    web_m, tp_m, web_d, tp_d, web_f = [], [], [], [], []
    for i in c.active_modules():
        e = c.effs(i)
        rng, fo = c.max_range(i) or 0.0, c.falloff(i) or 0.0
        aoe = max(0.0, rng + g(i, "doomsdayAOERange"))
        for en in ("remoteWebifierFalloff", "structureModuleEffectStasisWebifier"):
            if en in e:
                web_m.append((g(i, "speedFactor"), rng, fo, _resist_id(c, i, en)))
        if "doomsdayAOEWeb" in e:
            web_m.append((g(i, "speedFactor"), aoe, fo, _resist_id(c, i, "doomsdayAOEWeb")))
        for en in ("remoteTargetPaintFalloff", "structureModuleEffectTargetPainter"):
            if en in e:
                tp_m.append((g(i, "signatureRadiusBonus"), rng, fo, _resist_id(c, i, en)))
        if "doomsdayAOEPaint" in e:
            tp_m.append((g(i, "signatureRadiusBonus"), aoe, fo, _resist_id(c, i, "doomsdayAOEPaint")))
    for i, n in c.active_drones():
        e = c.effs(i)
        rng, fo = _drone_range(c, i), _drone_falloff(c, i)
        if "remoteWebifierEntity" in e:
            web_d.extend([(g(i, "speedFactor"), rng, fo, _resist_id(c, i, "remoteWebifierEntity"),
                           g(i, "maxVelocity"), g(i, "radius"))] * n)
        if "remoteTargetPaintEntity" in e:
            tp_d.extend([(g(i, "signatureRadiusBonus"), rng, fo, _resist_id(c, i, "remoteTargetPaintEntity"),
                          g(i, "maxVelocity"), g(i, "radius"))] * n)
    for i, n in c.active_fighters():
        if "fighterAbilityStasisWebifier" in c.fighter_abilities(i):
            web_f.append((g(i, "fighterAbilityStasisWebifierSpeedPenalty") * n,
                          g(i, "fighterAbilityStasisWebifierOptimalRange"),
                          g(i, "fighterAbilityStasisWebifierFalloffRange"),
                          _resist_id(c, i, "fighterAbilityStasisWebifier"), g(i, "maxVelocity"), g(i, "radius")))
    scram = None
    for i in c.active_modules():
        e = c.effs(i)
        reg = bool(e & {"warpScrambleBlockMWDWithNPCEffect", "structureWarpScrambleBlockMWDWithNPCEffect"}) and \
            bool(c.g(i, "activationBlockedStrenght"))
        ch = c.charge(i)
        hic = "warpDisruptSphere" in e and ch is not None and "shipModuleFocusedWarpScramblingScript" in c.effs(ch)
        if reg or hic:
            scram = max(scram or 0.0, c.max_range(i) or 0.0)
    d = c.memo["projdata"] = (web_m, tp_m, web_d, tp_d, web_f, scram)
    return d


def _drone_range(c, i):
    for a in ("shieldTransferRange", "powerTransferRange", "energyDestabilizationRange", "empFieldRange",
              "ecmBurstRange", "maxRange"):
        x = c.gopt(i, a)
        if x is not None:
            return x
    ch = c.charge(i)
    if ch is not None:
        dl, sp = c.gopt(ch, "explosionDelay"), c.gopt(ch, "maxVelocity")
        if dl is not None and sp is not None:
            return dl / 1000.0 * sp
    return 0.0


def _drone_falloff(c, i):
    for a in ("falloff", "falloffEffectiveness"):
        x = c.gopt(i, a)
        if x is not None:
            return x
    return 0.0


def _rf(opt, fall, d, n, restricted=True):
    if d is None:
        return np.ones(n)
    return range_factor(opt, fall, d, restricted)


def tackle(c, tgt, settings, d, n, speed):
    """(tgt speed array, sig multiplier array) after the source's webs / TPs / scram (Pyfa getTackledSpeed and
    getSigRadiusMult), vectorised over points; d = distance array or None"""
    speed = np.broadcast_to(np.asarray(speed, float), (n,)).copy()
    if not settings["apply_projected"]:
        return speed, np.ones(n)
    web_m, tp_m, web_d, tp_d, web_f, scram = _proj_data(c)
    lock = np.ones(n, bool) if (d is None or settings["ignore_lock_range"]) else d <= c.g(c.ship, "maxTargetRange")
    dcr = np.ones(n, bool) if (d is None or settings["ignore_drone_control_range"]) else d <= c.drone_control_range()
    if scram is None:
        scr = np.zeros(n, bool)
    else:
        scr = lock & (np.ones(n, bool) if d is None else d <= scram)
    scr = scr & tgt.scrammables()
    mode = settings["mobile_drone_mode"]
    atk_r = c.g(c.ship, "radius")
    out_v, out_s = speed.copy(), np.ones(n)
    if tgt.disallow():
        return out_v, out_s
    scr = np.asarray(scr, bool)
    groups = [(ign, np.flatnonzero(scr == ign)) for ign in (False, True)]
    for ign, idx in groups:
        if len(idx) == 0:
            continue
        m = len(idx)
        dd = None if d is None else d[idx]
        lk, dc = lock[idx], dcr[idx]
        # ---- speed
        vmax0 = tgt.extended("maxVelocity", [], ign)
        if tgt.vmax == 0:
            tv = np.zeros(m)
        else:
            ratio = speed[idx] / tgt.vmax
            rows = []
            for boost, o, f, rid in web_m:
                ab = boost * _rf(o, f, dd, m)
                rows.append((np.where(lk & (ab != 0), 1 + ab / 100, 1.0), rid))
            mobile = []
            if web_f or web_d:
                for w in web_f:
                    mobile.append((w, lk))
                for w in web_d:
                    mobile.append((w, lk & dc))
            if not mobile:
                tv = tgt.extended("maxVelocity", rows, ign) * ratio
            else:
                tv = np.empty(m)
                for j in range(m):
                    rj = [(np.array([r[0][j]]), r[1]) for r in rows]
                    mws = [w for w, ok in mobile if ok[j]]
                    dj = None if dd is None else float(dd[j])
                    longe = [w for w in mws if dj is None or dj <= w[1] - atk_r + w[5]]
                    for w in longe:
                        rj.append((np.array([1 + w[0] / 100]), w[3]))
                        mws.remove(w)
                    cur = _scalar(tgt.extended("maxVelocity", rj, ign) * ratio[j]) if rj else _scalar(vmax0 * ratio[j])
                    while mws:
                        fast = max(w[4] for w in mws)
                        for w in [w for w in mws if w[4] == fast]:
                            if (mode == "auto" and w[4] >= cur) or mode == "follow_target":
                                b = w[0]
                            else:
                                rd = None if dj is None else dj + atk_r - w[5]
                                b = w[0] * (1.0 if rd is None else float(range_factor(w[1], w[2], np.array([rd]))[0]))
                            rj.append((np.array([1 + b / 100]), w[3]))
                            mws.remove(w)
                        cur = _scalar(tgt.extended("maxVelocity", rj, ign) * ratio[j])
                    tv[j] = cur
            tv = unerr(np.broadcast_to(tv, (m,)))
        out_v[idx] = tv
        # ---- signature
        rows = []
        for boost, o, f, rid in tp_m:
            ab = boost * _rf(o, f, dd, m)
            rows.append((np.where(lk & (ab != 0), 1 + ab / 100, 1.0), rid))
        for w, ok in [(w, lk) for w in []] + [(w, lk & dc) for w in tp_d]:
            if (mode == "auto" and w[4] >= 0) and False:
                pass
            fast = (mode == "follow_target") | ((mode == "auto") & (w[4] >= tv))
            rd = None if dd is None else dd + atk_r - w[5]
            rfv = np.ones(m) if rd is None else range_factor(w[1], w[2], rd)
            b = np.where(fast, w[0], w[0] * rfv)
            rows.append((np.where(ok, 1 + b / 100, 1.0), w[3]))
        init = tgt.sig
        mod = tgt.extended("signatureRadius", rows, ign)
        mod = np.broadcast_to(np.asarray(mod, float), (m,))
        with np.errstate(divide="ignore", invalid="ignore"):
            if math.isinf(init):
                sm = np.ones(m)
            else:
                sm = unerr(mod / init)
        out_s[idx] = sm
    return out_v, out_s


# ---------------------------------------------------------------- application
def _turret_mult(cth):
    wreck = np.minimum(cth, 0.01)
    normal = cth - wreck
    avg = (0.01 + cth) / 2 + 0.49
    return np.where(normal > 0, normal * avg, 0.0) + wreck * 3


def _cth(atk_speed, atk_angle, atk_r, opt, fall, tracking, osr, d, tv, tgt_angle, tgt_r, sig, n):
    with np.errstate(divide="ignore", invalid="ignore", over="ignore"):
        if d is None:
            ang = np.zeros(n)
            rfv = np.ones(n)
        else:
            trans = np.abs(atk_speed * math.sin(atk_angle * math.pi / 180) - tv * math.sin(tgt_angle * math.pi / 180))
            ctc = atk_r + d + tgt_r
            ang = np.where(ctc == 0, np.where(trans == 0, 0.0, INF), trans / np.where(ctc == 0, 1, ctc))
            rfv = range_factor(opt, fall, d, restricted=False)
        x = (ang * osr) / (tracking * sig)
        x = np.where(np.isnan(x), 0.0, x)
        tf = 0.5 ** (x ** 2)
    return rfv * tf


def _missile_factor(er, ev, drf, tv, sig):
    with np.errstate(divide="ignore", invalid="ignore", over="ignore"):
        f = np.ones_like(np.asarray(sig, float) * 1.0)
        if er > 0:
            f = np.minimum(f, sig / er)
        fast = ((ev * sig) / (er * tv)) ** drf
        f = np.where(tv > 0, np.minimum(f, fast), f)
    return f


def application(c, tgt, settings, D, d, tv, sig, n, atk_speed, atk_angle, tgt_angle):
    g = c.g
    i = D.item
    lock = np.ones(n, bool) if (d is None or settings["ignore_lock_range"]) else d <= c.g(c.ship, "maxTargetRange")
    dcr = np.ones(n, bool) if (d is None or settings["ignore_drone_control_range"]) else d <= c.drone_control_range()
    k = D.kind
    atk_r = g(c.ship, "radius")
    if k == "vorton":
        a = _rf(g(i, "maxRange"), 0, d, n) * _missile_factor(g(i, "aoeCloudSize"), g(i, "aoeVelocity"),
                                                            g(i, "aoeDamageReductionFactor"), tv, sig)
        a = np.where(lock, a, NAN)  # Pyfa: no entry when out of lock range (counts as 0)
        a = np.where(np.isnan(a), 0.0, a)
    elif k == "turret":
        a = _turret_mult(_cth(atk_speed, atk_angle, atk_r, c.max_range(i) or 0.0, c.falloff(i) or 0.0,
                              g(i, "trackingSpeed"), g(i, "optimalSigRadius"), d, tv, tgt_angle, tgt.radius, sig, n))
        a = np.where(lock, a, 0.0)
    elif k == "missile":
        mrd = c.missile_range_data(i)
        if mrd is None:
            a = np.zeros(n)
        else:
            lo, hi, hc = mrd
            if d is None:
                df = np.ones(n)
            else:
                df = np.where(d <= lo, 1.0, np.where(d <= hi, hc, 0.0))
            ch = c.charge(i)
            a = df * _missile_factor(g(ch, "aoeCloudSize"), g(ch, "aoeVelocity"), g(ch, "aoeDamageReductionFactor"),
                                     tv, sig)
            fof = ch is not None and "fofMissileLaunching" in c.effs(ch)
            if not fof:
                a = np.where(lock, a, 0.0)
    elif k == "breacher":
        mrd = c.missile_range_data(i)
        if mrd is None:
            a = np.zeros(n)
        else:
            lo, hi, hc = mrd
            df = np.ones(n) if d is None else np.where(d <= lo, 1.0, np.where(d <= hi, hc, 0.0))
            rm = 1.0
            if tgt.fit is not None:
                rm = _ship_attr_or(tgt.fit, "breacherPodDamageResistance", 1.0)
            a = np.where(lock, df * rm, 0.0)
    elif k == "smartbomb":
        r = c.max_range(i)
        a = np.zeros(n) if r is None else (np.ones(n) if d is None else np.where(d > r, 0.0, 1.0))
    elif k == "bomb":
        r = c.max_range(i)
        if r is None:
            a = np.zeros(n)
        else:
            ch = c.charge(i)
            br = g(ch, "explosionRange")
            bf = _bomb_factor(g(ch, "aoeCloudSize"), sig)
            if d is None:
                a = bf * np.ones(n)
            else:
                lo = max(0.0, r - atk_r - tgt.radius - br)
                hi = max(0.0, r - atk_r + tgt.radius + br)
                a = np.where((d < lo) | (d > hi), 0.0, bf)
    elif k == "guided_bomb":
        r = c.max_range(i)
        if r is None:
            a = np.zeros(n)
        else:
            er = g(c.charge(i), "aoeCloudSize")
            f = np.ones(n) if er == 0 else np.minimum(1.0, sig / er)
            a = f if d is None else np.where(d > r - atk_r, 0.0, f)
            a = np.where(lock, a, 0.0)
    elif k == "doomsday":
        r = c.max_range(i)
        e = c.effs(i)
        a = np.ones(n)
        if d is not None and r:
            a = np.where(d > r, 0.0, a)
        cap_tgt = True
        if e & SUPER_SINGLE and tgt.fit is not None:
            cap_tgt = _requires_skill(tgt.fit, tgt.fit.ship, "Capital Ships")
        ds_ = g(i, "signatureRadius")
        if not cap_tgt:
            a = np.zeros(n)
        elif ds_:
            a = a * np.minimum(1.0, sig / ds_)
        if e & DD_LOCK:
            a = np.where(lock, a, 0.0)
    elif k == "drone":
        a = _drone_mult(c, tgt, settings, i, d, tv, sig, n, atk_speed, atk_angle, tgt_angle)
        a = np.where(lock & dcr, a, 0.0)
    elif k == "fighter":
        a = _fighter_mult(c, tgt, settings, D, d, tv, sig, n)
        if D.eff != "fighterAbilityLaunchBomb":
            a = np.where(lock, a, 0.0)
    else:
        return np.zeros(n)
    return unerr(a)


def _scalar(v):
    """float of a scalar or 1-element array (NumPy 2.5 refuses float() of a 1-d array)"""
    return float(np.ravel(v)[0])


def _ship_attr_or(f, name, default):
    """Pyfa ship.getModifiedItemAttr(name, default): a type without the attribute still gets the attribute's SDE
    default value (e.g. fighterAbilityAntiCapitalMissileResistance 0.1 on sub-capitals); `default` only when the
    attribute is unknown"""
    if f.has(f.ship, name):
        return f.g(f.ship, name)
    aid = f.ds.attr_by_name.get(name)
    if aid is None:
        return default
    v = f.v.get(f.ship, aid)
    return default if v is None else v


def _bomb_factor(er, sig):
    return np.ones_like(np.asarray(sig, float)) if er == 0 else np.minimum(1.0, sig / er)


def _requires_skill(f, i, skill_name):
    ds = f.ds
    ti = f.meta[i]["ti"]
    d = ds._tad_get(ti)
    from .ctx import _type_id_by_name
    sid = _type_id_by_name(ds, skill_name)
    for a in ("requiredSkill1", "requiredSkill2", "requiredSkill3", "requiredSkill4", "requiredSkill5", "requiredSkill6"):
        aid = ds.attr_by_name.get(a)
        if aid is not None and int(d.get(aid, 0) or 0) == sid:
            return True
    return False


def _drone_mult(c, tgt, settings, i, d, tv, sig, n, atk_speed, atk_angle, tgt_angle):
    g = c.g
    if d is not None:
        bad = np.zeros(n, bool)
        if not settings["ignore_drone_control_range"]:
            bad |= d > c.drone_control_range()
        if not settings["ignore_lock_range"]:
            bad |= d > g(c.ship, "maxTargetRange")
    else:
        bad = np.zeros(n, bool)
    dsp = g(i, "maxVelocity")
    mode = settings["mobile_drone_mode"]
    follow = np.zeros(n, bool)
    if dsp > 1:
        if mode == "follow_target":
            follow[:] = True
        elif mode == "auto":
            follow = dsp >= tv
    dr = g(i, "radius")
    cd = None if d is None else d + g(c.ship, "radius") - dr
    cth = _cth(min(atk_speed, dsp), atk_angle, dr, _drone_range(c, i), _drone_falloff(c, i), g(i, "trackingSpeed"),
               g(i, "optimalSigRadius"), cd, tv, tgt_angle, tgt.radius, sig, n)
    cth = np.where(follow, 1.0, cth)
    return np.where(bad, 0.0, _turret_mult(cth))


def _fighter_mult(c, tgt, settings, D, d, tv, sig, n):
    g = c.g
    i, p = D.item, D.prefix
    if p == "fighterAbilityLaunchBomb":
        ch = c.charge(i)
        return _bomb_factor(g(ch, "aoeCloudSize") if ch is not None else 0.0, sig)
    fsp = g(i, "maxVelocity")
    mode = settings["mobile_drone_mode"]
    follow = np.full(n, mode == "follow_target") | ((mode == "auto") & (fsp >= tv))
    rd = None if d is None else d + g(c.ship, "radius") - g(i, "radius")
    opt = g(i, p + "RangeOptimal") or g(i, p + "Range")
    rfv = np.ones(n) if rd is None else range_factor(opt, g(i, p + "RangeFalloff"), rd)
    rfv = np.where(follow, 1.0, rfv)
    drf = c.gopt(i, p + "ReductionFactor")
    if drf is None:
        drf = g(i, p + "DamageReductionFactor")
    drs = c.gopt(i, p + "ReductionSensitivity")
    if drs is None:
        drs = g(i, p + "DamageReductionSensitivity")
    agg = math.log(drf) / math.log(drs)
    mf = _missile_factor(g(i, p + "ExplosionRadius"), g(i, p + "ExplosionVelocity"), agg, tv, sig)
    rm = 1.0
    if tgt.fit is not None:
        rid = g(i, p + "ResistanceID")
        if rid:
            nm = c.ds.attr_name.get(int(rid))
            if nm is not None:
                rm = _ship_attr_or(tgt.fit, nm, 1.0)
    return rfv * mf * rm


# ---------------------------------------------------------------- series assembly
def _apply(maps, apps, tgt, settings, n):
    """maps: list of (dealer key, Dmg or per-point _PP); returns total array"""
    res = (0.0, 0.0, 0.0, 0.0) if settings["ignore_resists"] else tgt.res
    tot = np.zeros(n)
    ticks = {}
    gpos, gcols = {}, []
    for key, dm in maps:
        a = apps.get(key)
        if a is None:
            continue
        if isinstance(dm, Dmg):
            tot += sum(x * (1 - r) for x, r in zip(dm.v, res)) * a
            for t, L in dm.b.items():
                for ab, rl in L:
                    val = np.minimum(ab * a, rl * a * tgt.hp)
                    ticks[t] = np.maximum(ticks[t], val) if t in ticks else val
        else:
            if n == 0:  # empty x.values: nothing to evaluate (empty series, see DESIGN.md)
                continue
            tot = _pp_apply(dm, np.broadcast_to(np.asarray(a, float), (n,)), tuple(res), tgt.hp, n, tot, gpos,
                            gcols)
    for v in ticks.values():
        tot = tot + v
    if gcols:
        G = np.zeros((n, len(gpos)))
        for dm, a, vis, used, gi in gcols:
            _pp_ticks(dm, a, vis, used, gi, tgt.hp, G)
        # tot + col_0 + col_1 + ... in first-visit order (sequential accumulate == the scalar loop)
        tot = np.add.accumulate(np.concatenate([tot[:, None], G], axis=1), axis=1)[:, -1]
    return tot


class _Entries:
    """time-cache column of one dealer prepared for array evaluation: entry 4-vectors, unerr'd change times,
    resisted vectors per resist profile and breacher tick matrices (built once per time cache).
    cum=True: entry e is the running sum of increments 0..e (same float operations as chaining Dmg.plus)."""
    __slots__ = ("vs", "bs", "cum", "n", "times", "_res", "_ticks")

    def __init__(self, times, dmgs, cum=False):
        self.cum = cum
        if cum:
            vs, acc = [], (0.0, 0.0, 0.0, 0.0)
            for m in dmgs:
                acc = tuple(x + y for x, y in zip(acc, m.v))
                vs.append(acc)
            self.vs = vs
        else:
            self.vs = [m.v for m in dmgs]
        self.bs = [m.b for m in dmgs]
        self.n = len(dmgs)
        self.times = unerr(np.asarray(times, float)) if len(times) else None
        self._res = {}
        self._ticks = None

    def lookup(self, tq, tqu=None):
        if self.times is None:
            return np.full(len(tq), -1)
        return np.searchsorted(self.times, unerr(tq) if tqu is None else tqu, side="right") - 1

    def resisted(self, res):
        R = self._res.get(res)
        if R is None:
            R = self._res[res] = np.array([[x * (1 - r) for x, r in zip(v, res)] for v in self.vs],
                                          float).reshape(self.n, 4)
        return R

    def ticks(self):
        """(has_b per entry, ordered tick keys per entry, tick key list, AB, RL, present) or None"""
        if self._ticks is None:
            self._ticks = (self._cum_ticks() if self.cum else self._plain_ticks()) or False
        return self._ticks or None

    def _plain_ticks(self):
        hasb = np.array([bool(b) for b in self.bs], bool)
        if not hasb.any():
            return None
        keys, pos = [], {}
        lmax = 1
        for b in self.bs:
            for t, L in b.items():
                if t not in pos:
                    pos[t] = len(keys)
                    keys.append(t)
                lmax = max(lmax, len(L))
        E, T = self.n, len(keys)
        AB, RL = np.zeros((E, T, lmax)), np.zeros((E, T, lmax))
        PR = np.zeros((E, T, lmax), bool)
        for e, b in enumerate(self.bs):
            for t, L in b.items():
                ti = pos[t]
                for l, (ab, rl) in enumerate(L):
                    AB[e, ti, l], RL[e, ti, l], PR[e, ti, l] = ab, rl, True
        ekeys = [[pos[t] for t in b] for b in self.bs]
        return hasb, ekeys, keys, AB, RL, PR

    def _cum_ticks(self):
        # running sum e holds every tick key seen in increments 0..e (first-appearance order) with the
        # concatenated (absolute, relative) lists: build it incrementally instead of materialising every sum
        keys, pos, cnt, items, nk = [], {}, [], [], []
        for e, b in enumerate(self.bs):
            for t, L in b.items():
                if t not in pos:
                    pos[t] = len(keys)
                    keys.append(t)
                    cnt.append(0)
                ti = pos[t]
                for ab, rl in L:
                    items.append((e, ti, cnt[ti], ab, rl))
                    cnt[ti] += 1
            nk.append(len(keys))
        if not keys:
            return None
        E, T = self.n, len(keys)
        lmax = max([1] + cnt)
        AB, RL = np.zeros((E, T, lmax)), np.zeros((E, T, lmax))
        PR = np.zeros((E, T, lmax), bool)
        for e, ti, l, ab, rl in items:
            AB[e:, ti, l], RL[e:, ti, l], PR[e:, ti, l] = ab, rl, True
        hasb = np.array([k > 0 for k in nk], bool)
        ekeys = [list(range(k)) for k in nk]
        return hasb, ekeys, keys, AB, RL, PR


class _PP:
    """per-point view: dealer entries + entry index per point (-1 = none)"""
    __slots__ = ("E", "k")

    def __init__(self, E, k):
        self.E, self.k = E, k


def _prepared(c, tc):
    pc = c.memo.get("ppcache")
    if pc is None or pc[0] is not tc:
        pc = c.memo["ppcache"] = (tc, {})
    return pc[1]


def _per_point_maps(c, tq, n, what):
    """time-cache value per dealer for an array of times: list of (key, _PP)"""
    tmax = float(np.nanmax(tq)) if n else 0.0
    tc = time_cache(c, tmax)
    prep = _prepared(c, tc)
    out = []
    tqu = None
    for key, (pts, dts, dmgs) in tc.items():
        mode = "pts" + what if what in ("dps", "volley") else "cum"
        E = prep.get((key, mode))
        if E is None:
            if mode != "cum":
                col = 1 if what == "dps" else 2
                E = _Entries([p[0] for p in pts], [p[col] for p in pts])
            else:
                E = _Entries(dts, dmgs, cum=True)
            prep[(key, mode)] = E
        if tqu is None:
            tqu = unerr(tq)
        out.append((key, _PP(E, E.lookup(tq, tqu))))
    return out


def _sel(idx, size):
    """slice for a contiguous ascending index list (fast path), else the index array"""
    if idx and idx[-1] - idx[0] == len(idx) - 1 and (len(idx) == 1 or idx == list(range(idx[0], idx[-1] + 1))):
        return slice(idx[0], idx[-1] + 1)
    return np.asarray(idx)


def _pp_ticks(dm, a, vis, used, gi, hp, G):
    """running max of max_L min(ab*a, rl*a*hp) into the global tick columns gi (Python min/max semantics)"""
    _, _, _, AB, RL, PR = dm.E.ticks()
    kk = np.where(vis, dm.k, 0)
    T = AB.shape[1]
    us = _sel(used, T)
    ab, pr = AB[kk][:, us], PR[kk][:, us]
    pr &= vis[:, None, None]
    a3 = a[:, None, None]
    x = ab * a3
    if hp == INF:
        v = x  # rl*a*inf is inf or nan: never < x, Python min keeps x
    else:
        y = RL[kk][:, us] * a3 * hp
        v = np.where(y < x, y, x)  # Python min(x, y)
    best, present = v[:, :, 0], pr[:, :, 0]
    for l in range(1, v.shape[2]):
        best = np.where(pr[:, :, l] & (v[:, :, l] > best), v[:, :, l], best)
        present = present | pr[:, :, l]
    gs = _sel(gi, G.shape[1])
    col = G[:, gs]
    G[:, gs] = np.where(present & (best > col), best, col)


def _pp_apply(dm, a, res, hp, n, tot, gpos, gcols):
    """vectorised per-point accumulation; same operations and tick order as the scalar definition:
    tick arrays are created in first-visit order (dealer order, then point order, then tick order) and each holds
    the running max of max_L min(ab*a, rl*a*hp) (Python min/max comparison semantics)"""
    E, k = dm.E, dm.k
    valid = k >= 0
    kk = np.where(valid, k, 0)
    if E.n:
        R = E.resisted(res)
        vec = np.where(valid[:, None], R[kk], 0.0)
    else:
        vec = np.zeros((n, 4))
    tot += vec.sum(axis=1) * a
    tk = E.ticks()
    if tk is None:
        return tot
    hasb, ekeys, keys, AB, RL, PR = tk
    vis = valid & hasb[kk]
    if not vis.any():
        return tot
    vk = k[vis]
    _, first = np.unique(vk, return_index=True)
    order = vk[np.sort(first)].tolist()
    used = []
    seen = set()
    for e in order:
        for ti in ekeys[e]:
            if ti not in seen:
                seen.add(ti)
                used.append(ti)
                g = (keys[ti], "pp")
                if g not in gpos:
                    gpos[g] = len(gpos)
    gcols.append((dm, a, vis, used, [gpos[(keys[ti], "pp")] for ti in used]))
    return tot


def _speed_param(params, prefix, vmax):
    a = params.get(prefix + "_speed_mps")
    if a is not None:
        return float(a)
    p = params.get(prefix + "_speed_pct")
    if p is None:
        p = 100.0 if prefix == "tgt" else 0.0
    return float(p) / 100.0 * vmax


def run(eng, req, c, xs, ys, params, settings, gname, axis):
    if gname == "application_profile":
        from . import appprof
        return appprof.run(eng, req, c, xs, ys, params, settings)
    tgt = Target(eng, req, settings)
    return compute(eng, c, tgt, xs, ys, params, settings, axis)


def compute(eng, c, tgt, xs, ys, params, settings, axis, dealer_list=None):
    n = len(xs)
    x = np.asarray(xs, float)
    atk_speed = _speed_param(params, "atk", c.g(c.ship, "maxVelocity"))
    atk_angle = float(params.get("atk_angle_deg") if params.get("atk_angle_deg") is not None else 90.0)
    tgt_angle = float(params.get("tgt_angle_deg") if params.get("tgt_angle_deg") is not None else 90.0)
    tspeed = _speed_param(params, "tgt", tgt.vmax)
    dpar = params.get("distance_m")
    tpar = params.get("time_s")
    bad = np.zeros(n, bool)
    if axis == "distance_m":
        d = x
        bad = x < 0
        tv, sm = tackle(c, tgt, settings, d, n, tspeed)
        sig = tgt.sig * sm
    else:
        d = None if dpar is None else np.full(n, float(dpar))
        if axis in ("tgt_speed_mps", "tgt_speed_pct"):
            xv = x if axis == "tgt_speed_mps" else x / 100.0 * tgt.vmax  # Pyfa ('tgtSpeed', '%') normaliser
            tv, sm = tackle(c, tgt, settings, d, n, xv)
            sig = tgt.sig * sm
            bad = x < 0
        elif axis in ("tgt_sig_m", "tgt_sig_pct"):
            if axis == "tgt_sig_m":
                xs_ = x
                bad = x <= 0
            else:  # % of the target's signature; infinite-signature target -> null everywhere
                xs_ = x / 100.0 * tgt.sig if math.isfinite(tgt.sig) else np.full(n, NAN)
                bad = (x <= 0) | ~np.isfinite(xs_)
            tv, sm = tackle(c, tgt, settings, d, n, tspeed)
            sig = np.where(bad, 1.0, xs_) * sm
        else:  # time
            tv, sm = tackle(c, tgt, settings, d, n, tspeed)
            sig = tgt.sig * sm
            bad = (x < 0) | (x > 2500)
    dl = dealers(c) if dealer_list is None else dealer_list
    apps = {D.key: application(c, tgt, settings, D, d, tv, sig, n, atk_speed, atk_angle, tgt_angle) for D in dl}
    out = {}
    tq = None
    if axis == "time_s":
        tq = np.where(bad, 0.0, x)
    elif tpar is not None:
        tq = np.full(n, min(2500.0, max(0.0, float(tpar))))  # contract: time_s param clamped to 0 .. 2500
    for y in ys:
        if tq is None:
            if y == "damage":
                out[y] = np.full(n, NAN)
                continue
            maps = [(D.key, D.dps if y == "dps" else D.volley) for D in dl]
        else:
            maps = _per_point_maps(c, tq, n, y)
        out[y] = _apply(maps, apps, tgt, settings, n)
    res = {k: np.where(bad, NAN, v) for k, v in out.items()}
    if tgt.layer:
        res["_meta"] = {"target_resist_layer": tgt.layer}
    return res
