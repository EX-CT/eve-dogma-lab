"""Batch dogma engine.

A *batch* holds any number of fits. Everything that scales with the number of items or modifiers is
stored as flat NumPy columns over the whole batch:

  items      one row per item (ship, character, skills, modules, charges, drones, ...) of every fit
  mods       one row per (target item, attribute, operator, source) modifier
  nodes      one row per (item, attribute) value, keyed by  item << 14 | attribute

Building = relational joins (modifier templates x items x target tables) with searchsorted.
Evaluation = the dogma graph is levelised (longest path from unmodified attributes) and evaluated one
level at a time; within a level all operator stages, stacking penalties and min/max caps are vector
ops over every modified attribute of every fit in the batch.

The few effects that the SDE ships without modifierInfo (propulsion, MJD, slot/hardpoint modifiers,
projected ewar, warfare bursts, Reactive Armor Hardener) are registered by small per-fit Python code
that appends rows to the same tables.
"""
import math

import numpy as np
from array import array

from .dataset import (ATTR_BITS, SPECIAL_AB, SPECIAL_F_AB, SPECIAL_F_EVASIVE, SPECIAL_F_MWD, SPECIAL_HARDPOINT,
                      SPECIAL_MJD, SPECIAL_MWD, SPECIAL_SLOT)
from .request import RequestError

# item kinds / locations
SHIP, CHAR, SKILL, MODULE, CHARGE, DRONE, FIGHTER, IMPLANT, BOOSTER, MODE, BEACON, PROJECTED = range(12)
L_SHIP, L_CHAR, L_SPACE, L_NOWHERE = range(4)
OWNED_KINDS = (MODULE, CHARGE, DRONE, FIGHTER, SHIP)
FULL_KINDS = (SHIP, CHAR, MODULE, CHARGE, DRONE, FIGHTER, PROJECTED)  # all base attrs materialised
OFFLINE, ONLINE, ACTIVE, OVERHEATED = range(4)

EXEMPT_CATEGORIES = np.array([6, 8, 16, 20, 32, 65])
HULL_RESONANCES = (113, 111, 109, 110)
ATTR_SKILL_LEVEL = 280
STRUCTURE_SKILL_EFFECT_NAMES = (
    "targetingMaxTargetBonusModAddMaxLockedTargetsLocationChar", "skillStructureMissileDamageBonus",
    "skillStructureElectronicSystemsCapNeedBonus", "skillStructureEngineeringSystemsCapNeedBonus",
    "skillStructureDoomsdayDurationBonus")
# effect category -> minimum state for the effect to be applied locally (None = never local)
STATE_OK = np.zeros((8, 4), bool)
for _cat, _min in ((0, ONLINE), (4, ONLINE), (1, ACTIVE), (5, OVERHEATED), (7, OFFLINE)):
    STATE_OK[_cat, _min:] = True
OP_STAGES = (-1, 0, 1, 2, 3, 4, 5, 6, 7)  # CCP operator application order
STAGE_OF_OP = {op: s for s, op in enumerate(OP_STAGES)}
SRC_ATTR, SRC_CONST, SRC_PROP, SRC_PROJ = range(4)
PENALTY_DENOM = 7.1289
MAX_LEVELS = 64
LATE_ORDER = 1 << 40  # registration order of rows added after the item-effect pass

# target table classes (see Batch._target_table)
T_LOC_SHIP, T_LOC_CHAR, T_GRP_SHIP, T_GRP_CHAR, T_RS_SHIP, T_RS_OWNED, T_RS_CHAR = range(7)
KEY_X_BITS = 20


def _tkey(fit, cls, x):
    return ((np.asarray(fit, np.int64) * 8 + cls) << KEY_X_BITS) | np.asarray(x, np.int64)


def _libm_exp(x):
    """element-wise math.exp (libm, bit-identical to Rust f64::exp); NumPy's SIMD exp may differ by an ulp"""
    return np.array(list(map(math.exp, x.tolist())), np.float64)


def _sorted_unique(x):
    """np.unique via a stable (timsort) sort: the keys arrive mostly sorted (item-major), which makes this
    several times faster than NumPy 2's hash-based unique"""
    x = np.sort(x, kind="stable")
    if len(x) == 0:
        return x
    keep = np.empty(len(x), bool)
    keep[0] = True
    np.not_equal(x[1:], x[:-1], out=keep[1:])
    return x[keep]


def _expand_ranges(lo, hi):
    """for ranges [lo_i, hi_i): (owner index per element, element positions)"""
    n = hi - lo
    owner = np.repeat(np.arange(len(lo)), n)
    if len(owner) == 0:
        return owner, owner.astype(np.int64)
    starts = np.repeat(lo - np.concatenate(([0], np.cumsum(n)[:-1])), n)
    return owner, starts + np.arange(len(owner))


def _join(qkeys, tkeys_sorted, tvals_sorted):
    """many-to-many equi-join: (query row, matched table value) pairs"""
    lo = np.searchsorted(tkeys_sorted, qkeys, "left")
    hi = np.searchsorted(tkeys_sorted, qkeys, "right")
    q, pos = _expand_ranges(lo, hi)
    return q, tvals_sorted[pos]


LOCAL_SPECIAL = frozenset((
    "superWeaponAmarr", "superWeaponCaldari", "superWeaponGallente", "superWeaponMinmatar", "doomsdaySlash",
    "doomsdayBeamDOT", "doomsdayConeDOT", "doomsdayHOG", "debuffLance", "emergencyHullEnergizer", "entosisLink",
    "microJumpPortalDrive", "microJumpPortalDriveCapital", "warpDisruptSphere", "moduleBonusBreacherPodDamageControl"))

DAMAGE_EFFECTS = frozenset((
    "projectileFired", "targetAttack", "useMissiles", "barrage", "targetDisintegratorAttack", "missileLaunchingForEntity",
    "fighterAbilityAttackM", "fighterAbilityMissiles", "superWeaponAmarr", "superWeaponCaldari", "superWeaponGallente",
    "superWeaponMinmatar", "mining", "miningLaser", "miningClouds", "dotMissileLaunching", "ChainLightning",
    "salvageDroneEffect"))


def _sec_set(ds, src):
    c = ds.__dict__.setdefault("_sec_sets", {})
    if src not in c:
        c[src] = frozenset(ds.sec_types[src].tolist())
    return c[src]


def _sec_hits(ds, src, tis):
    """np.isin(tis, sec_types[src]), memoised on the skill list (identical for most fits)"""
    c = ds.__dict__.setdefault("_sec_hits", {})
    k = (src, tis.tobytes())
    h = c.get(k)
    if h is None:
        if len(c) > 64:
            c.clear()
        h = c[k] = np.isin(tis, ds.sec_types[src])
    return h


def _default_fighter_abilities(ds, effects):
    """Pyfa default: standard attack on; other abilities (except MWD/evasive/MJD) on only if they come before the
    standard attack in effect order"""
    on, std_seen = [], False
    for e in sorted(e for e, _ in effects):
        nm = ds.effect_name.get(e)
        if nm is None or not nm.startswith("fighterAbility"):
            continue
        if nm == "fighterAbilityAttackM":
            on.append(e)
            std_seen = True
        elif not std_seen and nm not in ("fighterAbilityMicroWarpDrive", "fighterAbilityEvasiveManeuvers",
                                         "fighterAbilityMicroJumpDrive"):
            on.append(e)
    return on


FIGHTER_SELF = {
    SPECIAL_F_MWD: (("maxVelocity", "fighterAbilityMicroWarpDriveSpeedBonus", 6),
                    ("signatureRadius", "fighterAbilityMicroWarpDriveSignatureRadiusBonus", 6)),
    SPECIAL_F_AB: (("maxVelocity", "fighterAbilityAfterburnerSpeedBonus", 6),),
    SPECIAL_F_EVASIVE: (("maxVelocity", "fighterAbilityEvasiveManeuversSpeedBonus", 6),
                        ("signatureRadius", "fighterAbilityEvasiveManeuversSignatureRadiusBonus", 6),
                        ("shieldEmDamageResonance", "fighterAbilityEvasiveManeuversEmResonance", 4),
                        ("shieldThermalDamageResonance", "fighterAbilityEvasiveManeuversThermResonance", 4),
                        ("shieldKineticDamageResonance", "fighterAbilityEvasiveManeuversKinResonance", 4),
                        ("shieldExplosiveDamageResonance", "fighterAbilityEvasiveManeuversExpResonance", 4)),
}


class BaseOverrides(dict):
    """(item, attr) -> base value, with a per-item index"""

    def __init__(self, *a):
        super().__init__(*a)
        self._by_item = {}
        for (i, at), v in self.items():
            self._by_item.setdefault(i, {})[at] = v

    def __setitem__(self, key, v):
        super().__setitem__(key, v)
        self._by_item.setdefault(key[0], {})[key[1]] = v

    def of_item(self, i):
        return self._by_item.get(i, {})

    def copy(self):
        return BaseOverrides(self)

    def drop_items_from(self, n):
        """remove every override of items >= n (rollback of a failed add_fit: it only creates new items)"""
        for i in [i for i in self._by_item if i >= n]:
            for at in self._by_item.pop(i):
                del self[(i, at)]


class Fit:
    """per-fit metadata (Python side). Item indices are global batch indices."""

    def __init__(self, req, index):
        self.req = req
        self.index = index
        self.warnings = []
        self.proj_special = []  # incoming remote reps / cap drains (stats)
        self.items = []  # non-skill items, global indices
        self.modules, self.drones, self.fighters = [], [], []
        self.error = None


