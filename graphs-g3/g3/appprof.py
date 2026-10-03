# SPDX-License-Identifier: LGPL-3.0-or-later
"""Application profile graph (Pyfa "Application Profile"): best charge per distance for the dominant weapon group.

Behaviour follows the Pyfa graph: charge stats come from base charge attributes scaled by the multipliers of the
loaded charge, the charge choice is made on a sampled distance grid (transitions refined by bisection to 10 m),
and the source's webs / painters enter through a sampled, linearly interpolated (speed, signature) table."""
import gzip
import json
import math
import os
from bisect import bisect_right

import numpy as np

from .common import NAN
from .ctx import ACTIVE, INF, GraphError
from .damage import Target, _speed_param, tackle

NAVY_PREFIXES = ("Imperial Navy ", "Republic Fleet ", "Caldari Navy ", "Federation Navy ")
CAPITAL_NAVY_PREFIXES = ("Sansha ", "Arch Angel ", "Shadow ")
DMG = ("emDamage", "thermalDamage", "kineticDamage", "explosiveDamage")
_META = {}


def _meta_groups(ds, path):
    """type id -> meta group (only in the raw dataset JSON); memoised in-process and in a sidecar next to
    variant G's dataset cache, keyed by the dataset sha256"""
    m = _META.get(ds.sha256)
    if m is not None:
        return m
    from evedogma_g.dataset import default_cache_dir
    side = os.path.join(default_cache_dir(), f"g3-metagroups-{ds.sha256}.json")
    try:
        with open(side) as f:
            m = {int(k): v for k, v in json.load(f).items()}
    except (OSError, ValueError):
        with gzip.open(path) as f:
            d = json.load(f)
        m = {int(k): t.get("meta_group") for k, t in d["types"].items()}
        try:
            os.makedirs(os.path.dirname(side), exist_ok=True)
            tmp = side + f".{os.getpid()}.tmp"
            with open(tmp, "w") as f:
                json.dump({k: v for k, v in m.items() if v is not None}, f)
            os.replace(tmp, side)
        except OSError:
            pass
    _META[ds.sha256] = m
    return m


def sample_step(max_d, min_step=100, target=300):
    if not max_d or max_d <= 0:
        return min_step
    step = max_d / target
    if step <= min_step:
        return min_step
    return int(math.ceil(step / min_step) * min_step)


def _base(ds, ti, name, default=0.0):
    a = ds.attr_by_name.get(name)
    if a is None:
        return default
    v = ds._tad_get(ti).get(a)
    return default if v is None else v


class Charges:
    """published charges per group (built once per dataset)"""
    _by_group = {}

    @classmethod
    def of_group(cls, ds, gid):
        m = cls._by_group.get(ds.sha256)
        if m is None:
            m = cls._by_group[ds.sha256] = {}
            pub = ds.t_pub
            for ti, gr in enumerate(ds.t_group.tolist()):
                if pub[ti]:
                    m.setdefault(gr, []).append(ti)
        return m.get(gid, [])


def valid_charges(c, i):
    ds = c.ds
    groups = []
    for k in range(5):
        x = c.gopt(i, f"chargeGroup{k}")
        if x:
            groups.append(int(x))
    if not groups:
        return []
    cap = float(ds.t_cap[c.meta[i]["ti"]])
    size = c.g(i, "chargeSize")
    out = []
    for gid in groups:
        for ti in Charges.of_group(ds, gid):
            vol = float(ds.t_vol[ti])
            if vol > cap:
                continue
            if size > 0 and size != _base(ds, ti, "chargeSize", None):
                continue
            out.append(ti)
    return sorted(set(out), key=lambda t: int(ds.t_id[t]))


