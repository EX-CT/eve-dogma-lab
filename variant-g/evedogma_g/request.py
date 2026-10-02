"""FitRequest v1 parsing (contract.md). Produces plain dicts with every default filled in.
Unknown fields are ignored; wrong types raise RequestError (-> BAD_REQUEST)."""

STATES = {"offline": 0, "online": 1, "active": 2, "overheated": 3}
STATE_NAMES = ["offline", "online", "active", "overheated"]
SLOTS = ("high", "mid", "low", "rig", "subsystem", "service")
SPOOL_TYPES = ("spool_scale", "cycle_scale", "time", "cycles")


class RequestError(Exception):
    def __init__(self, code, message, path=""):
        super().__init__(message)
        self.code, self.message, self.path = code, message, path


def _obj(v, path, default=None):
    if v is None:
        return {} if default is None else default
    if not isinstance(v, dict):
        raise RequestError("BAD_REQUEST", f"{path or '/'}: expected object", path)
    return v


def _list(v, path):
    if v is None:
        return []
    if not isinstance(v, list):
        raise RequestError("BAD_REQUEST", f"{path}: expected array", path)
    return v


def _uint(v, path, optional=False):
    if v is None and optional:
        return None
    if isinstance(v, bool) or not isinstance(v, (int, float)) or v < 0 or (isinstance(v, float) and not v.is_integer()):
        raise RequestError("BAD_REQUEST", f"{path}: expected unsigned integer", path)
    return int(v)


def _num(v, path, optional=False, default=0.0):
    if v is None:
        if optional:
            return None
        return default
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        raise RequestError("BAD_REQUEST", f"{path}: expected number", path)
    return float(v)


def _bool(v, path, default):
    if v is None:
        return default
    if not isinstance(v, bool):
        raise RequestError("BAD_REQUEST", f"{path}: expected boolean", path)
    return v


def _enum(v, allowed, path):
    if v is None:
        return None
    if v not in allowed:
        raise RequestError("BAD_REQUEST", f"{path}: unknown variant {v!r}", path)
    return v


def _spool(v, path):
    if v is None:
        return None
    v = _obj(v, path)
    return {"type": _enum(v.get("type"), SPOOL_TYPES, path + "/type") or _missing(path + "/type"),
            "amount": _num(v.get("amount"), path + "/amount", default=None) if "amount" in v else _missing(path + "/amount")}


def _missing(path):
    raise RequestError("BAD_REQUEST", f"{path}: missing field", path)


def _mutation(v, path):
    if v is None:
        return None
    v = _obj(v, path)
    attrs = _obj(v.get("attributes"), path + "/attributes")
    return {"base_type_id": _uint(v.get("base_type_id"), path + "/base_type_id"),
            "mutaplasmid_type_id": _uint(v.get("mutaplasmid_type_id"), path + "/mutaplasmid_type_id", True),
            "attributes": {str(k): _num(x, f"{path}/attributes/{k}") for k, x in sorted(attrs.items())}}


def _module(v, path, default_state=None):
    v = _obj(v, path)
    st = _enum(v.get("state"), STATES, path + "/state")
    return {"type_id": _uint(v.get("type_id"), path + "/type_id"),
            "slot": _enum(v.get("slot"), SLOTS, path + "/slot"),
            "state": STATES[st] if st is not None else default_state,
            "charge_type_id": _uint(v.get("charge_type_id"), path + "/charge_type_id", True),
            "mutation": _mutation(v.get("mutation"), path + "/mutation"),
            "spool": _spool(v.get("spool"), path + "/spool")}


def _drone(v, path):
    v = _obj(v, path)
    return {"type_id": _uint(v.get("type_id"), path + "/type_id"),
            "quantity": _uint(v.get("quantity", 1), path + "/quantity"),
            "active": _uint(v.get("active"), path + "/active", True),
            "mutation": _mutation(v.get("mutation"), path + "/mutation")}


def _fighter(f, path):
    f = _obj(f, path)
    return {"type_id": _uint(f.get("type_id"), f"{path}/type_id"),
            "quantity": _uint(f.get("quantity"), f"{path}/quantity", True),
            "active": _bool(f.get("active"), f"{path}/active", True),
            "abilities": None if f.get("abilities") is None else
            [_uint(x, f"{path}/abilities") for x in _list(f.get("abilities"), f"{path}/abilities")]}


def _resists(v, path):
    if v is None:
        return None
    v = _obj(v, path)
    return {k: _num(v.get(k), f"{path}/{k}") for k in ("em", "thermal", "kinetic", "explosive")}