class Batch:
    def __init__(self, ds):
        self.ds = ds
        self.fits = []
        # item columns (Python lists while building, NumPy after finish_items)
        # (array.array: appends like a list, converts to NumPy through the buffer protocol without a per-element loop)
        self._cols = {k: array(t) for k, t in (("fit", "q"), ("ti", "q"), ("kind", "b"), ("loc", "b"), ("owned", "b"),
                                                ("state", "b"), ("parent", "q"), ("charge", "q"))}
        self.meta = []  # per item dict (slot, req_index, quantity, ...) - None for skills
        self.meta_items = []  # indices of the items with a meta dict (all but skills), ascending
        self.skill_blocks = []  # (fit, first item index, type idx array, level array)
        self.n_items = 0
        self.overrides = BaseOverrides()  # (item, attr) -> base value
        self.custom_effects = {}  # item -> list[(eff, default)] (mutated items)
        self.custom_reqskills = {}
        self.extra_rows = []  # (item, type idx, excluded effects) template rows from a mutation base type
        self.mods = []  # list of dicts of NumPy columns, concatenated lazily
        self.small = []  # Python-side mod rows (tuples)
        self.values = None

    # ------------------------------------------------------------------ items
    def _new_item(self, fit, type_id, kind, loc, path, owned=None):
        ds = self.ds
        ti = ds.tidx(type_id)
        if ti < 0:
            raise RequestError("UNKNOWN_TYPE", f"unknown type_id {type_id}", path)
        c = self._cols
        c["fit"].append(fit.index); c["ti"].append(ti); c["kind"].append(kind); c["loc"].append(loc)
        c["owned"].append(kind in OWNED_KINDS if owned is None else owned)
        c["state"].append(ONLINE); c["parent"].append(-1); c["charge"].append(-1)
        idx = self.n_items
        self.n_items += 1
        tcache = ds.__dict__.get("_tinfo")
        if tcache is None:
            tcache = ds.__dict__["_tinfo"] = {}
        tinfo = tcache.get(ti)
        if tinfo is None:  # (effects (shared, read-only), group, category) per type
            tinfo = tcache[ti] = (ds.t_effects[ti], int(ds.t_group[ti]), int(ds.t_cat[ti]))
        self.meta.append({"type_id": type_id, "ti": ti, "kind": kind, "slot": None, "req_index": None,
                          "quantity": 1, "active_count": 0, "state": ONLINE, "charge": None, "parent": None,
                          "effects": tinfo[0], "fighter_abilities": None, "side_effects": (),
                          "spool": None, "distance": None, "group": tinfo[1], "category": tinfo[2]})
        fit.items.append(idx)
        self.meta_items.append(idx)
        return idx

    def _set_state(self, i, st):
        self._cols["state"][i] = st
        self.meta[i]["state"] = st

    def _apply_mutation(self, i, m):
        ds = self.ds
        it = self.meta[i]
        bti = ds.tidx(m["base_type_id"])
        if bti >= 0:
            own_ti = it["ti"]

            def raw(ti):
                d = ds.type_attrs(ti)
                for a in (4, 38, 161, 162):
                    d.pop(a, None)
                d.update(ds.t_raw_fields[ti])
                return d

            attrs = ds.type_attrs(own_ti)
            attrs.update(raw(bti))
            attrs.update(raw(own_ti))
            if attrs.get(4, 0.0) == 0.0 and ds.t_mass[bti] != 0.0:
                attrs[4] = float(ds.t_mass[bti])
            for a, v in attrs.items():
                self.overrides[(i, a)] = v
            own = {e for e, _ in it["effects"]}
            extra = [(e, d) for e, d in ds.t_effects[bti] if e not in own]
            if extra:
                it["effects"] = list(it["effects"]) + extra
                self.extra_rows.append((i, bti, own))
            if not ds.t_reqskills[own_ti]:
                self.custom_reqskills[i] = ds.t_reqskills[bti]
        muta = ds.muta.get(m["mutaplasmid_type_id"]) if m["mutaplasmid_type_id"] is not None else None
        for k, v in m["attributes"].items():
            try:
                aid = int(k)
            except ValueError:
                continue
            val = v
            if muta is not None and bti >= 0:
                rng = muta["attrs"].get(k)
                bv = ds.type_attr(bti, aid)
                if rng is not None and bv is not None and bv != 0.0:
                    a, b = bv * rng[0], bv * rng[1]
                    val = min(max(val, min(a, b)), max(a, b))
            self.overrides[(i, aid)] = val

    def add_fit(self, req, projected_frozen=None):
        """Create all items of one request (no modifiers yet). Raises RequestError for unknown types."""
        ds = self.ds
        fit = Fit(req, len(self.fits))
        fit.projected_frozen = projected_frozen
        n_items0, n_meta0 = self.n_items, len(self.meta)
        cols_len0 = {k: len(v) for k, v in self._cols.items()}
        try:
            self._add_fit_items(fit, req)
        except RequestError:
            # roll back partial items of this fit
            self.n_items = n_items0
            del self.meta[n_meta0:]
            while self.meta_items and self.meta_items[-1] >= n_items0:
                self.meta_items.pop()
            for k, v in self._cols.items():
                del v[cols_len0[k]:]
            self.overrides.drop_items_from(n_items0)
            self.skill_blocks = [b for b in self.skill_blocks if b[0] != fit.index]
            self.extra_rows = [r for r in self.extra_rows if r[0] < n_items0]
            raise
        self.fits.append(fit)
        return fit

    def _add_fit_items(self, fit, req):
        ds = self.ds
        ship = self._new_item(fit, req["ship"]["type_id"], SHIP, L_SHIP, "/ship/type_id")
        fit.ship = ship
        fit.is_structure = self.meta[ship]["category"] == 65
        ch = self._new_item(fit, 1373, CHAR, L_CHAR, "/character")
        fit.char = ch
        sec = req["character"]["security_status"]
        if sec is not None and ds.a("pilotSecurityStatus"):
            self.overrides[(ch, ds.a("pilotSecurityStatus"))] = sec
        # skills: every published skill at default level, then explicit levels (by id or name)
        sk = req["character"]["skills"]
        dl = sk["default_level"] or 0
        levels = {}
        for k, v in sk["levels"].items():
            try:
                sid = int(k)
            except ValueError:
                sid = ds.type_by_name.get(k.strip().lower())
                if sid is None:
                    continue
            levels[sid] = v
        sids = ds.published_skills
        lv = np.full(len(sids), min(dl, 5), np.float64)
        extra_ids = []
        if levels:
            pos = {int(s): k for k, s in enumerate(sids.tolist())} if len(levels) > 0 else {}
            for sid, v in levels.items():
                if sid in pos:
                    lv[pos[sid]] = min(v, 5)
                elif ds.tidx(sid) >= 0:
                    extra_ids.append((sid, min(v, 5)))
        all_ids = np.concatenate([sids, np.array([s for s, _ in extra_ids], np.int64)])
        all_lv = np.concatenate([lv, np.array([v for _, v in extra_ids], np.float64)])
        tis = ds.type_index[all_ids].astype(np.int32)
        first = self.n_items
        n = len(all_ids)
        self.skill_blocks.append((fit.index, first, tis, all_lv))
        c = self._cols
        c["fit"].extend(array("q", (fit.index,)) * n); c["ti"].frombytes(tis.astype(np.int64).tobytes())
        c["kind"].extend(array("b", (SKILL,)) * n); c["loc"].extend(array("b", (L_CHAR,)) * n)
        c["owned"].extend(array("b", (0,)) * n); c["state"].extend(array("b", (ONLINE,)) * n)
        c["parent"].extend(array("q", (-1,)) * n); c["charge"].extend(array("q", (-1,)) * n)
        self.meta.extend([None] * n)
        self.n_items += n
        fit.skill_levels = dict(zip(all_ids.tolist(), all_lv.tolist()))
        fit.skill_first, fit.skill_n = first, n
        # tactical destroyer mode
        mode = req["ship"]["mode_type_id"]
        if mode is None:
            sname = ds.t_name[self.meta[ship]["ti"]].lower()
            cands = [tid for tid, nm in ds.modes_1306 if nm.startswith(sname)]
            if cands:
                mode = min(cands)
                fit.warnings.append(f"no tactical mode given; defaulted to type {mode}")
        if mode is not None:
            self._new_item(fit, mode, MODE, L_NOWHERE, "/ship/mode_type_id", owned=False)
        for i, m in enumerate(req["modules"]):
            path = f"/modules/{i}"
            idx = self._new_item(fit, m["type_id"], MODULE, L_SHIP, path)
            it = self.meta[idx]
            slot = m["slot"] or ds.t_slot[it["ti"]]
            it["slot"], it["req_index"], it["spool"] = slot, i, m["spool"]
            st = m["state"] if m["state"] is not None else ONLINE
            if slot in ("rig", "subsystem") and st != OFFLINE:
                st = ONLINE
            self._set_state(idx, st)
            if m["mutation"]:
                self._apply_mutation(idx, m["mutation"])
            if m["charge_type_id"] is not None:
                cidx = self._new_item(fit, m["charge_type_id"], CHARGE, L_SHIP, path + "/charge_type_id")
                self._cols["parent"][cidx] = idx
                self._cols["charge"][idx] = cidx
                self.meta[cidx]["parent"] = idx
                self.meta[cidx]["req_index"] = i
                it["charge"] = cidx
            fit.modules.append(idx)
        for i, d in enumerate(req["drones"]):
            idx = self._new_item(fit, d["type_id"], DRONE, L_SPACE, f"/drones/{i}")
            if d["mutation"]:
                self._apply_mutation(idx, d["mutation"])
            it = self.meta[idx]
            it["quantity"] = max(d["quantity"], 1)
            it["active_count"] = min(d["active"] or 0, it["quantity"])
            self._set_state(idx, ACTIVE if it["active_count"] > 0 else OFFLINE)
            it["req_index"] = i
            fit.drones.append(idx)
        sq_attr = ds.a("fighterSquadronMaxSize")
        for i, f in enumerate(req["fighters"]):
            idx = self._new_item(fit, f["type_id"], FIGHTER, L_SPACE, f"/fighters/{i}")
            it = self.meta[idx]
            maxsq = int(ds.type_attr(it["ti"], sq_attr, 1.0))
            q = f["quantity"]
            it["quantity"] = min(max(q if q is not None else maxsq, 1), max(maxsq, 1))
            if (q or 0) > maxsq:
                fit.warnings.append(f"fighters/{i}: squadron size {q} capped to {maxsq}")
            it["active_count"] = it["quantity"] if f["active"] else 0
            self._set_state(idx, ACTIVE if f["active"] else OFFLINE)
            it["fighter_abilities"] = list(f["abilities"]) if f["abilities"] is not None else \
                _default_fighter_abilities(ds, it["effects"])
            it["req_index"] = i
            fit.fighters.append(idx)
        for i, imp in enumerate(req["implants"]):
            idx = self._new_item(fit, imp, IMPLANT, L_CHAR, f"/implants/{i}", owned=False)
            self.meta[idx]["req_index"] = i
        for i, b in enumerate(req["boosters"]):
            idx = self._new_item(fit, b["type_id"], BOOSTER, L_CHAR, f"/boosters/{i}", owned=False)
            self.meta[idx]["side_effects"] = tuple(b["side_effects"])
            self.meta[idx]["req_index"] = i
        for i, e in enumerate(req["environment"]["effect_type_ids"]):
            self._new_item(fit, e, BEACON, L_NOWHERE, f"/environment/effect_type_ids/{i}", owned=False)
        for i, p in enumerate(req["projected"]):
            if p["kind"] == "module":
                if p["module"]:
                    m = p["module"]
                    for _ in range(max(p["amount"], 1)):
                        idx = self._new_item(fit, m["type_id"], PROJECTED, L_NOWHERE, f"/projected/{i}", owned=False)
                        self._set_state(idx, m["state"] if m["state"] is not None else ACTIVE)
                        self.meta[idx]["distance"] = p["distance_m"]
                        self.meta[idx]["req_index"] = i
                        if m["charge_type_id"] is not None:
                            cidx = self._new_item(fit, m["charge_type_id"], CHARGE, L_NOWHERE,
                                                  f"/projected/{i}/module/charge_type_id", owned=False)
                            self._cols["parent"][cidx] = idx
                            self._cols["charge"][idx] = cidx
                            self.meta[cidx]["parent"] = idx
                            self.meta[idx]["charge"] = cidx
            elif p["kind"] == "fit":
                # whole projected fit, computed beforehand on its own (calc._batch): active modules and drones
                # are projected as frozen items carrying the source fit's modified attribute values
                frozen = (getattr(fit, "projected_frozen", None) or {}).get(i)
                if frozen is None:
                    continue
                if isinstance(frozen, str):
                    fit.warnings.append(f"projected[{i}] fit: {frozen}")
                    continue
                for type_id, copies, vals, src_kind, qty, abil in frozen:
                    for _ in range(copies * max(p["amount"], 1)):
                        idx = self._new_item(fit, type_id, PROJECTED, L_NOWHERE, f"/projected/{i}", owned=False)
                        self._set_state(idx, ACTIVE)
                        self.meta[idx]["distance"] = p["distance_m"]
                        self.meta[idx]["req_index"] = i
                        if src_kind == FIGHTER:
                            self.meta[idx]["quantity"] = qty
                            self.meta[idx]["active_count"] = qty
                            self.meta[idx]["fighter_abilities"] = None if abil is None else list(abil)
                        for at, v in vals.items():
                            self.overrides[(idx, at)] = v
            elif p["kind"] == "fighter":
                if p["fighter"]:
                    f = p["fighter"]
                    sq = ds.a("fighterSquadronMaxSize")
                    for _ in range(max(p["amount"], 1)):
                        idx = self._new_item(fit, f["type_id"], PROJECTED, L_NOWHERE, f"/projected/{i}", owned=False)
                        it = self.meta[idx]
                        b = ds.type_attr(it["ti"], sq)
                        maxsq = max(int(b) if b is not None and b > 0 else (0 if b is not None else 1), 1)
                        self._set_state(idx, ACTIVE if f["active"] else OFFLINE)
                        q = f["quantity"] if f["quantity"] is not None else maxsq
                        it["quantity"] = min(max(q, 1), maxsq)
                        it["active_count"] = it["quantity"]
                        it["distance"] = p["distance_m"]
                        it["req_index"] = i
                        it["fighter_abilities"] = list(f["abilities"]) if f["abilities"] is not None else \
                            _default_fighter_abilities(ds, it["effects"])
            elif p["kind"] == "drone":
                if p["drone"]:
                    d = p["drone"]
                    for _ in range(max(p["amount"], 1) * max(d["quantity"], 1)):
                        idx = self._new_item(fit, d["type_id"], PROJECTED, L_NOWHERE, f"/projected/{i}", owned=False)
                        self._set_state(idx, ACTIVE)
                        self.meta[idx]["distance"] = p["distance_m"]
            else:
                fit.warnings.append(f"projected kind '{p['kind']}' not supported yet (index {i})")
        # system security -> securityModifier
        sec = (req["environment"]["system_security"] or "nullsec").lower()
        if sec in ("hisec", "highsec", "high"):
            src = "hiSecModifier"
        elif sec in ("lowsec", "low"):
            src = "lowSecModifier"
        elif sec in ("nullsec", "null", "wspace", "wormhole", "w-space"):
            src = "nullSecModifier"
        else:
            fit.warnings.append(f"unknown system_security '{sec}', using nullsec")
            src = "nullSecModifier"
        src_id, dst_id = ds.a(src), ds.a("securityModifier")
        sec_types = ds.sec_types.get(src)
        if sec_types is not None and len(sec_types):
            sec_set = _sec_set(ds, src)
            for i in fit.items:
                ti = self.meta[i]["ti"]
                key = (i, src_id)
                if key in self.overrides:
                    self.overrides[(i, dst_id)] = self.overrides[key]
                elif ti in sec_set:
                    self.overrides[(i, dst_id)] = ds.type_attr(ti, src_id)
            tis = self.skill_blocks[-1][2]
            hit = _sec_hits(ds, src, tis)
            for k in np.nonzero(hit)[0].tolist():
                self.overrides[(fit.skill_first + k, dst_id)] = ds.type_attr(int(tis[k]), src_id)
        for o in req["overrides"]:
            for i in fit.items:
                if self.meta[i]["type_id"] == o["type_id"]:
                    self.overrides[(i, o["attribute_id"])] = o["value"]
            if o["type_id"] in fit.skill_levels:
                sid_list = self.skill_blocks[-1][2]
                ti = ds.tidx(o["type_id"])
                for k in np.nonzero(sid_list == ti)[0].tolist():
                    self.overrides[(fit.skill_first + k, o["attribute_id"])] = o["value"]

    # ------------------------------------------------------------------ registration
    def finish_items(self):
        c = self._cols
        def col(k, dt):
            return np.frombuffer(c[k], dt).copy() if len(c[k]) else np.zeros(0, dt)
        self.it_fit = col("fit", np.int64)
        self.it_ti = col("ti", np.int64)
        self.it_kind = col("kind", np.int8)
        self.it_loc = col("loc", np.int8)
        self.it_owned = col("owned", np.int8).astype(bool)
        self.it_state = col("state", np.int8)
        self.it_parent = col("parent", np.int64)
        self.it_charge = col("charge", np.int64)
        ds = self.ds
        self.it_group = ds.t_group[self.it_ti].astype(np.int64)
        self.it_cat = ds.t_cat[self.it_ti].astype(np.int64)
        nf = len(self.fits)
        self.fit_ship = np.array([f.ship for f in self.fits], np.int64)
        self.fit_char = np.array([f.char for f in self.fits], np.int64)
        self.fit_struct = np.array([f.is_structure for f in self.fits], bool) if nf else np.zeros(0, bool)
        # effective state for effect registration
        st = self.it_state.copy()
        k = self.it_kind
        st[np.isin(k, (SHIP, CHAR, SKILL, IMPLANT, BOOSTER, MODE, BEACON))] = ONLINE
        ch = k == CHARGE
        st[ch] = np.where(self.it_parent[ch] >= 0, self.it_state[np.maximum(self.it_parent[ch], 0)], ONLINE)
        self.it_estate = st
        self._target_table()

    def _target_table(self):
        """sorted (key -> item) table used to resolve Location / LocationGroup / RequiredSkill modifiers"""
        n = self.n_items
        items = np.arange(n, dtype=np.int64)
        f, loc, grp = self.it_fit, self.it_loc, self.it_group
        keys, vals = [], []
        s = loc == L_SHIP
        c = loc == L_CHAR
        keys += [_tkey(f[s], T_LOC_SHIP, 0), _tkey(f[c], T_LOC_CHAR, 0),
                 _tkey(f[s], T_GRP_SHIP, grp[s]), _tkey(f[c], T_GRP_CHAR, grp[c])]
        vals += [items[s], items[c], items[s], items[c]]
        # required skills (skills' own requirements are never used as targets)
        rs_item, rs_skill = [], []
        meta = self.meta
        for i in self.meta_items:
            req = self.custom_reqskills.get(i) or self.ds.t_reqskills[meta[i]["ti"]]
            for sk in req:
                rs_item.append(i)
                rs_skill.append(sk)
        rs_item = np.array(rs_item, np.int64)
        rs_skill = np.array(rs_skill, np.int64)
        self.rs_item, self.rs_skill = rs_item, rs_skill
        rf = f[rs_item]
        for cls, mask in ((T_RS_SHIP, loc[rs_item] == L_SHIP), (T_RS_OWNED, self.it_owned[rs_item]),
                          (T_RS_CHAR, (self.it_owned[rs_item] | (loc[rs_item] == L_CHAR)) & (self.it_kind[rs_item] != SKILL))):
            keys.append(_tkey(rf[mask], cls, rs_skill[mask]))
            vals.append(rs_item[mask])
        keys = np.concatenate(keys)
        vals = np.concatenate(vals)
        o = np.argsort(keys, kind="stable")
        self.tt_keys, self.tt_vals = keys[o], vals[o]

    def register_all(self):
        self.finish_items()
        self._register_generic()
        for fit in self.fits:
            self._register_python(fit)

    def _register_generic(self):
        """vectorised registration of all SDE modifiers of all items in the batch"""
        ds, tm = self.ds, self.ds.tm
        ti = self.it_ti
        lo, hi = ds.tm_ptr[ti], ds.tm_ptr[ti + 1]
        src, rows = _expand_ranges(lo, hi)
        src = src.astype(np.int64)
        seq = rows - lo[src]  # position of the row in the item's own effect/modifier order
        # template rows from mutation base types (effects the mutated type does not have itself)
        for i, bti, own in self.extra_rows:
            r = np.arange(ds.tm_ptr[bti], ds.tm_ptr[bti + 1])
            r = r[~np.isin(tm["eff"][r], list(own))]
            src = np.concatenate([src, np.full(len(r), i, np.int64)])
            seq = np.concatenate([seq, 100000 + np.arange(len(r))])  # base-type effects come after own
            rows = np.concatenate([rows, r])
        eff = tm["eff"][rows]
        ecat = tm["ecat"][rows].astype(np.int64)
        kind = self.it_kind[src]
        fit = self.it_fit[src]
        struct = self.fit_struct[fit]
        keep = kind != PROJECTED
        keep &= ~(struct & np.isin(kind, (DRONE, IMPLANT, BOOSTER)))
        sok = np.array([ds.e(n) for n in STRUCTURE_SKILL_EFFECT_NAMES])
        keep &= ~(struct & (kind == SKILL) & ~np.isin(eff, sok) & ~tm["allitem"][rows])
        # booster side effects only when selected; fighter abilities only when enabled
        fuc = tm["fuc"][rows]
        if fuc.any():
            allowed = {(i, e) for i in self.meta_items for e in self.meta[i]["side_effects"]}
            idx = np.nonzero(fuc)[0]
            ok = np.array([(int(src[j]), int(eff[j])) in allowed for j in idx], bool)
            keep[idx[~ok]] = False
        fi = np.nonzero((kind == FIGHTER) & (ecat != 0))[0]
        if len(fi):
            ok = np.array([int(eff[j]) in self.meta[int(src[j])]["fighter_abilities"] for j in fi], bool)
            keep[fi[~ok]] = False
        keep &= STATE_OK[np.clip(ecat, 0, 7), self.it_estate[src]] & (ecat <= 7)
        src, rows, eff, seq = src[keep], rows[keep], eff[keep], seq[keep]
        special = tm["special"][rows]
        sp = special != 0
        self.special_rows = {}
        for i, code, sq in zip(src[sp].tolist(), special[sp].tolist(), seq[sp].tolist()):
            self.special_rows.setdefault(int(self.it_fit[i]), []).append((i, code, sq))
        src, rows, eff, seq = src[~sp], rows[~sp], eff[~sp], seq[~sp]
        func = tm["func"][rows].astype(np.int64)
        dom = tm["dom"][rows].astype(np.int64)
        extra = tm["extra"][rows].copy()
        z = (extra == 0) & ((func == 3) | (func == 4))
        extra[z] = self.ds.t_id[self.it_ti[src[z]]]  # EXCT patch convention: skill 0 = the effect's owner
        fit = self.it_fit[src]
        # direct targets
        tgt_q, tgt_t = [], []
        r = np.arange(len(src))
        m = (dom == 0) & (func == 0)
        tgt_q.append(r[m]); tgt_t.append(src[m])
        m = dom == 3
        other = np.where(self.it_charge[src] >= 0, self.it_charge[src], self.it_parent[src])
        m &= other >= 0
        tgt_q.append(r[m]); tgt_t.append(other[m])
        shipdom = (dom == 1) | ((dom == 4) & self.fit_struct[fit])
        m = shipdom & (func == 0)
        tgt_q.append(r[m]); tgt_t.append(self.fit_ship[fit[m]])
        m = (dom == 2) & (func == 0)
        tgt_q.append(r[m]); tgt_t.append(self.fit_char[fit[m]])
        # table targets
        cls = np.full(len(src), -1, np.int64)
        x = np.zeros(len(src), np.int64)
        cls[shipdom & (func == 1)] = T_LOC_SHIP
        cls[shipdom & (func == 2)] = T_GRP_SHIP
        cls[shipdom & (func == 3)] = T_RS_SHIP
        cls[shipdom & (func == 4)] = T_RS_OWNED
        cls[(dom == 2) & (func == 1)] = T_LOC_CHAR
        cls[(dom == 2) & (func == 2)] = T_GRP_CHAR
        cls[(dom == 2) & ((func == 3) | (func == 4))] = T_RS_CHAR
        has_x = np.isin(cls, (T_GRP_SHIP, T_GRP_CHAR, T_RS_SHIP, T_RS_OWNED, T_RS_CHAR))
        x[has_x] = extra[has_x]
        jm = np.nonzero(cls >= 0)[0]
        q, t = _join(_tkey(fit[jm], cls[jm], x[jm]), self.tt_keys, self.tt_vals)
        tgt_q.append(jm[q]); tgt_t.append(t)
        q = np.concatenate(tgt_q)
        t = np.concatenate(tgt_t)
        rq = rows[q]
        cat = self.it_cat[src[q]]
        bastion = ds.e("moduleBonusBastionModule")
        if bastion:
            cat = np.where((eff[q] == bastion) & np.isin(tm["modified"][rq], HULL_RESONANCES), 6, cat)
        attr = tm["modified"][rq].astype(np.int64)
        self.mods.append({
            "tgt": t, "attr": attr, "op": tm["op"][rq].astype(np.int64),
            "pen": ~ds.attr_stack[attr] & ~np.isin(cat, EXEMPT_CATEGORIES),
            "kind": np.zeros(len(q), np.int64),
            "a": (src[q] << ATTR_BITS) | tm["modifying"][rq].astype(np.int64),
            "b": np.full(len(q), -1, np.int64), "c": np.full(len(q), -1, np.int64),
            "const": np.zeros(len(q)), "factor": np.ones(len(q)), "mul": np.zeros(len(q), bool),
            "src_item": src[q], "o1": src[q], "o2": seq[q]})

    # small per-fit registrations -------------------------------------------------
    def push(self, tgt, attr, op, kind, a=-1, b=-1, c=-1, const=0.0, factor=1.0, mul=False, src_item=-1, src_cat=0,
             order=None):
        """append one modifier row. `order` = (item, position) inside the item-effect registration pass;
        rows registered later (buffs, RAH) are ordered by insertion."""
        pen = (not self.ds.attr_stack[attr]) and src_cat not in (6, 8, 16, 20, 32, 65)
        o1, o2 = order if order is not None else (LATE_ORDER + len(self.small), 0)
        self.small.append((tgt, attr, op, pen, kind, a, b, c, const, factor, mul, src_item, o1, o2))

    @staticmethod
    def key(item, attr):
        return (item << ATTR_BITS) | attr

    def _register_python(self, fit):
        ds = self.ds
        a = ds.a
        K = self.key
        ship = fit.ship
        for i, sp, sq in self.special_rows.get(fit.index, ()):
            cat = self.meta[i]["category"]
            o = (i, sq)
            if sp in (SPECIAL_AB, SPECIAL_MWD):
                self.push(ship, 4, 2, SRC_ATTR, K(i, a("massAddition")), src_item=i, src_cat=cat, order=o)
                self.push(ship, a("maxVelocity"), 4, SRC_PROP, K(i, a("speedFactor")), K(i, a("speedBoostFactor")),
                          K(ship, 4), src_item=i, src_cat=cat, order=o)
                if sp == SPECIAL_MWD:
                    self.push(ship, a("signatureRadius"), 6, SRC_ATTR, K(i, a("signatureRadiusBonus")), src_item=i, src_cat=cat, order=o)
            elif sp == SPECIAL_MJD:
                self.push(ship, a("signatureRadius"), 6, SRC_ATTR, K(i, a("signatureRadiusBonusPercent")), src_item=i, src_cat=6, order=o)
            elif sp == SPECIAL_SLOT:
                for t, s in (("hiSlots", "hiSlotModifier"), ("medSlots", "medSlotModifier"), ("lowSlots", "lowSlotModifier")):
                    self.push(ship, a(t), 2, SRC_ATTR, K(i, a(s)), src_item=i, src_cat=cat, order=o)
            elif sp in FIGHTER_SELF:
                if self.meta[i]["kind"] == FIGHTER:
                    for t, s_, op in FIGHTER_SELF[sp]:
                        self.push(i, a(t), op, SRC_ATTR, K(i, a(s_)), src_item=i, src_cat=cat, order=o)
            elif sp == SPECIAL_HARDPOINT:
                for t, s in (("turretSlotsLeft", "turretHardPointModifier"), ("launcherSlotsLeft", "launcherHardPointModifier")):
                    self.push(ship, a(t), 2, SRC_ATTR, K(i, a(s)), src_item=i, src_cat=cat, order=o)
        for i in fit.modules:
            if self.meta[i]["state"] >= ACTIVE:
                self._register_local_special(fit, i)
        for i in fit.items:
            k = self.meta[i]["kind"]
            if k == PROJECTED:
                self._register_projected(fit, i)
            elif k == BEACON:
                self._register_beacon(fit, i)
        # explicit fleet buffs (aggregated per buff id)
        agg = {}
        for b in fit.req["fleet"]["buffs"]:
            info = ds.dbuffs.get(b["buff_id"])
            if info is None:
                fit.warnings.append(f"unknown warfare buff {b['buff_id']}")
                continue
            if b["buff_id"] not in agg:
                agg[b["buff_id"]] = b["value"]
            elif info.get("aggregate") == "Minimum":
                agg[b["buff_id"]] = min(agg[b["buff_id"]], b["value"])
            else:
                agg[b["buff_id"]] = max(agg[b["buff_id"]], b["value"])
        fit.explicit_buffs = agg  # applied in run() together with the bursts, sorted by buff id

    def _register_local_special(self, fit, i):
        """Pyfa 'active' handlers for local module effects that have no modifierInfo in the SDE (eos/effects.py,
        LGPL; re-expressed): superweapons/lances (speed, warp scramble status), Emergency Hull Energizer, Entosis
        Link, Micro Jump Field Generator, Warp Disruption Field Generator. Source category 6 = unpenalised."""
        ds = self.ds
        m = self.meta[i]
        effs = m["effects"]
        names = [ds.effect_name.get(e) for e, _ in effs]
        if not any(nm in LOCAL_SPECIAL for nm in names):
            return
        a = ds.a
        K = self.key
        ship = fit.ship
        cat = m["category"]
        # o2 = position of the effect's (would-be) template rows in the item's row order
        ti = m["ti"]
        r_eff = ds.tm["eff"][ds.tm_ptr[ti]:ds.tm_ptr[ti + 1]].tolist()
        for en, (eid, _) in enumerate(effs):
            nm = names[en]
            if nm not in LOCAL_SPECIAL:
                continue
            e = ds.eff_info.get(eid)
            if e is None or e["mods"]:
                continue
            before = {x for x, _ in effs[:en]}
            o = (i, sum(1 for x in r_eff if x in before))

            def push(tgt, attr, op, src_attr=None, const=None, src_cat=cat):
                if const is not None:
                    self.push(tgt, attr, op, SRC_CONST, const=const, src_item=i, src_cat=src_cat, order=o)
                else:
                    self.push(tgt, attr, op, SRC_ATTR, K(i, a(src_attr)), src_item=i, src_cat=src_cat, order=o)

            if nm in ("superWeaponAmarr", "superWeaponCaldari", "superWeaponGallente", "superWeaponMinmatar",
                      "doomsdaySlash", "doomsdayBeamDOT", "doomsdayConeDOT", "doomsdayHOG", "debuffLance"):
                push(ship, a("maxVelocity"), 6, "speedFactor")
                push(ship, a("warpScrambleStatus"), 2, "siegeModeWarpStatus")
            elif nm == "emergencyHullEnergizer":
                for t in ("Em", "Thermal", "Kinetic", "Explosive"):
                    push(ship, a(f"{t.lower()}DamageResonance"), 4, f"hull{t}DamageResonance")
            elif nm == "entosisLink":
                push(ship, a("disallowAssistance"), 7, "disallowAssistance", src_cat=6)
                for t in ("Gravimetric", "Magnetometric", "Radar", "Ladar"):
                    push(ship, a(f"scan{t}Strength"), 6, f"scan{t}StrengthPercent")
            elif nm in ("microJumpPortalDrive", "microJumpPortalDriveCapital"):
                push(ship, a("signatureRadius"), 6, "signatureRadiusBonusPercent")
            elif nm == "moduleBonusBreacherPodDamageControl":
                push(ship, a("breacherPodDamageResistance"), 6, "breacherPodActivatedDamageReceivedPercentage", src_cat=6)
            elif nm == "warpDisruptSphere":
                push(ship, a("disallowAssistance"), 7, const=1.0, src_cat=6)
                if m["charge"] is None:
                    push(ship, 4, 6, "massBonusPercentage", src_cat=6)
                    push(ship, a("signatureRadius"), 6, "signatureRadiusBonus", src_cat=6)
                    for t in fit.items:
                        mt = self.meta[t]
                        if mt is not None and mt["kind"] == MODULE and self.it_loc[t] == L_SHIP and \
                                ds.group_name.get(mt["group"]) == "Propulsion Module":
                            push(t, a("speedBoostFactor"), 6, "speedBoostFactorBonus", src_cat=6)
                            push(t, a("speedFactor"), 6, "speedFactorBonus", src_cat=6)

    def _register_beacon(self, fit, b):
        """Sansha / Drifter incursion system effects (Pyfa Effect4728 OffensiveDefensiveReduction, LGPL; re-expressed):
        unpenalised PostPercent of missile-charge and smartbomb damage, turret and drone damageMultiplier by
        systemEffectDamageReduction, and of the ship's armor/shield resonances by the beacon's resistance bonuses."""
        ds = self.ds
        m = self.meta[b]
        if not any(ds.effect_name.get(e) == "OffensiveDefensiveReduction" for e, _ in m["effects"]):
            return
        a = ds.a
        K = self.key
        o = (b, 0)
        red = K(b, a("systemEffectDamageReduction"))
        mls = ds.type_by_name.get("missile launcher operation") or 0
        gun = ds.type_by_name.get("gunnery") or 0
        for t in fit.items:
            mt = self.meta[t]
            if mt is None or not self.it_owned[t] or (self.it_loc[t] != L_SHIP and mt["kind"] != DRONE):
                continue
            k = mt["kind"]
            dmg = mult = False
            if k in (CHARGE, MODULE):
                req = self.custom_reqskills.get(t) or ds.t_reqskills[mt["ti"]]
                if k == CHARGE:
                    dmg = mls in req
                else:
                    dmg = ds.group_name.get(mt["group"]) == "Smart Bomb"
                    mult = gun in req
            elif k == DRONE:
                mult = True
            if dmg:
                for d in ("em", "thermal", "kinetic", "explosive"):
                    self.push(t, a(f"{d}Damage"), 6, SRC_ATTR, red, src_item=b, src_cat=6, order=o)
            if mult:
                self.push(t, a("damageMultiplier"), 6, SRC_ATTR, red, src_item=b, src_cat=6, order=o)
        ship = fit.ship
        for d in ("Em", "Thermal", "Kinetic", "Explosive"):
            for l in ("armor", "shield"):
                self.push(ship, a(f"{l}{d}DamageResonance"), 6, SRC_ATTR, K(b, a(f"{l}{d}DamageResistanceBonus")),
                          src_item=b, src_cat=6, order=o)

    def _register_projected(self, fit, i):
        ds = self.ds
        it = self.meta[i]
        ship = fit.ship
        K = self.key
        if it["state"] < ACTIVE:
            return
        base = ds.type_attrs(it["ti"])
        base.update(self.overrides.of_item(i))
        abil = it["fighter_abilities"]
        qty = float(max(it["quantity"], 1))
        a = ds.a

        def look(n):
            aid = a(n)
            return int(base[aid]) if aid in base else 0  # `as u32` of the base value

        for en, (eid, _) in enumerate(it["effects"]):
            e = ds.eff_info.get(eid)
            if e is None or (e["category"] not in (2, 3) and e["name"] != "ECMBurstJammer"
                             and not e["name"].startswith("doomsdayAOE")):
                continue
            if abil is not None and e["name"].startswith("fighterAbility") and eid not in abil:
                continue
            opt = base.get(e["range_attr"], 0.0) if e["range_attr"] is not None else 0.0
            fo = base.get(e["falloff_attr"], 0.0) if e["falloff_attr"] is not None else 0.0
            factor = range_factor(opt, fo, it["distance"], True)
            resist = e["resistance_attr"]
            if resist is None:
                if e["name"].startswith("fighterAbility"):
                    resist = look(e["name"] + "ResistanceID") or look(e["name"] + "RemoteResistanceID")
                else:
                    resist = look("remoteResistanceID")
            doff = a("disallowOffensiveModifiers")
            sv = self.overrides.get((ship, doff))
            if sv is None:
                sv = ds.type_attr(self.meta[ship]["ti"], doff)
            target_offense_ok = sv is None or sv == 0.0

            def push(tattr, sattr, op):
                self.push(ship, tattr, op, SRC_PROJ, K(i, sattr), -1, K(ship, resist) if resist else -1,
                          factor=factor, mul=op in (0, 4), src_item=i, src_cat=it["category"], order=(i, en))

            nm = e["name"]
            # burst projectors / Standup weapon disruptor stay engine-side even if a dataset revision gives them
            # modifiers (no AoE full-strength rule on the generic path)
            engine_side = nm.startswith("doomsdayAOE") or nm == "structureModuleEffectWeaponDisruption"
            if e["mods"] and not engine_side:
                for f, dom, mod_, mding, op, extra in e["mods"]:
                    if dom in (5, 6, 1) and f == 0:
                        push(mod_, mding, op)
                continue
            pb = lambda n: base.get(a(n), 0.0)  # noqa: E731
            if nm.startswith("doomsdayAOE") and nm != "doomsdayAOETrack":
                # burst projectors (Pyfa Effect6476-6482/6513): full strength on every ship in the AoE
                if nm in ("doomsdayAOEWeb", "doomsdayAOEPaint", "doomsdayAOEDamp"):
                    if target_offense_ok:
                        prs = {"doomsdayAOEWeb": (("maxVelocity", "speedFactor"),),
                               "doomsdayAOEPaint": (("signatureRadius", "signatureRadiusBonus"),)}.get(
                            nm, (("maxTargetRange", "maxTargetRangeBonus"), ("scanResolution", "scanResolutionBonus")))
                        for t_, sa_ in prs:
                            self.push(ship, a(t_), 6, SRC_PROJ, K(i, a(sa_)), -1, K(ship, resist) if resist else -1,
                                      factor=1.0, mul=False, src_item=i, src_cat=it["category"], order=(i, en))
                elif nm == "doomsdayAOENeut":
                    fit.proj_special.append(("drain", i, a("energyNeutralizerAmount"), a("duration"), 1.0, resist, 1.0))
                elif nm == "doomsdayAOEECM":
                    if target_offense_ok:
                        fit.proj_special.append(("ecm", i, False, 1.0, resist))
                elif nm not in ("doomsdayAOEBubble", "doomsdayAOEGuide"):
                    fit.warnings.append(f"projected effect '{nm}' not modelled yet")
                continue
            if nm == "fighterAbilityStasisWebifier":
                if target_offense_ok:
                    f = range_factor(pb("fighterAbilityStasisWebifierOptimalRange"), pb("fighterAbilityStasisWebifierFalloffRange"),
                                     it["distance"], True) * qty
                    self.push(ship, a("maxVelocity"), 6, SRC_PROJ, K(i, a("fighterAbilityStasisWebifierSpeedPenalty")), -1,
                              K(ship, resist) if resist else -1, factor=f, mul=False, src_item=i, src_cat=it["category"],
                              order=(i, en))
                continue
            if nm == "fighterAbilityWarpDisruption":
                if target_offense_ok and pb("fighterAbilityWarpDisruptionRange") >= (it["distance"] or 0.0):
                    self.push(ship, a("warpScrambleStatus"), 2, SRC_PROJ, K(i, a("fighterAbilityWarpDisruptionPointStrength")),
                              -1, K(ship, resist) if resist else -1, factor=qty, mul=False, src_item=i,
                              src_cat=it["category"], order=(i, en))
                continue
            if nm.startswith("remoteWebifier") or nm == "structureModuleEffectStasisWebifier":
                push(a("maxVelocity"), a("speedFactor"), 6)
            elif nm.startswith("remoteTargetPaint") or nm == "structureModuleEffectTargetPainter":
                push(a("signatureRadius"), a("signatureRadiusBonus"), 6)
            elif nm.startswith("remoteSensorDamp") or nm == "structureModuleEffectRemoteSensorDampener":
                push(a("maxTargetRange"), a("maxTargetRangeBonus"), 6)
                push(a("scanResolution"), a("scanResolutionBonus"), 6)
            elif nm in ("doomsdayAOETrack", "structureModuleEffectWeaponDisruption"):
                # AoE weapon disruption burst (full strength) / Standup Weapon Disruptor (range factor)
                if target_offense_ok:
                    tf = 1.0 if nm == "doomsdayAOETrack" else \
                        range_factor(pb("maxRange"), pb("falloffEffectiveness"), it["distance"], True)
                    gun = ds.type_by_name.get("gunnery") or 0
                    mls = ds.type_by_name.get("missile launcher operation") or 0
                    for t in fit.items:
                        mt = self.meta[t]
                        if mt is None or self.it_loc[t] != L_SHIP or not self.it_owned[t]:
                            continue
                        req = self.custom_reqskills.get(t) or ds.t_reqskills[mt["ti"]]
                        if mt["kind"] == MODULE and gun in req:
                            prs = (("trackingSpeedBonus", "trackingSpeed"), ("maxRangeBonus", "maxRange"),
                                   ("falloffBonus", "falloff"))
                        elif mt["kind"] == CHARGE and mls in req:
                            prs = (("aoeCloudSizeBonus", "aoeCloudSize"), ("aoeVelocityBonus", "aoeVelocity"),
                                   ("missileVelocityBonus", "maxVelocity"), ("explosionDelayBonus", "explosionDelay"))
                        else:
                            continue
                        for sa, ta in prs:
                            self.push(t, a(ta), 6, SRC_PROJ, K(i, a(sa)), -1, K(ship, resist) if resist else -1,
                                      factor=tf, mul=False, src_item=i, src_cat=it["category"], order=(i, en))
            elif nm in ("shipModuleTrackingDisruptor", "shipModuleGuidanceDisruptor", "shipModuleRemoteTrackingComputer",
                        "npcEntityWeaponDisruptor"):
                # Pyfa Effect6424 / Effect6423 / shipModuleRemoteTrackingComputer: the target's gunnery modules
                # (TD, remote tracking computer) / missile charges (GD), postPercent
                if nm == "shipModuleRemoteTrackingComputer":
                    da = a("disallowAssistance")
                    sb = self.overrides.get((ship, da))
                    if sb is None:
                        sb = ds.type_attr(self.meta[ship]["ti"], da)
                    allowed = sb is None or sb == 0.0
                else:
                    allowed = target_offense_ok
                if allowed:
                    if nm != "shipModuleGuidanceDisruptor":
                        skill, want_kind, pairs = "Gunnery", MODULE, (("trackingSpeedBonus", "trackingSpeed"),
                                                                      ("maxRangeBonus", "maxRange"),
                                                                      ("falloffBonus", "falloff"))
                    else:
                        skill, want_kind, pairs = "Missile Launcher Operation", CHARGE, (
                            ("aoeCloudSizeBonus", "aoeCloudSize"), ("aoeVelocityBonus", "aoeVelocity"),
                            ("missileVelocityBonus", "maxVelocity"), ("explosionDelayBonus", "explosionDelay"))
                    sk = ds.type_by_name.get(skill.lower()) or 0
                    if nm == "npcEntityWeaponDisruptor":  # TD drones (Pyfa Effect6694): full inside maxRange, else 0
                        tf = 0.0 if pb("maxRange") < (it["distance"] or 0.0) else 1.0
                    else:
                        tf = range_factor(pb("maxRange"), pb("falloffEffectiveness"), it["distance"], True)
                    for t in fit.items:
                        mt = self.meta[t]
                        if mt is None or mt["kind"] != want_kind or self.it_loc[t] != L_SHIP or not self.it_owned[t]:
                            continue
                        req = self.custom_reqskills.get(t) or ds.t_reqskills[mt["ti"]]
                        if sk not in req:
                            continue
                        for sa, ta in pairs:
                            self.push(t, a(ta), 6, SRC_PROJ, K(i, a(sa)), -1, K(ship, resist) if resist else -1,
                                      factor=tf, mul=False, src_item=i, src_cat=it["category"], order=(i, en))
            elif nm.startswith("remoteSensorBoost"):
                push(a("maxTargetRange"), a("maxTargetRangeBonus"), 6)
                push(a("scanResolution"), a("scanResolutionBonus"), 6)
                for t in ("Gravimetric", "Ladar", "Magnetometric", "Radar"):
                    push(a(f"scan{t}Strength"), a(f"scan{t}StrengthPercent"), 6)
            else:
                ps = self._proj_special(fit, i, nm, resist, base)
                if ps is not None:
                    fit.proj_special.extend(ps)
                elif nm not in DAMAGE_EFFECTS:
                    fit.warnings.append(f"projected effect '{nm}' not modelled yet")

    def _proj_special(self, fit, i, nm, resist, base):
        """projected effects that feed tank / capacitor stats instead of attributes (Pyfa: remote reps, addDrain)"""
        ds = self.ds
        a = ds.a
        it = self.meta[i]
        b = lambda n: base.get(a(n), 0.0)  # noqa: E731
        dist = it["distance"]
        falloff = lambda: range_factor(b("maxRange"), b("falloffEffectiveness"), dist, True)  # noqa: E731
        gate = lambda opt: 0.0 if opt < (dist or 0.0) else 1.0  # noqa: E731
        ship = fit.ship
        da = a("disallowAssistance")
        sb = self.overrides.get((ship, da))
        if sb is None:
            sb = ds.type_attr(self.meta[ship]["ti"], da)
        no_assist = sb is not None and sb != 0.0
        rep = lambda layer, amt, mult, factor: [] if no_assist else [("rep", i, layer, a(amt), mult, factor)]  # noqa: E731
        drain = lambda amt, dur, factor, sign: [("drain", i, a(amt), a(dur), factor, resist, sign)]  # noqa: E731
        c = it["charge"]
        paste = c is not None and ds.t_name[self.meta[c]["ti"]] == "Nanite Repair Paste"
        doff = a("disallowOffensiveModifiers")
        so = self.overrides.get((ship, doff))
        if so is None:
            so = ds.type_attr(self.meta[ship]["ti"], doff)
        no_offense = so is not None and so != 0.0
        ecm = lambda fighter, factor: [] if no_offense else [("ecm", i, fighter, factor, resist)]  # noqa: E731
        q = float(max(it["quantity"], 1))
        if nm == "fighterAbilityEnergyNeutralizer":
            f = range_factor(b("fighterAbilityEnergyNeutralizerOptimalRange"), b("fighterAbilityEnergyNeutralizerFalloffRange"), dist, True)
            return drain("fighterAbilityEnergyNeutralizerAmount", "fighterAbilityEnergyNeutralizerDuration", f * q, 1.0)
        if nm in ("remoteECMFalloff", "structureModuleEffectECM"):
            return ecm(False, falloff())
        if nm == "entityECMFalloff":
            return ecm(False, gate(b("ECMRangeOptimal")))
        if nm == "ECMBurstJammer":
            return ecm(False, gate(b("ecmBurstRange")))
        if nm == "fighterAbilityECM":
            f = range_factor(b("fighterAbilityECMRangeOptimal"), b("fighterAbilityECMRangeFalloff"), dist, True)
            return ecm(True, f * q)
        if nm in ("shipModuleRemoteShieldBooster", "shipModuleAncillaryRemoteShieldBooster"):
            return rep(0, "shieldBonus", 1.0, falloff())
        if nm in ("shipModuleRemoteArmorRepairer", "ShipModuleRemoteArmorMutadaptiveRepairer"):
            return rep(1, "armorDamageAmount", 1.0, falloff())
        if nm == "shipModuleAncillaryRemoteArmorRepairer":
            return rep(1, "armorDamageAmount", 3.0 if paste else 1.0, falloff())
        if nm == "shipModuleRemoteHullRepairer":
            return rep(2, "structureDamageAmount", 1.0, falloff())
        if nm == "npcEntityRemoteShieldBooster":
            return rep(0, "shieldBonus", 1.0, gate(b("maxRange")))
        if nm == "npcEntityRemoteArmorRepairer":
            return rep(1, "armorDamageAmount", 1.0, gate(b("maxRange")))
        if nm == "npcEntityRemoteHullRepairer":
            return rep(2, "structureDamageAmount", 1.0, gate(b("maxRange")))
        if nm == "shipModuleRemoteCapacitorTransmitter":
            return [] if no_assist else drain("powerTransferAmount", "duration", gate(b("maxRange")), -1.0)
        if nm == "energyNeutralizerFalloff":
            return drain("energyNeutralizerAmount", "duration", falloff(), 1.0)
        if nm == "energyNosferatuFalloff":
            return drain("powerTransferAmount", "duration", falloff(), 1.0)
        if nm == "structureEnergyNeutralizerFalloff":
            return drain("energyNeutralizerAmount", "duration", 1.0, 1.0)
        if nm == "entityEnergyNeutralizerFalloff":
            return drain("energyNeutralizerAmount", "energyNeutralizerDuration", gate(b("energyNeutralizerRangeOptimal")), 1.0)
        return None

    def attr_value(self, i, attr):
        """base value of an item attribute (override, else type value), None when absent"""
        v = self.overrides.get((i, attr))
        return v if v is not None else self.ds.type_attr(self.meta[i]["ti"], attr)

    def loc_ship_items(self, fit):
        return [i for i in fit.items if self.meta[i]["kind"] in (SHIP, MODULE, CHARGE)]

    def apply_buff(self, fit, bid, kind, a_key, const, source_item):
        info = self.ds.dbuffs.get(bid)
        if info is None:
            return
        op = info["op"]
        ship = fit.ship
        loc = self.loc_ship_items(fit)
        # Pyfa penalises most buffs; the abyssal weather resistance/HP/velocity buffs are not
        cat = 6 if bid in (90, 93, 94, 95, 96, 98, 99) else 0
        for at in info["item"]:
            self.push(ship, at, op, kind, a_key, const=const, src_item=source_item, src_cat=cat)
        # AoE cloud / weather buffs also hit drones that require the Drones skill (Pyfa fit.py commandBonus)
        dattrs = BUFF_DRONE_ATTRS.get(bid)
        if dattrs:
            ds = self.ds
            for d in fit.items:
                md = self.meta[d]
                if md is None or md["kind"] != DRONE:
                    continue
                req = self.custom_reqskills.get(d) or ds.t_reqskills[md["ti"]]
                if 3436 not in req:
                    continue
                for n in dattrs:
                    at = ds.a(n)
                    if at:
                        self.push(d, at, op, kind, a_key, const=const, src_item=source_item, src_cat=cat)
        for at in info["location"]:
            for t in loc:
                self.push(t, at, op, kind, a_key, const=const, src_item=source_item, src_cat=cat)
        for at, g in info["location_group"]:
            for t in loc:
                if self.meta[t]["group"] == g:
                    self.push(t, at, op, kind, a_key, const=const, src_item=source_item, src_cat=cat)
        for at, s in info["location_skill"]:
            for t in loc:
                req = self.custom_reqskills.get(t) or self.ds.t_reqskills[self.meta[t]["ti"]]
                if s in req:
                    self.push(t, at, op, kind, a_key, const=const, src_item=source_item, src_cat=cat)