def filter_quality(eng, tis, tier):
    if tier == "all":
        return tis
    ds = eng.ds
    mg = _meta_groups(ds, eng.dataset_path)
    out, classifiable = [], False
    for ti in tis:
        g = mg.get(int(ds.t_id[ti]))
        if g is not None:
            classifiable = True
        name = ds.t_name[ti]
        if g in (1, None):
            out.append(ti)
            continue
        if tier == "navy":
            if g == 2:
                out.append(ti)
            elif g == 4:
                if name.endswith(" XL"):
                    if any(name.startswith(p) for p in CAPITAL_NAVY_PREFIXES):
                        out.append(ti)
                elif any(name.startswith(p) for p in NAVY_PREFIXES):
                    out.append(ti)
    if out or classifiable:
        return out
    return tis


class Proj:
    """sampled (target speed, signature) table with linear interpolation (None table = base values)"""

    def __init__(self, c, tgt, settings, speed, sig, max_d, step):
        self.speed, self.sig = speed, sig
        self.d = None
        if settings["apply_projected"]:
            d = np.arange(0, max_d + 1, step, dtype=float) if max_d >= 0 else np.zeros(1)
            if len(d) == 0:
                d = np.zeros(1)
            tv, sm = tackle(c, tgt, settings, d, len(d), speed)
            self.d, self.tv, self.ts = d, tv, sig * sm

    def at(self, x):
        x = np.asarray(x, float)
        if self.d is None:
            return np.full(x.shape, float(self.speed)), np.full(x.shape, float(self.sig))
        d = self.d
        idx = np.searchsorted(d, x, side="right") - 1
        idx = np.maximum(idx, 0)
        last = idx >= len(d) - 1
        lo = np.minimum(idx, len(d) - 1)
        hi = np.minimum(idx + 1, len(d) - 1)
        dl, dh = d[lo], d[hi]
        with np.errstate(divide="ignore", invalid="ignore"):
            t = np.where(dh > dl, (x - dl) / np.where(dh > dl, dh - dl, 1.0), 0.0)
            sp = self.tv[lo] + t * (self.tv[hi] - self.tv[lo])
            inf = np.isinf(self.ts[lo]) | np.isinf(self.ts[hi])
            sg = np.where(inf, INF, self.ts[lo] + t * (self.ts[hi] - self.ts[lo]))
        plain = last | (x <= dl)
        sp = np.where(plain, self.tv[lo], sp)
        sg = np.where(plain, self.ts[lo], sg)
        return sp, sg

    def at1(self, x):
        s, g = self.at(np.array([float(x)]))
        return float(s[0]), float(g[0])


# ---------------------------------------------------------------- turrets
def _turret_rf(d, opt, fall):
    if d <= opt:
        return 1.0
    if fall > 0:
        return 0.5 ** ((max(0, d - opt) / fall) ** 2)
    return 0.0


def _ang(tp, d):
    a1, a2 = tp["atk_angle"] * math.pi / 180, tp["tgt_angle"] * math.pi / 180
    ctc = tp["atk_r"] + d + tp["tgt_r"]
    tr = abs(tp["atk_speed"] * math.sin(a1) - tp["tgt_speed"] * math.sin(a2))
    if ctc == 0:
        return 0 if tr == 0 else INF
    return tr / ctc


def _track(tracking, osr, ang, sig):
    if tracking <= 0 or sig <= 0:
        return 0
    if ang <= 0:
        return 1.0
    e = (ang * osr) / (tracking * sig)
    return 0.5 ** (e ** 2)


def _tmult(cth):
    w = min(cth, 0.01)
    n = cth - w
    return (n * ((0.01 + cth) / 2 + 0.49) if n > 0 else 0) + w * 3


def _turret_volley(cd, d, base, tp, proj):
    rf = _turret_rf(d, cd["opt"], cd["fall"])
    if tp is None:
        tf = 1.0
    else:
        sp, sg = proj.at1(d)
        tq = dict(tp, tgt_speed=sp)
        tf = _track(cd["track"], base["osr"], _ang(tq, d), sg)
    return cd["raw"] * _tmult(rf * tf)


