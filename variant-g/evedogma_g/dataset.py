"""Dataset loading + derived NumPy cache.

The canonical input is the EX-CT dataset (`dataset-<build>.json.gz`, format exct-eve-dataset v1).
On first use it is flattened into column arrays (CSR tables per type) and pickled next to a sha256 of
the .gz, so later processes start in a few ms instead of re-parsing 7 MB of JSON.
"""
import gzip
import hashlib
import json
import marshal
import os
import pickle

import numpy as np

CACHE_VERSION = 8
ATTR_BITS = 14  # attribute ids < 16384 (max in SDE 3569502: 6465)
ATTR_MASK = (1 << ATTR_BITS) - 1

# effect ids handled by hand-written code ("specials": no modifierInfo in the SDE)
SPECIAL_NONE, SPECIAL_AB, SPECIAL_MWD, SPECIAL_MJD, SPECIAL_SLOT, SPECIAL_HARDPOINT = 0, 1, 2, 3, 4, 5
# fighter self abilities without modifierInfo (Pyfa hand-written handlers); only act on fighters
SPECIAL_F_MWD, SPECIAL_F_AB, SPECIAL_F_EVASIVE = 6, 7, 8
EFFECT_SKILL_EFFECT = 132


# large Python tables that one calc only touches a few entries of: stored marshalled per entry and decoded on
# demand (saves ~15 ms of every cold start); small ones are plain objects in the pickle
_LAZY_SEQ = ("t_effects", "t_reqskills")  # list indexed by dense type index
_LAZY_MAP = ("eff_info", "muta")  # dict with int keys
_LAZY_WHOLE = ("names_zh", "type_by_name", "t_meta", "t_mgroup", "group_cat", "cat_name")  # only used by name lookups, search, EFT export


def _pack_entries(values):
    blobs = [marshal.dumps(v) for v in values]
    off = np.zeros(len(blobs) + 1, np.int64)
    off[1:] = np.cumsum([len(b) for b in blobs])
    return b"".join(blobs), off


class LazySeq:
    """read-only list whose entries are decoded (and cached) on first access"""

    def __init__(self, packed):
        self._blob, self._off = packed
        self._cache = {}

    def __len__(self):
        return len(self._off) - 1

    def __getitem__(self, i):
        v = self._cache.get(i)
        if v is None:
            if i < 0:
                i += len(self)
            v = self._cache[i] = marshal.loads(self._blob[self._off[i]:self._off[i + 1]])
        return v

    def __iter__(self):
        return (self[i] for i in range(len(self)))


class LazyMap:
    """read-only int-keyed dict whose values are decoded (and cached) on first access"""

    def __init__(self, packed):
        keys, (self._blob, self._off) = packed
        self._pos = {k: j for j, k in enumerate(keys)}
        self._cache = {}

    def __len__(self):
        return len(self._pos)

    def __contains__(self, k):
        return k in self._pos

    def __iter__(self):
        return iter(self._pos)

    def get(self, k, default=None):
        v = self._cache.get(k)
        if v is None:
            j = self._pos.get(k)
            if j is None:
                return default
            v = self._cache[k] = marshal.loads(self._blob[self._off[j]:self._off[j + 1]])
        return v

    def __getitem__(self, k):
        v = self.get(k)
        if v is None and k not in self._pos:
            raise KeyError(k)
        return v

    def keys(self):
        return self._pos.keys()

    def items(self):
        return ((k, self.get(k)) for k in self._pos)

    def values(self):
        return (self.get(k) for k in self._pos)


def _pack(c):
    """cache form of the column dict (see _LAZY_*)"""
    c = dict(c)
    for k in _LAZY_SEQ:
        c[k] = ("lazyseq", _pack_entries(c[k]))
    for k in _LAZY_MAP:
        keys = list(c[k].keys())
        c[k] = ("lazymap", (keys, _pack_entries([c[k][x] for x in keys])))
    for k in _LAZY_WHOLE:
        c[k] = ("lazywhole", marshal.dumps(c[k]))
    return c