def range_factor(optimal, falloff, distance, restricted):
    if distance is None:
        return 1.0
    if falloff > 0.0:
        if restricted and distance > optimal + 3.0 * falloff:
            return 0.0
        return 0.5 ** ((max(distance - optimal, 0.0) / falloff) ** 2)
    return 1.0 if distance <= optimal else 0.0


# ====================================================================== evaluation
MOD_COLS = ("tgt", "attr", "op", "pen", "kind", "a", "b", "c", "const", "factor", "mul", "src_item", "o1", "o2")


def _py_round2(x):
    """Python round(x, 2) (Pyfa): correctly rounded on the exact binary value, ties to even"""
    return round(x, 2) if math.isfinite(x) else x


class Evaluated:
    """values of every node of (a subset of) a batch"""

    def __init__(self, keys, val, base, full):
        # full[i]: item i has all its type attributes (those without a node are unmodified base values)
        self.keys, self.val, self.base, self.full = keys, val, base, full

    def lookup(self, keys):
        k = np.asarray(keys, np.int64)
        pos = np.searchsorted(self.keys, k)
        pos = np.minimum(pos, len(self.keys) - 1)
        found = self.keys[pos] == k if len(self.keys) else np.zeros(len(k), bool)
        return pos, found


def _small_table(rows):
    cols = list(zip(*rows))
    return {k: np.array(v, dtype=(bool if k in ("pen", "mul") else (np.float64 if k in ("const", "factor") else np.int64)))
            for k, v in zip(MOD_COLS, cols)}