def _best(cds, f):
    bv, bn, bi = 0, None, 0
    for k, cd in enumerate(cds):
        v = f(cd)
        if v > bv:
            bv, bn, bi = v, cd["tid"], k
    return bv, bn, bi


def _transitions(cds, f_at, max_d, launcher=False):
    """Pyfa calculateTransitions: [(distance, charge index, charge id, volley)]"""
    if not cds:
        return []
    res = sample_step(max_d)
    best = _best_l if launcher else _best
    bv, bn, bi = best(cds, lambda cd: f_at(cd, 0))
    tr = [(0, bi, bn, bv)]
    cur = bn
    d = res
    while d <= max_d:
        bv, bn, bi = best(cds, lambda cd: f_at(cd, d))
        if bn != cur:
            lo, hi = d - res, d
            while hi - lo > 10:
                mid = (lo + hi) // 2
                _, mn, _ = best(cds, lambda cd: f_at(cd, mid))
                if mn == cur:
                    lo = mid
                else:
                    hi = mid
            bv, _, _ = best(cds, lambda cd: f_at(cd, hi))
            tr.append((hi, bi, bn, bv))
            cur = bn
        if launcher and bv < 0.01:
            tr.append((d, -1, None, 0))
            break
        d += res
    return tr


def _turret_group(eng, c, i, tier, res, tp, proj_max_holder):
    g = c.g
    ds = c.ds
    opt, fall, track = c.gopt(i, "maxRange") or 0, c.gopt(i, "falloff") or 0, c.gopt(i, "trackingSpeed") or 0
    osr = c.gopt(i, "optimalSigRadius") or 0
    dmul = c.gopt(i, "damageMultiplier") or 1
    ch = c.charge(i)
    skill = 1.0
    if ch is not None:
        cti = c.meta[ch]["ti"]
        rm = _base(ds, cti, "weaponRangeMultiplier") or 1
        fm = _base(ds, cti, "fallofMultiplier") or 1
        tm = _base(ds, cti, "trackingSpeedMultiplier") or 1
        opt, fall, track = opt / rm, fall / fm, track / tm
        bd = sum(_base(ds, cti, a) or 0 for a in DMG)
        if bd > 0:
            skill = sum(g(ch, a) or 0 for a in DMG) / bd
    cyc = c.cycle(i)
    if cyc is None:
        return None
    tis = filter_quality(eng, valid_charges(c, i), tier)
    if not tis:
        return None
    longest = max([1.0] + [(_base(ds, t, "weaponRangeMultiplier") or 1.0) for t in tis])
    rng_info = int(opt * longest + fall * 3.1)
    base = {"opt": opt, "fall": fall, "track": track, "osr": osr, "dmul": dmul}
    cds = []
    for t in tis:
        dm = [(_base(ds, t, a) or 0) for a in DMG]
        if res:
            dm = [x * (1 - r) for x, r in zip(dm, res)]
        cds.append({"tid": int(ds.t_id[t]), "raw": sum(dm) * skill * dmul,
                    "opt": opt * (_base(ds, t, "weaponRangeMultiplier") or 1),
                    "fall": fall * (_base(ds, t, "fallofMultiplier") or 1),
                    "track": track * (_base(ds, t, "trackingSpeedMultiplier") or 1)})
    return {"kind": "turret", "base": base, "cds": cds, "cycle": cyc.average, "range": rng_info, "count": 1,
            "max_eff": int(max(cd["opt"] for cd in cds) + max(cd["fall"] for cd in cds) * 3.1)}


# ---------------------------------------------------------------- launchers
def _mfactor(er, ev, drf, sp, sig):
    f = [1]
    if er > 0:
        f.append(sig / er)
    if sp > 0 and er > 0:
        f.append(((ev * sig) / (er * sp)) ** drf)
    return min(f)