def parse(req, path=""):
    if not isinstance(req, dict):
        raise RequestError("BAD_REQUEST", "request must be a JSON object", path)
    ship = req.get("ship")
    if ship is None:
        _missing(path + "/ship")
    ship = _obj(ship, path + "/ship")
    ch = _obj(req.get("character"), path + "/character")
    sk = _obj(ch.get("skills"), path + "/character/skills")
    dl = sk.get("default_level")
    levels = {}
    for k, x in _obj(sk.get("levels"), path + "/character/skills/levels").items():
        lv = _uint(x, f"{path}/character/skills/levels/{k}")
        if lv > 255:
            raise RequestError("BAD_REQUEST", f"{path}/character/skills/levels/{k}: out of range", path)
        levels[str(k)] = lv
    fleet = _obj(req.get("fleet"), path + "/fleet")
    env = _obj(req.get("environment"), path + "/environment")
    opts = _obj(req.get("options"), path + "/options")
    cs = _obj(opts.get("cap_sim"), path + "/options/cap_sim")
    tp = req.get("target_profile")
    if tp is not None:
        tpo = _obj(tp, path + "/target_profile")
        tp = {k: _num(tpo.get(k), f"{path}/target_profile/{k}") for k in ("em", "thermal", "kinetic", "explosive")}
        for k in ("signature_radius", "max_velocity", "radius"):
            tp[k] = _num(tpo.get(k), f"{path}/target_profile/{k}", optional=True)
    projected = []
    for i, p in enumerate(_list(req.get("projected"), path + "/projected")):
        pp = f"{path}/projected/{i}"
        p = _obj(p, pp)
        if "kind" not in p or not isinstance(p["kind"], str):
            _missing(pp + "/kind")
        projected.append({
            "kind": p["kind"],
            "module": _module(p["module"], pp + "/module") if p.get("module") is not None else None,
            "drone": _drone(p["drone"], pp + "/drone") if p.get("drone") is not None else None,
            "fit": parse(p["fit"], pp + "/fit") if p.get("fit") is not None else None,
            "fighter": _fighter(p["fighter"], pp + "/fighter") if p.get("fighter") is not None else None,
            "amount": _uint(p.get("amount", 1), pp + "/amount"),
            "distance_m": _num(p.get("distance_m"), pp + "/distance_m", optional=True)})
    inc = opts.get("include_attributes")
    if inc is not None and not isinstance(inc, str):
        raise RequestError("BAD_REQUEST", "/options/include_attributes: expected string", path)
    rah = opts.get("rah")
    if rah is not None and not isinstance(rah, str):
        raise RequestError("BAD_REQUEST", "/options/rah: expected string", path)
    sec = env.get("system_security")
    if sec is not None and not isinstance(sec, str):
        raise RequestError("BAD_REQUEST", "/environment/system_security: expected string", path)
    return {
        "ship": {"type_id": _uint(ship.get("type_id"), path + "/ship/type_id"),
                 "mode_type_id": _uint(ship.get("mode_type_id"), path + "/ship/mode_type_id", True)},
        "character": {"skills": {"default_level": _uint(dl, path + "/character/skills/default_level", True),
                                 "levels": levels},
                      "security_status": _num(ch.get("security_status"), path + "/character/security_status", True)},
        "modules": [_module(m, f"{path}/modules/{i}") for i, m in enumerate(_list(req.get("modules"), path + "/modules"))],
        "drones": [_drone(m, f"{path}/drones/{i}") for i, m in enumerate(_list(req.get("drones"), path + "/drones"))],
        "fighters": [_fighter(f, f"{path}/fighters/{i}") for i, f in enumerate(_list(req.get("fighters"), path + "/fighters"))],
        "implants": [_uint(x, f"{path}/implants/{i}") for i, x in enumerate(_list(req.get("implants"), path + "/implants"))],
        "boosters": [{"type_id": _uint(_obj(b, f"{path}/boosters/{i}").get("type_id"), f"{path}/boosters/{i}/type_id"),
                      "side_effects": [_uint(x, f"{path}/boosters/{i}/side_effects") for x in _list(b.get("side_effects"), "")]}
                     for i, b in enumerate(_list(req.get("boosters"), path + "/boosters"))],
        "cargo": [{"type_id": _uint(_obj(c, f"{path}/cargo/{i}").get("type_id"), f"{path}/cargo/{i}/type_id"),
                   "quantity": _uint(c.get("quantity", 1), f"{path}/cargo/{i}/quantity")}
                  for i, c in enumerate(_list(req.get("cargo"), path + "/cargo"))],
        "fleet": {"buffs": [{"buff_id": _uint(_obj(b, "").get("buff_id"), f"{path}/fleet/buffs/{i}/buff_id"),
                             "value": _num(b.get("value"), f"{path}/fleet/buffs/{i}/value")}
                            for i, b in enumerate(_list(fleet.get("buffs"), path + "/fleet/buffs"))],
                  "booster_fits": [parse(b, f"{path}/fleet/booster_fits/{i}") for i, b in
                                   enumerate(_list(fleet.get("booster_fits"), path + "/fleet/booster_fits"))]},
        "projected": projected,
        "environment": {"effect_type_ids": [_uint(x, f"{path}/environment/effect_type_ids/{i}") for i, x in
                                            enumerate(_list(env.get("effect_type_ids"), path + "/environment/effect_type_ids"))],
                        "system_security": sec},
        "damage_pattern": _resists(req.get("damage_pattern"), path + "/damage_pattern"),
        "target_profile": tp,
        "overrides": [{"type_id": _uint(_obj(o, "").get("type_id"), f"{path}/overrides/{i}/type_id"),
                       "attribute_id": _uint(o.get("attribute_id"), f"{path}/overrides/{i}/attribute_id"),
                       "value": _num(o.get("value"), f"{path}/overrides/{i}/value")}
                      for i, o in enumerate(_list(req.get("overrides"), path + "/overrides"))],
        "options": {"nos_no_target_cap": _bool(opts.get("nos_no_target_cap"), "/options/nos_no_target_cap", False),
                    "factor_reload": _bool(opts.get("factor_reload"), "/options/factor_reload", False),
                    "default_spool": _spool(opts.get("default_spool"), "/options/default_spool"),
                    "rah": rah, "include_attributes": inc,
                    "sources": _bool(opts.get("sources"), "/options/sources", False),
                    "validate": _bool(opts.get("validate"), "/options/validate", True),
                    "cap_sim": {"reload": _bool(cs.get("reload"), "/options/cap_sim/reload", False),
                                "stagger": _bool(cs.get("stagger"), "/options/cap_sim/stagger", False),
                                "max_time_s": _num(cs.get("max_time_s"), "/options/cap_sim/max_time_s", True)}},
    }