def _mods_table(batch):
    """all modifier rows as one column table. batch.mods / batch.small only ever grow between evaluation
    passes, so the table of the previous pass is extended instead of rebuilt (row order is unchanged:
    all array parts, then all small rows)."""
    nm, ns = len(batch.mods), len(batch.small)
    hit = batch.__dict__.get("_mtab")
    if hit is not None and hit[0] == nm and hit[1] == ns:
        return hit[2]
    if hit is not None and hit[0] == nm and 0 < hit[1] + nm and hit[1] <= ns:
        # only small rows were added: append them
        M = hit[2]
        add = _small_table(batch.small[hit[1]:])
        M = {k: np.concatenate([M[k], add[k]]) for k in MOD_COLS}
    else:
        parts = list(batch.mods)
        if batch.small:
            parts.append(_small_table(batch.small))
        if not parts:
            M = {k: np.zeros(0, np.int64) for k in MOD_COLS}
        else:
            M = {k: np.concatenate([p[k] for p in parts]) for k in MOD_COLS}
    batch._mtab = (nm, ns, M)
    return M


def _needed_rows(batch, M, tgt_key):
    """Rows that write skill attributes are kept only if the written value can reach a non-skill item: the stats
    layer never reads skill nodes, so a skill bonus whose consumers do not exist in the fit (no matching module,
    no such ship bonus, ...) is dead work. Backward closure from the rows targeting non-skill items over source
    edges (a, b, c) and min/max cap edges of skill nodes. None = keep everything."""
    B = ATTR_BITS
    kind = batch.it_kind
    skill_tgt = kind[M["tgt"]] == SKILL
    if not skill_tgt.any():
        return None
    ds = batch.ds
    amask = (1 << B) - 1

    def skill_srcs(rows):
        parts = []
        for col in ("a", "b", "c"):
            x = M[col][rows]
            x = x[x >= 0]
            parts.append(x[kind[x >> B] == SKILL])
        return np.concatenate(parts)

    def with_caps(k):
        attr = k & amask
        out = [k]
        for tab in (ds.attr_min, ds.attr_max):
            ca = tab[attr].astype(np.int64)
            h = ca >= 0
            out.append(((k[h] >> B) << B) | ca[h])
        return np.concatenate(out)

    keep = ~skill_tgt
    needed = _sorted_unique(with_caps(skill_srcs(keep)))
    cand = np.nonzero(skill_tgt)[0]
    while len(cand) and len(needed):
        p = np.minimum(np.searchsorted(needed, tgt_key[cand]), len(needed) - 1)
        hit = needed[p] == tgt_key[cand]
        if not hit.any():
            break
        rows = cand[hit]
        keep[rows] = True
        cand = cand[~hit]
        new = with_caps(skill_srcs(rows))
        needed = _sorted_unique(np.concatenate([needed, new]))
    return keep