def _missile_volley(cd, d, proj):
    if d <= cd["lo"]:
        rf = 1.0
    elif d <= cd["hi"]:
        rf = cd["hc"]
    else:
        return 0
    sp, sg = proj.at1(d) if proj is not None else (0, INF)
    return cd["raw"] * rf * _mfactor(cd["er"], cd["ev"], cd["drf"], sp, sg)


def _best_l(cds, f):
    bv, bn, bi, bp = 0, None, 0, 99
    for k, cd in enumerate(cds):
        v = f(cd)
        if v > bv or (v == bv and v > 0 and cd["prio"] < bp):
            bv, bn, bi, bp = v, cd["tid"], k, cd["prio"]
    return bv, bn, bi


def _prio(name):
    n = name.lower()
    for k, p in (("mjolnir", 0), ("inferno", 1), ("scourge", 2), ("nova", 3)):
        if k in n:
            return p
    return 99


def _launcher_group(eng, c, i, tier, res, ship_r):
    ds = c.ds
    g = c.g
    cyc = c.cycle(i)
    if cyc is None:
        return None
    tis = filter_quality(eng, valid_charges(c, i), tier)
    if not tis:
        return None
    cc = c
    ii = i
    if c.charge(i) is None:
        cc, ii = _with_charge(eng, c, i, int(ds.t_id[tis[0]]))
    ch = cc.charge(ii)
    cti = cc.meta[ch]["ti"]
    mdm = cc.g(cc.char, "missileDamageMultiplier") or 1.0

    def mult(attr, extra=1.0):
        b = _base(ds, cti, attr) or 0
        if b > 0:
            return (cc.g(ch, attr) * extra or 0) / b
        return 1.0
    dmults = [mult(a, mdm) for a in DMG]
    fv, fd = mult("maxVelocity"), mult("explosionDelay")
    ae, av, ad = mult("aoeCloudSize"), mult("aoeVelocity"), mult("aoeDamageReductionFactor")
    lmult = cc.gopt(ii, "damageMultiplier") or 1
    cms = cyc.average
    cds = []
    for t in tis:
        bv, bdl = _base(ds, t, "maxVelocity") or 0, _base(ds, t, "explosionDelay") or 0
        bm, ba = _base(ds, t, "mass") or 1, _base(ds, t, "agility") or 1
        if bv <= 0 or bdl <= 0:
            continue
        vel, dl = bv * fv, bdl * fd
        ft = dl / 1000 + ship_r / vel
        lt, ht = math.floor(ft), math.ceil(ft)

        def rng(tt):
            acc = min(tt, bm * ba / 1000000)
            return vel / 2 * acc + vel * (tt - acc)
        lo, hi = max(0, rng(lt) - ship_r), max(0, rng(ht) - ship_r)
        dm = [(_base(ds, t, a) or 0) * m for a, m in zip(DMG, dmults)]
        tot = sum(dm) if not res else sum(x * (1 - r) for x, r in zip(dm, res))
        raw = tot * lmult
        cds.append({"tid": int(ds.t_id[t]), "raw": raw, "dps": raw / (cms / 1000) if cms > 0 else 0, "lo": lo,
                    "hi": hi, "hc": ft - lt, "max": hi, "er": (_base(ds, t, "aoeCloudSize") or 0) * ae,
                    "ev": (_base(ds, t, "aoeVelocity") or 0) * av,
                    "drf": (_base(ds, t, "aoeDamageReductionFactor") or 1) * ad, "prio": _prio(ds.t_name[t])})
    if not cds:
        return None
    cds.sort(key=lambda x: (-x["max"], -x["dps"]))
    return {"kind": "launcher", "cds": cds, "cycle": cms, "range": cds[0]["max"], "count": 1}