class Dataset:
    """Column-oriented view of the dataset. All per-type tables are indexed by a dense type index."""

    def __init__(self, c):
        lazy_whole = {}
        for k, v in c.items():
            if isinstance(v, tuple) and len(v) == 2 and isinstance(v[0], str) and v[0].startswith("lazy"):
                if v[0] == "lazyseq":
                    v = LazySeq(v[1])
                elif v[0] == "lazymap":
                    v = LazyMap(v[1])
                else:
                    lazy_whole[k] = v[1]
                    continue
            self.__dict__[k] = v
        self._lazy_whole = lazy_whole
        self.attr_id = self.attr_by_name.get  # name -> id (None if unknown)
        self._tad = {}  # type index -> {attr: base value} (lazy)
        self._attr_def_list = self.attr_def.tolist()

    def __getattr__(self, name):  # only called for missing attributes: the _LAZY_WHOLE tables
        blob = self.__dict__.get("_lazy_whole", {}).pop(name, None)
        if blob is None:
            raise AttributeError(name)
        v = self.__dict__[name] = marshal.loads(blob)
        return v

    # ---- helpers used all over the engine
    def a(self, name):
        return self.attr_by_name.get(name, 0)

    def e(self, name):
        return self.effect_by_name.get(name, 0)

    def tidx(self, type_id):
        if type_id is None or type_id < 0 or type_id >= len(self.type_index):
            return -1
        return int(self.type_index[type_id])

    def type_attr(self, ti, attr, default=None):
        """base attribute of a type (dense index) or `default`"""
        d = self._tad.get(ti)
        if d is None:
            d = self._tad[ti] = self._type_attrs(ti)
        return d.get(attr, default)

    def _type_attrs(self, ti):
        lo, hi = self.t_attr_ptr[ti], self.t_attr_ptr[ti + 1]
        return dict(zip(self.t_attr_ids[lo:hi].tolist(), self.t_attr_vals[lo:hi].tolist()))

    def _tad_get(self, ti):
        """shared (do not mutate) {attr: base value} of a type"""
        d = self._tad.get(ti)
        if d is None:
            d = self._tad[ti] = self._type_attrs(ti)
        return d

    def type_attrs(self, ti):
        """copy of a type's base attributes {attr: value}"""
        d = self._tad.get(ti)
        if d is None:
            d = self._tad[ti] = self._type_attrs(ti)
        return dict(d)

    def attr_default(self, attr):
        return self._attr_def_list[attr] if 0 <= attr < len(self._attr_def_list) else 0.0


def _sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        h.update(f.read())
    return h.hexdigest()