def evaluate(batch, fit_mask=None):
    """Evaluate every attribute of every fit (or of fits where fit_mask is True)."""
    ds = batch.ds
    B = ATTR_BITS
    M = _mods_table(batch)
    tgt_key = (M["tgt"] << B) | M["attr"]
    item_fit = batch.it_fit
    if fit_mask is not None:
        sel = fit_mask[item_fit[M["tgt"]]]
        M = {k: v[sel] for k, v in M.items()}
        tgt_key = tgt_key[sel]
    keep = _needed_rows(batch, M, tgt_key)
    if keep is not None:
        M = {k: v[keep] for k, v in M.items()}
        tgt_key = tgt_key[keep]
    # ---- node set
    full = np.isin(batch.it_kind, FULL_KINDS)
    if fit_mask is not None:
        full &= fit_mask[item_fit]
    fi = np.nonzero(full)[0]
    ti = batch.it_ti[fi]
    owner, pos = _expand_ranges(ds.t_attr_ptr[ti], ds.t_attr_ptr[ti + 1])
    base_attr = ds.t_attr_ids[pos].astype(np.int64)
    # unmodified base attributes are not materialised as nodes (Values serves them from the type table);
    # only those the evaluation itself changes without a modifier (min/max caps, cpu/power rounding) are kept
    keep = (ds.attr_min[base_attr] >= 0) | (ds.attr_max[base_attr] >= 0) | ds.attr_round[base_attr]
    base_keys = (fi[owner[keep]] << B) | base_attr[keep]
    ov_keys = np.array([(i << B) | a for i, a in batch.overrides], np.int64)
    ov_vals = np.array(list(batch.overrides.values()), np.float64)
    if fit_mask is not None and len(ov_keys):
        s = fit_mask[item_fit[ov_keys >> B]]
        ov_keys, ov_vals = ov_keys[s], ov_vals[s]
    sk_keys, sk_vals = [], []
    for f, first, tis, lv in batch.skill_blocks:
        if fit_mask is None or fit_mask[f]:
            sk_keys.append(((first + np.arange(len(tis), dtype=np.int64)) << B) | ATTR_SKILL_LEVEL)
            sk_vals.append(lv)
    sk_keys = np.concatenate(sk_keys) if sk_keys else np.zeros(0, np.int64)
    sk_vals = np.concatenate(sk_vals) if sk_vals else np.zeros(0)
    refs = [M["a"], M["b"], M["c"]]
    refs = [r[r >= 0] for r in refs]
    # skill-level nodes only where a modifier reads or writes them (refs / tgt_key); the others are never read
    # (their base value is still resolved from sk_keys for the nodes that exist)
    keys = _sorted_unique(np.concatenate([base_keys, ov_keys, tgt_key] + refs))
    n = len(keys)
    node_item = keys >> B
    node_attr = keys & ((1 << B) - 1)
    base = _resolve_base(batch, keys, node_item, node_attr, ov_keys, ov_vals, sk_keys, sk_vals)

    # ---- modifier -> node indices
    tgt = np.searchsorted(keys, tgt_key)

    def idx(k):
        out = np.full(len(k), -1, np.int64)
        m = k >= 0
        out[m] = np.searchsorted(keys, k[m])
        return out

    ia, ib, ic = idx(M["a"]), idx(M["b"]), idx(M["c"])
    kind = M["kind"]
    # ---- min/max caps
    cap_node = []
    for cap_attr_tab in (ds.attr_min, ds.attr_max):
        ca = cap_attr_tab[node_attr].astype(np.int64)
        has = ca >= 0
        ck = np.where(has, (node_item << B) | np.maximum(ca, 0), -1)
        ci = np.full(n, -1, np.int64)
        p = np.searchsorted(keys, ck[has])
        p = np.minimum(p, n - 1)
        ok = keys[p] == ck[has]
        ci[np.nonzero(has)[0][ok]] = p[ok]
        # fallback constant when the cap attribute is not a node (pruned/absent)
        fb = np.zeros(n)
        miss = np.nonzero(has)[0][~ok]
        if len(miss):
            fb[miss] = _resolve_base(batch, ck[miss], node_item[miss], ca[miss], ov_keys, ov_vals, sk_keys, sk_vals)
        cap_node.append((has, ci, fb))
    rnd = ds.attr_round[node_attr]

    # ---- levelise: longest path from unmodified nodes
    lev = np.zeros(n, np.int64)
    work = np.zeros(n, bool)
    work[tgt] = True
    for has, _, _ in cap_node:
        work |= has
    work |= rnd
    lev[work] = 1
    e_dst = [tgt[ia >= 0], tgt[ib >= 0], tgt[ic >= 0]]
    e_src = [ia[ia >= 0], ib[ib >= 0], ic[ic >= 0]]
    for has, ci, _ in cap_node:
        m = ci >= 0
        e_dst.append(np.nonzero(m)[0])
        e_src.append(ci[m])
    e_dst = np.concatenate(e_dst)
    e_src = np.concatenate(e_src)
    for _ in range(MAX_LEVELS):
        new = lev.copy()
        np.maximum.at(new, e_dst, lev[e_src] + 1)
        np.minimum(new, MAX_LEVELS, out=new)
        if np.array_equal(new, lev):
            break
        lev = new
    val = base.copy()
    if not work.any():
        return Evaluated(keys, val, base, full)
    mlev = lev[tgt]
    reg = np.lexsort((M["o2"], M["o1"]))  # registration order (like the reference engine)
    order = reg[np.argsort(mlev[reg], kind="stable")]
    mstarts = np.searchsorted(mlev[order], np.arange(MAX_LEVELS + 2))
    nodes_by_lev = np.argsort(lev, kind="stable")
    nstarts = np.searchsorted(lev[nodes_by_lev], np.arange(MAX_LEVELS + 2))
    stage = np.full(16, -1, np.int64)
    for s, op in enumerate(OP_STAGES):
        stage[op + 1] = s
    mstage = stage[M["op"] + 1]
    hig = ds.attr_hig
    for L in range(1, int(lev.max()) + 1):
        nodes = nodes_by_lev[nstarts[L]:nstarts[L + 1]]
        if len(nodes) == 0:
            continue
        mi = order[mstarts[L]:mstarts[L + 1]]
        v = val[nodes]
        if len(mi):
            t_loc = np.searchsorted(nodes, tgt[mi])  # nodes are sorted (stable argsort of ascending ids)
            k = kind[mi]
            sv = np.empty(len(mi))
            m0 = k == SRC_ATTR
            sv[m0] = val[ia[mi[m0]]]
            m1 = k == SRC_CONST
            sv[m1] = M["const"][mi[m1]]
            m2 = k == SRC_PROP
            if m2.any():
                mass = val[ic[mi[m2]]]
                with np.errstate(divide="ignore", invalid="ignore"):
                    pv = 1.0 + val[ia[mi[m2]]] / 100.0 * val[ib[mi[m2]]] / mass
                sv[m2] = np.where(mass == 0.0, 1.0, pv)
            m3 = k == SRC_PROJ
            if m3.any():
                j = mi[m3]
                f = M["factor"][j] * np.where(ic[j] >= 0, val[np.maximum(ic[j], 0)], 1.0)
                pv = val[ia[j]]
                sv[m3] = np.where(M["mul"][j], (pv - 1.0) * f + 1.0, pv * f)
            op = M["op"][mi]
            st = mstage[mi]
            nl = len(nodes)
            flat = st * nl + t_loc
            present = np.zeros(9 * nl, bool)
            present[flat] = True
            # assignments (PreAssign / PostAssign): highest (or lowest if not high_is_good) wins
            asg = (op == -1) | (op == 7)
            assign = np.full(9 * nl, -np.inf)
            if asg.any():
                h = hig[M["attr"][mi[asg]]]
                np.maximum.at(assign, flat[asg], np.where(h, sv[asg], -sv[asg]))
            # multiplicative operators -> factor per row; stacking penalty by rank inside its group
            mu = ~asg & (op != 2) & (op != 3)
            fac = np.ones(len(mi))
            seqk = np.zeros(len(mi))  # application order inside a (node, stage): unpenalised rows first
            if mu.any():
                mv = sv[mu]
                o = op[mu]
                safe = np.where(mv == 0.0, 1.0, mv)
                mm = np.where((o == 0) | (o == 4), mv, np.where((o == 1) | (o == 5), 1.0 / safe,
                                                                np.where(o == 6, 1.0 + mv / 100.0, 1.0)))
                pen = M["pen"][mi[mu]]
                f_mu = mm.copy()
                k_mu = np.zeros(len(mm))
                pidx = np.nonzero(pen & (mm != 1.0))[0]
                if len(pidx):
                    pm = mm[pidx]
                    neg = pm < 1.0
                    grp = flat[mu][pidx] * 2 + neg
                    srt = np.lexsort((np.arange(len(pm)), -np.abs(pm - 1.0), grp))
                    g_s = grp[srt]
                    first = np.r_[True, g_s[1:] != g_s[:-1]]
                    start = np.maximum.accumulate(np.where(first, np.arange(len(g_s)), 0))
                    rank = np.empty(len(pm))
                    rank[srt] = np.arange(len(g_s)) - start
                    f_mu[pidx] = 1.0 + (pm - 1.0) * _libm_exp(-(rank * rank) / PENALTY_DENOM)
                    k_mu[pidx] = 1.0 + neg + rank / (len(pm) + 1.0)
                # penalised rows with factor exactly 1 are dropped (no-op)
                k_mu[pen & (mm == 1.0)] = 3.0
                fac[mu] = f_mu
                seqk[mu] = k_mu
            attr_n = node_attr[nodes]
            hn = hig[attr_n]
            for s_, opv in enumerate(OP_STAGES):
                sl = slice(s_ * nl, (s_ + 1) * nl)
                pr = present[sl]
                if not pr.any():
                    continue
                sel = st == s_
                if opv in (-1, 7):
                    a = assign[sl]
                    a = np.where(hn, a, -a)
                    v = np.where(pr, a, v)
                elif opv == 2:
                    np.add.at(v, t_loc[sel], sv[sel])
                elif opv == 3:
                    np.subtract.at(v, t_loc[sel], sv[sel])
                else:
                    j = np.nonzero(sel)[0]
                    j = j[np.argsort(seqk[j], kind="stable")]  # rows are already in registration order
                    np.multiply.at(v, t_loc[j], fac[j])
        for has, ci, fb in cap_node:
            hs = has[nodes]
            if hs.any():
                c = ci[nodes]
                cv = np.where(c >= 0, val[np.maximum(c, 0)], fb[nodes])
                if has is cap_node[0][0]:
                    v = np.where(hs, np.where(cv > v, cv, v), v)
                else:
                    v = np.where(hs, np.where(cv < v, cv, v), v)
        r = rnd[nodes]
        if r.any():
            rr = np.nonzero(r)[0]
            v[rr] = [_py_round2(x) for x in v[rr].tolist()]
        val[nodes] = v
    return Evaluated(keys, val, base, full)