def _with_charge(eng, c, i, tid):
    req = dict(c.req)
    mods = []
    k = c.meta[i]["req_index"]
    for j, m in enumerate(req.get("modules") or []):
        m2 = dict(m)
        if j == k:
            m2["charge_type_id"] = tid
        mods.append(m2)
    req["modules"] = mods
    cc = eng.cache.get(req, tag="appcharge:")
    for jj in cc.f.modules:
        if cc.meta[jj]["req_index"] == k:
            return cc, jj
    raise GraphError("INTERNAL", "charge variant lost its module")


# ---------------------------------------------------------------- vectorised kernels ([charges] x [distances])
def _soa(cds, keys):
    return {k: np.array([cd[k] for cd in cds], float)[:, None] for k in keys}


def turret_matrix(soa, base, tp, proj, d):
    """applied volley of every charge (rows) at every distance (cols)"""
    d = np.asarray(d, float)[None, :]
    opt, fall, track = soa["opt"], soa["fall"], soa["track"]
    with np.errstate(divide="ignore", invalid="ignore", over="ignore"):
        rf = np.where(d <= opt, 1.0, np.where(fall > 0, 0.5 ** ((np.maximum(0, d - opt) / np.where(fall > 0, fall, 1.0)) ** 2), 0.0))
        if tp is None:
            tf = 1.0
        else:
            sp, sg = proj.at(d[0])
            sp, sg = sp[None, :], sg[None, :]
            a1, a2 = tp["atk_angle"] * math.pi / 180, tp["tgt_angle"] * math.pi / 180
            ctc = tp["atk_r"] + d + tp["tgt_r"]
            tr = np.abs(tp["atk_speed"] * math.sin(a1) - sp * math.sin(a2))
            ang = np.where(ctc == 0, np.where(tr == 0, 0.0, INF), tr / np.where(ctc == 0, 1.0, ctc))
            e = (ang * base["osr"]) / (track * sg)
            tf = np.where((track <= 0) | (sg <= 0), 0.0, np.where(ang <= 0, 1.0, 0.5 ** (e ** 2)))
        cth = rf * tf
        w = np.minimum(cth, 0.01)
        n = cth - w
        m = np.where(n > 0, n * ((0.01 + cth) / 2 + 0.49), 0.0) + w * 3
    return soa["raw"] * m


def missile_matrix(soa, proj, d):
    d = np.asarray(d, float)[None, :]
    if proj is not None:
        sp, sg = proj.at(d[0])
        sp, sg = sp[None, :], sg[None, :]
    else:
        sp, sg = np.zeros_like(d), np.full(d.shape, INF)
    er, ev, drf = soa["er"], soa["ev"], soa["drf"]
    with np.errstate(divide="ignore", invalid="ignore", over="ignore"):
        rf = np.where(d <= soa["lo"], 1.0, np.where(d <= soa["hi"], soa["hc"], 0.0))
        f = np.ones(np.broadcast(rf, sg).shape)
        f = np.where(er > 0, np.minimum(f, sg / np.where(er > 0, er, 1.0)), f)
        ok = (sp > 0) & (er > 0)
        f = np.where(ok, np.minimum(f, ((ev * sg) / np.where(ok, er * sp, 1.0)) ** drf), f)
    return np.where(rf == 0, 0.0, soa["raw"] * rf * f)


def _pick(mat, prio=None):
    """Pyfa best-charge rule per column: first strict maximum (> 0); launchers prefer lower damage priority on ties"""
    mx = mat.max(axis=0)
    if prio is None:
        idx = mat.argmax(axis=0)
    else:
        cand = np.where(mat == mx[None, :], prio, np.inf)
        idx = cand.argmin(axis=0)
    idx = np.where(mx > 0, idx, -1)
    return mx, idx


