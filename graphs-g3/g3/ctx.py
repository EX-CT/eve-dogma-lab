# SPDX-License-Identifier: LGPL-3.0-or-later
"""Per-fit evaluation context: one variant-g calc of a FitRequest plus the attribute helpers the graphs need.

A context is a pure function of (dataset, canonical FitRequest); `FitCache` memoises contexts (and everything
derived from them: dealer tables, time caches, capacitor histories) by the canonical JSON of the request."""
import json
import math
from collections import OrderedDict

from evedogma_g import capsim, engine
from evedogma_g import stats as gstats
from evedogma_g.calc import _batch
from evedogma_g.request import RequestError, parse
from evedogma_g.stats import FitStats, float_unerr

ACTIVE, ONLINE = engine.ACTIVE, engine.ONLINE
INF = math.inf


class GraphError(Exception):
    def __init__(self, code, message, path=""):
        super().__init__(message)
        self.code, self.message, self.path = code, message, path


def canon(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


class Cycle:
    """Pyfa cycle parameters: a sequence of (active_ms, inactive_ms, count, is_inactivity_reload) segments,
    repeated `repeat` times (math.inf = forever)."""
    __slots__ = ("seq", "repeat")

    def __init__(self, seq, repeat):
        self.seq, self.repeat = seq, repeat

    @property
    def average(self):
        n = sum(s[2] for s in self.seq)
        t = sum((s[0] + s[1]) * s[2] for s in self.seq)
        if n == INF:
            s = self.seq[0]
            return s[0] + s[1]
        return t / n

    def iter(self):
        k = 0
        while k < self.repeat:
            for a, ina, cnt, rl in self.seq:
                j = 0
                while j < cnt:
                    yield a, ina, rl
                    j += 1
            k += 1


class Ctx:
    def __init__(self, ds, req, path="/fit"):
        try:
            p = parse(req)
        except RequestError as e:
            raise GraphError(e.code, e.message, path + (e.path or ""))
        batch, vals, fits = _batch(ds, [p])
        f = fits[0]
        if isinstance(f, RequestError):
            raise GraphError(f.code, f.message, path + (f.path or ""))
        self.ds, self.b, self.v, self.f, self.req, self.p = ds, batch, vals, f, req, p
        self.meta = batch.meta
        self.st = FitStats(batch, f, vals)
        self.ship, self.char = f.ship, f.char
        self.memo = {}  # derived data of this fit (dealers, caches, ...)
        self._stats = None
        self._drains = None

    # ------------------------------------------------------------------ attribute helpers
    def g(self, i, name):
        return self.st.g(i, name)

    def has(self, i, name):
        a = self.ds.attr_by_name.get(name)
        return a is not None and self.v.has(i, a)

    def gopt(self, i, name):
        """Pyfa getModifiedItemAttr(name) without default: None when the item lacks the attribute"""
        a = self.ds.attr_by_name.get(name)
        if a is None or not self.v.has(i, a):
            return None
        return self.v.get(i, a)

    def effs(self, i):
        return gstats._effect_names(self.ds, self.meta[i])

    def group(self, i):
        return self.ds.group_name.get(self.meta[i]["group"]) or ""

    def name(self, i):
        return self.ds.t_name[self.meta[i]["ti"]]

    def charge(self, i):
        return self.meta[i]["charge"]

    def state(self, i):
        return self.meta[i]["state"]

    def active_modules(self):
        return [i for i in self.f.modules if self.meta[i]["state"] >= ACTIVE]

    def active_drones(self):
        return [(i, self.meta[i]["active_count"]) for i in self.f.drones if self.meta[i]["active_count"] > 0]

    def fighter_abilities(self, i):
        """names of the active abilities of an active fighter squadron (empty when inactive)"""
        m = self.meta[i]
        if m["active_count"] <= 0:
            return ()
        ab = m["fighter_abilities"]
        en = self.ds.effect_name
        if ab is None:
            ab = engine._default_fighter_abilities(self.ds, m["effects"])
        return tuple(en.get(e) for e in ab)

    def active_fighters(self):
        return [(i, self.meta[i]["active_count"]) for i in self.f.fighters if self.meta[i]["active_count"] > 0]

    # Pyfa Module.maxRange / falloff
    RANGE_ATTRS = ("maxRange", "shieldTransferRange", "powerTransferRange", "energyDestabilizationRange",
                   "empFieldRange", "ecmBurstRange", "warpScrambleRange", "cargoScanRange", "shipScanRange",
                   "surveyScanRange")

    def max_range(self, i):
        for a in self.RANGE_ATTRS:
            x = self.gopt(i, a)
            if x:
                if "burst projector" in self.name(i).lower():
                    x -= self.g(self.ship, "radius")
                return x
        d = self.missile_range_data(i)
        if d is None:
            return None
        lo, hi, hc = d
        return lo * (1 - hc) + hi * hc

    def falloff(self, i):
        for a in ("falloffEffectiveness", "falloff", "shipScanFalloff"):
            x = self.gopt(i, a)
            if x:
                return x
        return None

    def missile_range_data(self, i):
        c = self.charge(i)
        if c is None:
            return None
        if self.ds.group_name.get(self.meta[c]["group"]) in ("Scanner Probe", "Interdiction Probe",
                                                             "Survey Probe", "Warp Disruption Probe"):
            return None
        vel = self.g(c, "maxVelocity")
        if not vel:
            return None
        g = self.g
        radius = g(self.ship, "radius")
        ft = float_unerr(g(c, "explosionDelay") / 1000.0 + radius / vel)
        accel = g(c, "mass") * g(c, "agility") / 1e6

        def rng(t):
            a = min(t, accel)
            return vel / 2.0 * a + vel * (t - a)

        lt, ht = math.floor(ft), math.ceil(ft)
        lr, hr = rng(lt), rng(ht)
        if "fofMissileLaunching" in self.effs(c):
            lim = g(c, "maxFOFTargetRange")
            if lim:
                lr, hr = min(lr, lim), min(hr, lim)
        lr, hr = max(lr - radius, 0.0), max(hr - radius, 0.0)
        return lr, hr, ft - lt

    # Pyfa cycle parameters
    def raw_cycle_ms(self, i):
        return self.st.raw_cycle_ms(i)

    def num_shots(self, i):
        return self.st.num_shots(i)

    def cycle(self, i, factor_reload=None, breacher_override=False):
        """Pyfa Module.getCycleParameters(reloadOverride=factor_reload); None = no cycle"""
        if factor_reload is None:
            factor_reload = self.p["options"]["factor_reload"]
        active = self.raw_cycle_ms(i)
        if active == 0:
            return None
        shots = self.num_shots(i)
        until = INF if shots == 0 else shots
        inactive = self.g(i, "moduleReactivationDelay")
        reload = self.g(i, "reloadTime")
        if not factor_reload or until == INF or inactive >= reload:
            return Cycle(((active, inactive, INF, bool(factor_reload and inactive >= reload)),), 1)
        early = until - 1
        if early == 0:
            return Cycle(((active, reload, INF, True),), 1)
        return Cycle(((active, inactive, early, False), (active, reload, 1, True)), INF)

    def drone_cycle_ms(self, i):
        """Pyfa Drone.cycleTime"""
        if self.gopt(i, "entityMissileTypeID"):
            return max(self.g(i, "missileLaunchDuration"), 0.0)
        for a in ("speed", "duration", "durationHighisGood"):
            x = self.gopt(i, a)
            if x:
                return max(x, 0.0)
        return 0.0

    # ------------------------------------------------------------------ fit-level values
    def drone_control_range(self):
        r = self.memo.get("dcr")
        if r is not None:
            return r
        ds = self.ds
        sk = self.p["character"]["skills"]
        dl = sk["default_level"] or 0
        add = 0.0
        for nm in ("Drone Avionics", "Advanced Drone Avionics"):
            ti = ds.t_by_name.get(nm) if hasattr(ds, "t_by_name") else None
            tid = None
            if ti is None:
                tid = _type_id_by_name(ds, nm)
            else:
                tid = int(ds.t_id[ti])
            if tid is None:
                continue
            lv = sk["levels"].get(str(tid), dl)
            bonus = ds._tad_get(ds.tidx(tid)).get(ds.a("droneRangeBonus"), 0.0)
            add += bonus * lv
        mult = 1.0
        for i in [self.ship] + list(self.f.modules):
            if i != self.ship and self.state(i) < ONLINE:
                continue
            e = self.effs(i)
            if "droneRangeBonusAdd" in e:
                add += self.g(i, "droneRangeBonus")
            if "eliteBonusHeavyGunshipDroneControlRange1" in e:
                add += self.g(i, "eliteBonusHeavyGunship1")
            if "shipBonusRole1DroneHitpointsDroneControlRange" in e:
                mult *= 1 + self.g(i, "shipBonusRole1") / 100.0
        r = self.memo["dcr"] = (20000.0 + add) * mult
        return r

    def stats(self):
        """full variant-g FitStats (cached); also records the capacitor-simulation drains"""
        if self._stats is None:
            seen = {}
            orig = capsim.simulate

            def spy(capacity, recharge_ms, drains, start_frac, reload, stagger, t_max_ms):
                seen["args"] = (capacity, recharge_ms, list(drains), reload)
                return orig(capacity, recharge_ms, drains, start_frac, reload, stagger, t_max_ms)

            gstats.capsim.simulate = spy
            try:
                self._stats = FitStats(self.b, self.f, self.v).compute()
            finally:
                gstats.capsim.simulate = orig
            self._drains = seen.get("args")
        return self._stats

    def cap_drains(self):
        self.stats()
        return self._drains


_NAME_IDX = {}


def _type_id_by_name(ds, nm):
    m = _NAME_IDX.get(id(ds))
    if m is None:
        m = _NAME_IDX[id(ds)] = {n: int(t) for n, t in zip(ds.t_name, ds.t_id)}
    return m.get(nm)


class FitCache:
    """LRU memo of contexts keyed by canonical FitRequest JSON (+ a variant tag)"""

    def __init__(self, ds, enabled=True, size=64):
        self.ds, self.enabled, self.size = ds, enabled, size
        self._d = OrderedDict()
        self.hits = self.misses = 0

    def get(self, req, tag="", build=None, path="/fit"):
        if not self.enabled:
            return build(req) if build else Ctx(self.ds, req, path)
        key = tag + canon(req)
        c = self._d.get(key)
        if c is not None:
            self._d.move_to_end(key)
            self.hits += 1
            return c
        self.misses += 1
        c = build(req) if build else Ctx(self.ds, req, path)
        self._d[key] = c
        if len(self._d) > self.size:
            self._d.popitem(last=False)
        return c