def _resolve_base(batch, keys, node_item, node_attr, ov_keys, ov_vals, sk_keys, sk_vals):
    """base value of (item, attr) nodes: override > skill level > type attribute > attribute default"""
    ds = batch.ds
    B = ATTR_BITS
    base = ds.attr_def[node_attr].copy()
    tk = (batch.it_ti[node_item] << B) | node_attr
    p = np.minimum(np.searchsorted(ds.ta_key, tk), len(ds.ta_key) - 1)
    ok = ds.ta_key[p] == tk
    base[ok] = ds.t_attr_vals[p[ok]]
    n = len(keys)
    for kk, vv in ((sk_keys, sk_vals), (ov_keys, ov_vals)):
        if len(kk) and n:
            # look the (fewer) override keys up in the sorted node keys; the first of equal keys wins
            o = np.argsort(kk, kind="stable")
            ks, vs = kk[o], vv[o]
            if len(ks) > 1:
                first = np.ones(len(ks), bool)
                first[1:] = ks[1:] != ks[:-1]
                ks, vs = ks[first], vs[first]
            p = np.minimum(np.searchsorted(keys, ks), n - 1)
            ok = keys[p] == ks
            base[p[ok]] = vs[ok]
    return base


class Values:
    """attribute access for one evaluated batch (Rust-like get / has / base semantics).
    Scalar access goes through small per-item dicts built lazily from the sorted node arrays."""

    def __init__(self, batch, ev):
        self.batch, self.ev, self.ds = batch, ev, batch.ds
        self._keys = ev.keys
        self._n = len(ev.keys)
        self._items = {}
        self._bases = {}
        self._full = ev.full.tolist()
        self._defs = batch.ds._attr_def_list
        self._n_def = len(self._defs)
        # item -> [lo, hi) range of its nodes
        it = ev.keys >> ATTR_BITS
        # item -> first node (keys are sorted by item): exclusive prefix sum of the per-item node counts
        starts = np.zeros(batch.n_items + 1, np.int64)
        if len(it):
            np.cumsum(np.bincount(it, minlength=batch.n_items)[:batch.n_items], out=starts[1:])
        self._starts = starts
        self._lists = None

    def _range(self, i):
        return int(self._starts[i]), int(self._starts[i + 1])

    def _type_dict(self, i):
        return dict(self.ds._tad_get(self.batch.it_ti[i])) if self._full[i] else {}

    def item_dict(self, i):
        """all evaluated attributes of one item {attr: value}"""
        d = self._items.get(i)
        if d is None:
            L = self._lists
            if L is None and len(self._items) >= 256:
                # many items are read (the stats pass): convert the node arrays to Python lists once
                # instead of slicing NumPy arrays per item
                L = self._lists = ((self._keys & ((1 << ATTR_BITS) - 1)).tolist(), self.ev.val.tolist(),
                                   self._starts.tolist(), self.batch.it_ti.tolist())
            if L is not None:
                attrs, vals, starts, tis = L
                lo, hi = starts[i], starts[i + 1]
                d = dict(self.ds._tad_get(tis[i])) if self._full[i] else {}
                d.update(zip(attrs[lo:hi], vals[lo:hi]))
            else:
                lo, hi = self._range(i)
                mask = (1 << ATTR_BITS) - 1
                d = self._type_dict(i)
                d.update(zip((self._keys[lo:hi] & mask).tolist(), self.ev.val[lo:hi].tolist()))
            self._items[i] = d
        return d

    def get(self, item, attr):
        d = self._items.get(item)
        if d is None:
            d = self.item_dict(item)
        v = d.get(attr)
        if v is not None:
            return v
        if 0 <= attr < self._n_def:
            return self._defs[attr]
        return 0.0

    def has(self, item, attr):
        return attr in self.item_dict(item)

    def base(self, item, attr):
        d = self._bases.get(item)
        if d is None:
            lo, hi = self._range(item)
            mask = (1 << ATTR_BITS) - 1
            d = self._type_dict(item)
            d.update(zip((self._keys[lo:hi] & mask).tolist(), self.ev.base[lo:hi].tolist()))
            self._bases[item] = d
        v = d.get(attr)
        return v if v is not None else self.ds.attr_default(attr)