def transitions_v(cds, matf, max_d, prio=None, launcher=False):
    if not cds:
        return []
    res = sample_step(max_d)
    grid = np.arange(0, max_d + 1, res, dtype=np.int64) if max_d >= 0 else np.zeros(1, np.int64)
    if len(grid) == 0 or grid[0] != 0:
        grid = np.zeros(1, np.int64)
    mx, idx = _pick(matf(grid.astype(float)), prio)
    tid = [None if k < 0 else cds[k]["tid"] for k in idx.tolist()]
    tr = [(0, max(int(idx[0]), 0), tid[0], float(mx[0]))]
    # every change of best charge between grid[j-1] and grid[j] is bisected independently (the current charge
    # at j is always the grid best at j-1), so all bisections run as one batch per step
    J = [j for j in range(1, len(grid)) if tid[j] != tid[j - 1]]
    hv = {}
    if J:
        lo = grid[J] - res
        hi = grid[J].copy()
        cur = [tid[j - 1] for j in J]
        while True:
            act = np.flatnonzero(hi - lo > 10)
            if len(act) == 0:
                break
            mid = (lo[act] + hi[act]) // 2
            _, mi = _pick(matf(mid.astype(float)), prio)
            for q, m, k in zip(act.tolist(), mid.tolist(), mi.tolist()):
                mn = None if k < 0 else cds[k]["tid"]
                if mn == cur[q]:
                    lo[q] = m
                else:
                    hi[q] = m
        hvals, _ = _pick(matf(hi.astype(float)), prio)
        hv = {j: (int(h), float(v)) for j, h, v in zip(J, hi.tolist(), hvals.tolist())}
    for j in range(1, len(grid)):
        d = int(grid[j])
        bv, bn, bi = float(mx[j]), tid[j], max(int(idx[j]), 0)
        if j in hv:
            h, bv = hv[j]
            tr.append((h, bi, bn, bv))
        if launcher and bv < 0.01:
            tr.append((d, -1, None, 0))
            break
    return tr


def turret_matrix_diag(soa, base, tp, proj, d):
    """one charge per distance: soa rows are [1, N] aligned with d"""
    return _diag(turret_matrix, soa, d, base, tp, proj)


def missile_matrix_diag(soa, proj, d):
    return _diag(missile_matrix, soa, d, proj)


def _diag(fn, soa, d, *a):
    # the kernels broadcast [C,1] x [1,N]; with [1,N] parameter rows they evaluate element-wise
    if fn is turret_matrix:
        return fn(soa, a[0], a[1], a[2], d)[0]
    return fn(soa, a[0], d)[0]


# ---------------------------------------------------------------- graph
def run(eng, req, c, xs, ys, params, settings):
    tgt = Target(eng, req, settings)
    x = np.asarray(xs, float)
    n = len(x)
    tier = params.get("ammo_quality") or "all"
    res = None if settings["ignore_resists"] else tgt.res
    if res is not None and not any(res):
        res = None if False else res
    tspeed = _speed_param(params, "tgt", tgt.vmax)
    aspeed = _speed_param(params, "atk", c.g(c.ship, "maxVelocity"))
    aang = float(params.get("atk_angle_deg") if params.get("atk_angle_deg") is not None else 90.0)
    tang = float(params.get("tgt_angle_deg") if params.get("tgt_angle_deg") is not None else 90.0)
    sig = tgt.sig
    ship_r = c.g(c.ship, "radius")
    key = ("appprof", tier, res, settings["apply_projected"], settings["ignore_lock_range"],
           settings["ignore_drone_control_range"], settings["mobile_drone_mode"], tspeed, aspeed, aang, tang,
           repr(_tgt_key(tgt)))
    cache = c.memo.get(key)
    if cache is None:
        cache = _build(eng, c, tgt, settings, tier, res, tspeed, aspeed, aang, tang, sig, ship_r)
        c.memo[key] = cache
    groups, proj, tp, wtype = cache
    out = {y: np.zeros(n) for y in ys}
    ids = {y: [None] * n for y in ys}
    xc = np.maximum(x, 0.0)
    for gr in groups.values():
        dists = np.array([t[0] for t in gr["tr"]], float)
        k = np.maximum(np.searchsorted(dists, xc, side="right") - 1, 0)
        ci = np.array([t[1] for t in gr["tr"]], np.int64)[k]
        valid = (ci >= 0) & (x >= 0)
        cis = np.where(valid, ci, 0)
        row = {kk: v[:, 0][cis] for kk, v in gr["soa"].items()}
        soa = {kk: v[None, :] for kk, v in row.items()}
        if gr["kind"] == "turret":
            v = turret_matrix_diag(soa, gr["base"], tp, proj, xc)
        else:
            v = missile_matrix_diag(soa, gr["pj"], xc)
        v = np.where(valid, v, 0.0)
        cyc = gr["cycle"]
        for y in ys:
            if y == "dps":
                out[y] += (v / (cyc / 1000) if cyc > 0 else 0 * v) * gr["count"]
            else:
                out[y] += v * gr["count"]
        tids = [cd["tid"] for cd in gr["cds"]]
        for y in ys:
            col = ids[y]
            for j in np.flatnonzero(valid).tolist():
                if col[j] is None:
                    col[j] = tids[cis[j]]
    r = {y: np.where(x < 0, NAN, out[y]) for y in ys}
    for y in ys:
        r[y + "_charge_type_id"] = ids[y]
    return r