def _build(path):
    raw_bytes = open(path, "rb").read()
    js = gzip.decompress(raw_bytes) if raw_bytes[:2] == b"\x1f\x8b" else raw_bytes
    sha_json = hashlib.sha256(js).hexdigest()
    d = json.loads(js)
    if d.get("format") != "exct-eve-dataset" or d.get("format_version") != 1:
        raise ValueError(f"unsupported dataset format {d.get('format')} v{d.get('format_version')}")

    # ---- attributes
    attrs = {int(k): v for k, v in d["attributes"].items()}
    n_attr = max(attrs) + 1
    assert n_attr <= ATTR_MASK
    attr_def = np.zeros(n_attr)
    attr_stack = np.ones(n_attr, bool)
    attr_hig = np.ones(n_attr, bool)
    attr_min = np.full(n_attr, -1, np.int32)
    attr_max = np.full(n_attr, -1, np.int32)
    attr_round = np.zeros(n_attr, bool)
    attr_by_name, attr_name = {}, {}
    for i, a in attrs.items():
        attr_def[i] = a.get("default") or 0.0
        attr_stack[i] = a.get("stackable", True) is not False
        attr_hig[i] = a.get("high_is_good", True) is not False
        if a.get("min_attr") is not None:
            attr_min[i] = a["min_attr"]
        if a.get("max_attr") is not None:
            attr_max[i] = a["max_attr"]
        attr_by_name[a["name"]] = i
        attr_name[i] = a["name"]
        if a["name"] in ("cpu", "power", "cpuOutput", "powerOutput"):
            attr_round[i] = True

    # ---- effects
    effects = {int(k): v for k, v in d["effects"].items()}
    effect_by_name = {e["name"]: i for i, e in effects.items()}
    effect_name = {i: e["name"] for i, e in effects.items()}
    special = {}
    for nm, code in (("moduleBonusAfterburner", SPECIAL_AB), ("moduleBonusMicrowarpdrive", SPECIAL_MWD),
                     ("microJumpDrive", SPECIAL_MJD), ("slotModifier", SPECIAL_SLOT),
                     ("hardPointModifierEffect", SPECIAL_HARDPOINT)):
        if nm in effect_by_name:
            special[effect_by_name[nm]] = code
    for nm, code in (("fighterAbilityMicroWarpDrive", SPECIAL_F_MWD), ("fighterAbilityAfterburner", SPECIAL_F_AB),
                     ("fighterAbilityEvasiveManeuvers", SPECIAL_F_EVASIVE)):
        if nm in effect_by_name and not effects[effect_by_name[nm]].get("mods"):
            special[effect_by_name[nm]] = code
    eff_info = {}
    for i, e in effects.items():
        eff_info[i] = {
            "name": e["name"], "category": e.get("category") or 0,
            "duration_attr": e.get("duration_attr"), "discharge_attr": e.get("discharge_attr"),
            "range_attr": e.get("range_attr"), "falloff_attr": e.get("falloff_attr"),
            "resistance_attr": e.get("resistance_attr"),
            "fuc": e.get("fitting_usage_chance_attr") is not None,
            "mods": [tuple(m) for m in e.get("mods") or []],
        }

    # ---- groups / types
    groups = {int(k): v for k, v in d["groups"].items()}
    group_name = {k: (v.get("name") or "") for k, v in groups.items()}
    group_cat = {k: int(v.get("category") or 0) for k, v in groups.items()}
    cat_name = {int(k): (v.get("name") or "") for k, v in (d.get("categories") or {}).items()}
    types = {int(k): v for k, v in d["types"].items()}
    tids = sorted(types)
    n_t = len(tids)
    type_index = np.full(max(tids) + 1, -1, np.int32)
    type_index[tids] = np.arange(n_t, dtype=np.int32)
    t_id = np.array(tids, np.int64)
    t_group = np.zeros(n_t, np.int32)
    t_cat = np.zeros(n_t, np.int32)
    t_pub = np.zeros(n_t, bool)
    t_mass, t_vol, t_cap, t_rad = (np.zeros(n_t) for _ in range(4))
    t_name, t_effects, t_reqskills, t_slot, t_raw_fields = [], [], [], [], []
    a_ptr, a_ids, a_vals = [0], [], []
    REQ = [182, 183, 184, 1285, 1289, 1290]
    SLOT_EFF = {12: "high", 13: "mid", 11: "low", 2663: "rig", 3772: "subsystem", 6306: "service"}
    type_by_name = {}
    t_meta = {}  # dense type index -> meta_level (search results)
    t_mgroup = {}  # dense type index -> market group (EFT export drone order)
    for ti, tid in enumerate(tids):
        t = types[tid]
        if t.get("meta_level") is not None:
            t_meta[ti] = int(t["meta_level"])
        if t.get("market_group") is not None:
            t_mgroup[ti] = int(t["market_group"])
        t_group[ti] = t["group"]
        t_cat[ti] = t["category"]
        t_pub[ti] = bool(t.get("published"))
        t_mass[ti], t_vol[ti], t_cap[ti], t_rad[ti] = (t.get(k) or 0.0 for k in ("mass", "volume", "capacity", "radius"))
        nm = t.get("name") or ""
        t_name.append(nm)
        if t_pub[ti] or nm.lower() not in type_by_name:
            type_by_name[nm.lower()] = tid
        ta = {int(k): float(v) for k, v in (t.get("attrs") or {}).items()}
        t_raw_fields.append({a: ta[a] for a in (4, 38, 161, 162) if a in ta})
        for aid, v in ((4, t_mass[ti]), (38, t_cap[ti]), (161, t_vol[ti]), (162, t_rad[ti])):
            if v != 0.0 or aid not in ta:
                ta[aid] = float(v)
        for aid in sorted(ta):
            a_ids.append(aid)
            a_vals.append(ta[aid])
        a_ptr.append(len(a_ids))
        effs = [(int(e), bool(df)) for e, df in (t.get("effects") or [])]
        t_effects.append(effs)
        t_reqskills.append([int(ta[a]) for a in REQ if a in ta and int(ta[a]) != 0])
        sl = None
        for e, _ in effs:
            if e in SLOT_EFF:
                sl = SLOT_EFF[e]
                break
        t_slot.append(sl)

    # ---- modifier templates per type (CSR): one row per (effect, modifier); specials get one marker row
    cols = {k: [] for k in ("eff", "ecat", "edef", "func", "dom", "modified", "modifying", "op", "extra",
                            "special", "fuc", "allitem")}
    m_ptr = [0]
    for ti, tid in enumerate(tids):
        for eid, dflt in t_effects[ti]:
            if eid == EFFECT_SKILL_EFFECT or eid not in eff_info:
                continue
            e = eff_info[eid]
            allitem = all(m[1] == 0 for m in e["mods"])
            rows = []
            if eid in special:
                rows.append((-1, -1, 0, 0, 0, 0, special[eid]))
            else:
                for f, dom, mod_, mding, op, extra in e["mods"]:
                    if f >= 5 or op == 9 or dom in (5, 6) or op not in (-1, 0, 1, 2, 3, 4, 5, 6, 7):
                        continue
                    rows.append((f, dom, mod_, mding, op, extra, 0))
            for f, dom, mod_, mding, op, extra, sp in rows:
                cols["eff"].append(eid); cols["ecat"].append(e["category"]); cols["edef"].append(dflt)
                cols["func"].append(f); cols["dom"].append(dom); cols["modified"].append(mod_)
                cols["modifying"].append(mding); cols["op"].append(op); cols["extra"].append(extra)
                cols["special"].append(sp); cols["fuc"].append(e["fuc"]); cols["allitem"].append(allitem)
        m_ptr.append(len(cols["eff"]))
    dt = {"eff": np.int32, "ecat": np.int8, "edef": bool, "func": np.int8, "dom": np.int8, "modified": np.int32,
          "modifying": np.int32, "op": np.int8, "extra": np.int64, "special": np.int8, "fuc": bool, "allitem": bool}
    tm = {k: np.array(v, dt[k]) for k, v in cols.items()}

    t_attr_ids = np.array(a_ids, np.int32)
    t_attr_vals = np.array(a_vals, np.float64)
    t_attr_ptr = np.array(a_ptr, np.int64)
    # global (type, attr) lookup keys, sorted by construction (types ascending, attrs ascending)
    counts = np.diff(t_attr_ptr)
    ta_key = (np.repeat(np.arange(n_t, dtype=np.int64), counts) << ATTR_BITS) | t_attr_ids

    skills = [tid for tid in tids if types[tid]["category"] == 16]
    published_skills = np.array([tid for tid in skills if types[tid].get("published")], np.int64)
    modes_1306 = sorted((tid, (types[tid].get("name") or "").lower()) for tid in tids if types[tid]["group"] == 1306)
    dbuffs = {int(k): v for k, v in (d.get("dbuffs") or {}).items()}
    muta = {int(k): v for k, v in (d.get("mutaplasmids") or {}).items()}
    sec_types = {}
    for nm in ("hiSecModifier", "lowSecModifier", "nullSecModifier"):
        aid = attr_by_name.get(nm)
        if aid is not None:
            sec_types[nm] = np.unique(np.repeat(np.arange(n_t), counts)[t_attr_ids == aid])
    return {
        "cache_version": CACHE_VERSION, "sha256": sha_json, "build": d["sde"]["build"],
        "release_date": d["sde"].get("release_date"),
        "attr_def": attr_def, "attr_stack": attr_stack, "attr_hig": attr_hig, "attr_min": attr_min,
        "attr_max": attr_max, "attr_round": attr_round, "attr_by_name": attr_by_name, "attr_name": attr_name,
        "eff_info": eff_info, "effect_by_name": effect_by_name, "effect_name": effect_name,
        "group_name": group_name, "type_index": type_index, "t_id": t_id, "t_group": t_group, "t_cat": t_cat,
        "t_pub": t_pub, "t_mass": t_mass, "t_vol": t_vol, "t_cap": t_cap, "t_rad": t_rad, "t_name": t_name,
        "t_effects": t_effects, "t_raw_fields": t_raw_fields, "t_reqskills": t_reqskills, "t_slot": t_slot,
        "t_attr_ptr": t_attr_ptr, "t_attr_ids": t_attr_ids, "t_attr_vals": t_attr_vals, "ta_key": ta_key,
        "tm": tm, "tm_ptr": np.array(m_ptr, np.int64), "published_skills": published_skills,
        "modes_1306": modes_1306, "dbuffs": dbuffs, "muta": muta, "type_by_name": type_by_name, "t_meta": t_meta, "t_mgroup": t_mgroup, "group_cat": group_cat,
        "cat_name": cat_name,
        "sec_types": sec_types, "names_zh": (d.get("names") or {}).get("zh", {}),
    }


def default_cache_dir():
    return os.environ.get("EVE_DOGMA_G_CACHE") or os.path.join(
        os.environ.get("XDG_CACHE_HOME") or os.path.expanduser("~/.cache"), "eve-dogma-g")


def load(path):
    """Load the dataset, using/refreshing the pickle cache keyed by the sha256 of the dataset file."""
    key = _sha256_file(path)
    cdir = default_cache_dir()
    cpath = os.path.join(cdir, f"{key[:16]}-v{CACHE_VERSION}.pkl")
    try:
        with open(cpath, "rb") as f:
            c = pickle.load(f)
        if c.get("cache_version") == CACHE_VERSION and c.get("src_sha256") == key:
            return Dataset(c)
    except (OSError, pickle.UnpicklingError, EOFError, AttributeError):
        pass
    c = _pack(_build(path))
    c["src_sha256"] = key
    try:
        os.makedirs(cdir, exist_ok=True)
        tmp = cpath + f".{os.getpid()}.tmp"
        with open(tmp, "wb") as f:
            pickle.dump(c, f, protocol=pickle.HIGHEST_PROTOCOL)
        os.replace(tmp, cpath)
    except OSError:
        pass
    return Dataset(c)