BUFF_DRONE_ATTRS = {
    79: ("signatureRadius",),
    90: ("shieldEmDamageResonance", "armorEmDamageResonance", "emDamageResonance"),
    93: ("shieldExplosiveDamageResonance", "armorExplosiveDamageResonance", "explosiveDamageResonance"),
    95: ("shieldThermalDamageResonance", "armorThermalDamageResonance", "thermalDamageResonance"),
    99: ("shieldKineticDamageResonance", "armorKineticDamageResonance", "kineticDamageResonance"),
    94: ("shieldCapacity",),
    96: ("armorHP",),
    97: ("maxRange", "falloff"),
    98: ("maxVelocity",),
}

WARFARE_PAIRS = [(f"warfareBuff{k}ID", f"warfareBuff{k}Value") for k in range(1, 5)]


def _weather_beacons(batch, fit):
    ds = batch.ds
    out = []
    for i in fit.items:
        m = batch.meta[i]
        if m is None or m["kind"] != BEACON:
            continue
        for e, _ in m["effects"]:
            nm = ds.effect_name.get(e) or ""
            if nm.startswith("weather_") or nm.startswith("aoe_beacon_"):
                out.append(i)
                break
    return out


def run(batch):
    """register everything, resolve the evaluation-dependent effects (bursts, RAH), final evaluation"""
    ds = batch.ds
    batch.register_all()
    nf = len(batch.fits)
    # ---- local command bursts: warfareBuffNID of active modules (may be assigned by the charge)
    pairs = [(ds.a(i), ds.a(v)) for i, v in WARFARE_PAIRS]
    id_attrs = {p[0] for p in pairs}
    tgt_attrs = {}
    for part in batch.mods:
        m = np.isin(part["attr"], list(id_attrs))
        for t in part["tgt"][m].tolist():
            tgt_attrs.setdefault(t, True)
    for r in batch.small:
        if r[1] in id_attrs:
            tgt_attrs[r[0]] = True
    need = np.zeros(nf, bool)
    for fit in batch.fits:
        for i in fit.modules:
            if batch.meta[i]["state"] >= ACTIVE:
                ti = batch.meta[i]["ti"]
                if i in tgt_attrs or any(ds.type_attr(ti, a) is not None for a in id_attrs) or \
                        any((i, a) in batch.overrides for a in id_attrs):
                    need[fit.index] = True
    vals = Values(batch, evaluate(batch, need)) if need.any() else None
    for fit in batch.fits:
        fit.warnings.extend(getattr(fit, "booster_warnings", None) or [])
        offers = getattr(fit, "booster_offers", None) or []
        explicit = fit.explicit_buffs
        wb = _weather_beacons(batch, fit)
        if not need[fit.index] and not offers and not explicit and not wb:
            continue
        # Pyfa keeps per buff id the single strongest (|value|) source among the fit's own bursts and the
        # fleet booster fits; explicit fleet.buffs (already registered) override both.
        best = {}

        def offer(bid, v, src):
            old = best.get(bid)
            if old is None or abs(old[0]) < abs(v):
                best[bid] = (v, src)

        if need[fit.index]:
            for i in fit.modules:
                if batch.meta[i]["state"] < ACTIVE:
                    continue
                for ida, vala in pairs:
                    bid = int(vals.get(i, ida)) if vals.has(i, ida) else 0
                    if bid == 0 or bid in explicit:
                        continue
                    offer(bid, vals.get(i, vala), (SRC_ATTR, Batch.key(i, vala), 0.0, i))
        # abyssal weather / AoE cloud beacons: warfareBuff1/2 join the same pool
        for i in wb:
            for ida, vala in pairs[:2]:
                bv = batch.attr_value(i, ida)
                bid = int(bv) if bv is not None else 0
                if bid == 0 or bid in explicit:
                    continue
                v = batch.attr_value(i, vala) or 0.0
                offer(bid, v, (SRC_CONST, -1, v, i))
        for bid, v in offers:
            if bid == 0 or bid in explicit:
                continue
            offer(bid, v, (SRC_CONST, -1, v, fit.ship))
        for bid, v in explicit.items():
            best[bid] = (v, (SRC_CONST, -1, v, fit.ship))
        for bid in sorted(best):
            kind, akey, const, src_item = best[bid][1]
            batch.apply_buff(fit, bid, kind, akey, const, src_item)
    # ---- Reactive Armor Hardener adaptation (sequential per RAH, like the reference)
    eid = ds.e("adaptiveArmorHardener")
    if eid:
        rahs = {}
        for fit in batch.fits:
            r = [i for i in fit.modules if batch.meta[i]["state"] >= ACTIVE
                 and any(e == eid for e, _ in batch.meta[i]["effects"])]
            if r:
                rahs[fit.index] = r
        rnd = 0
        while rahs and any(len(v) > rnd for v in rahs.values()):
            mask = np.zeros(nf, bool)
            for f, v in rahs.items():
                if len(v) > rnd:
                    mask[f] = True
            vals = Values(batch, evaluate(batch, mask))
            for f in np.nonzero(mask)[0].tolist():
                _apply_rah(batch, batch.fits[f], rahs[f][rnd], vals)
            rnd += 1
    return Values(batch, evaluate(batch))