def _tgt_key(tgt):
    return (tgt.vmax, tgt.sig, tgt.radius, tgt.res, tgt.hp, id(tgt.fit) if tgt.fit is not None else None)


def _build(eng, c, tgt, settings, tier, res, tspeed, aspeed, aang, tang, sig, ship_r):
    tur, lau = [], []
    for i in c.active_modules():
        if c.g(i, "miningAmount"):
            continue
        e = c.effs(i)
        if "turretFitted" in e:
            tur.append(i)
        elif "launcherFitted" in e:
            lau.append(i)
    if not tur and not lau:
        return {}, None, None, None
    wtype = "turret" if len(tur) >= len(lau) else "launcher"
    mods = tur if wtype == "turret" else lau
    infos = {}
    max_eff = 0
    for i in mods:
        tid = c.meta[i]["type_id"]
        if tid in infos:
            continue
        info = _turret_group(eng, c, i, tier, res, None, None) if wtype == "turret" else \
            _launcher_group(eng, c, i, tier, None, ship_r)
        if info:
            infos[tid] = (i, info)
            max_eff = max(max_eff, info["range"])
    if not infos:
        return {}, None, None, wtype
    proj = Proj(c, tgt, settings, tspeed, sig, max_eff, sample_step(max_eff))
    tp = None if sig == 0 else {"atk_speed": aspeed, "atk_angle": aang, "atk_r": ship_r, "tgt_speed": tspeed,
                                "tgt_angle": tang, "tgt_r": tgt.radius}
    groups = {}
    for i in mods:
        tid = c.meta[i]["type_id"]
        if tid in groups:
            groups[tid]["count"] += 1
            continue
        if tid not in infos:
            continue
        i0, info = infos[tid]
        if wtype == "turret":
            gr = dict(info)
            gr["soa"] = _soa(gr["cds"], ("opt", "fall", "track", "raw"))
            gr["tr"] = transitions_v(gr["cds"], lambda d, g=gr: turret_matrix(g["soa"], g["base"], tp, proj, d),
                                     gr["max_eff"])
        else:
            gr = _launcher_group(eng, c, i0, tier, res, ship_r)
            if gr is None:
                continue
            pj = proj if tp is not None else None
            gr["pj"] = pj
            gr["soa"] = _soa(gr["cds"], ("lo", "hi", "hc", "er", "ev", "drf", "raw"))
            prio = np.array([cd["prio"] for cd in gr["cds"]], float)[:, None]
            gr["tr"] = transitions_v(gr["cds"], lambda d, g=gr: missile_matrix(g["soa"], pj, d), int(gr["range"]),
                                     prio, True)
        gr["count"] = 1
        groups[tid] = gr
    return groups, proj, tp, wtype