RAH_ATTRS = ("armorEmDamageResonance", "armorThermalDamageResonance", "armorKineticDamageResonance",
             "armorExplosiveDamageResonance")


def _apply_rah(batch, fit, m, vals):
    ds = batch.ds
    attrs = [ds.a(n) for n in RAH_ATTRS]
    ship = fit.ship
    req = fit.req
    disable = req["options"]["rah"] == "disable"
    dp = req["damage_pattern"] or {"em": 25.0, "thermal": 25.0, "kinetic": 25.0, "explosive": 25.0}
    pattern = [dp["em"], dp["thermal"], dp["kinetic"], dp["explosive"]]
    res = [vals.get(m, a) for a in attrs]
    if not disable:
        base = [pattern[k] * vals.get(ship, attrs[k]) for k in range(4)]
        shift = vals.get(m, ds.a("resistanceShiftAmount")) / 100.0
        cycles = []
        loop_start = -20
        for _ in range(50):
            t = [(k, base[k] * res[k], res[k]) for k in (0, 3, 2, 1)]  # tie order em, explosive, kinetic, thermal
            t.sort(key=lambda x: x[1])
            if t[2][1] == 0.0:
                c0, c1, c2 = 1.0 - t[0][2], 1.0 - t[1][2], 1.0 - t[2][2]
                c3 = -(c0 + c1 + c2)
            elif t[1][1] == 0.0:
                c0, c1 = 1.0 - t[0][2], 1.0 - t[1][2]
                c2 = c3 = -(c0 + c1) / 2.0
            else:
                c0, c1 = min(shift, 1.0 - t[0][2]), min(shift, 1.0 - t[1][2])
                c2 = c3 = -(c0 + c1) / 2.0
            res[t[0][0]] = t[0][2] + c0
            res[t[1][0]] = t[1][2] + c1
            res[t[2][0]] = t[2][2] + c2
            res[t[3][0]] = t[3][2] + c3
            hit = next((j for j, v in enumerate(cycles) if all(abs(res[k] - v[k]) <= 1e-6 for k in range(4))), None)
            if hit is not None:
                loop_start = hit
                break
            cycles.append(list(res))
        start = loop_start if loop_start >= 0 else max(len(cycles) - 20, 0)
        lp = cycles[start:]
        if lp:
            for k in range(4):
                x = sum(v[k] for v in lp) / len(lp) * 1000.0
                res[k] = float(np.sign(x) * np.floor(abs(x) + 0.5)) / 1000.0
    cat = batch.meta[m]["category"]
    for k in range(4):
        if not disable:
            batch.push(m, attrs[k], 7, SRC_CONST, const=res[k], src_item=m, src_cat=cat)
        batch.push(ship, attrs[k], 0, SRC_CONST, const=res[k], src_item=m, src_cat=cat)
