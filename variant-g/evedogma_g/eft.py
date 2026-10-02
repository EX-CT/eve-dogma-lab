"""EFT fitting text import/export (same behaviour and output as the reference engine's eft module).

parse(ds, text) -> FitRequest dict (serialised like the reference: every field present, keys sorted)
export(ds, req, name) -> EFT text
Errors raise EftError (CLI: message on stderr, exit 2; RPC: {"error": {"code": "EFT_PARSE", ...}})."""
import decimal

from .request import RequestError, parse as parse_request

SLOT_EFFECTS = {12: "high", 13: "mid", 11: "low", 2663: "rig", 3772: "subsystem", 6306: "service"}
SLOT_ORDER = ("low", "mid", "high", "rig", "subsystem", "service")


class EftError(Exception):
    pass


def _lines(text):
    """Rust str::lines: split on \\n, strip one trailing \\r, no final empty line"""
    parts = text.split("\n")
    if parts and parts[-1] == "":
        parts.pop()
    return [p[:-1] if p.endswith("\r") else p for p in parts]


def _type_by_name(ds, name):
    return ds.type_by_name.get(name.strip().lower())


def _rust_float(s):
    """str::parse::<f64> (no '_' separators, no surrounding whitespace)"""
    if "_" in s or s != s.strip() or not s:
        return None
    try:
        return float(s)
    except ValueError:
        return None


def _rust_uint(s):
    """str::parse::<u32>"""
    if not s or not all("0" <= c <= "9" for c in s.lstrip("+")) or s.lstrip("+") == "" or s.count("+") > 1:
        return None
    v = int(s)
    return v if v < 2 ** 32 else None


def fmt_f64(v):
    """Rust `{}` Display of an f64: shortest round-trip digits, never an exponent, no trailing .0"""
    if v != v:
        return "NaN"
    if v in (float("inf"), float("-inf")):
        return "inf" if v > 0 else "-inf"
    r = repr(v)
    if "e" in r or "E" in r:
        r = format(decimal.Decimal(r), "f")
    if r.endswith(".0"):
        r = r[:-2]
    if r == "-0":
        return "-0"
    return r


def infer_slot(ds, ti):
    for e, _ in ds.t_effects[ti]:
        s = SLOT_EFFECTS.get(e)
        if s:
            return s
    return None


def _mut_ref(line):
    l = line.rstrip()
    if l.endswith("]"):
        p = l.rfind(" [")
        if p >= 0:
            n = _rust_uint(l[p + 2:-1])
            if n is not None:
                return l[:p].rstrip(), n
    return l, None


def _is_head(l):
    t = l.strip()
    if not t.startswith("["):
        return False
    e = t.find("]")
    return e >= 0 and _rust_uint(t[1:e]) is not None


def _parse_mutations(ds, text):
    lines = _lines(text)
    out = {}
    first = next((k for k, l in enumerate(lines) if _is_head(l)), len(lines))
    i = first
    while i < len(lines):
        t = lines[i].strip()
        if not _is_head(t):
            i += 1
            continue
        e = t.find("]")
        n = int(t[1:e])
        base_name = t[e + 1:].strip()
        base = _type_by_name(ds, base_name)
        if base is None:
            raise EftError(f"unknown mutated base '{base_name}'")
        m = {"base_type_id": base, "mutaplasmid_type_id": None, "attributes": {}}
        i += 1
        while i < len(lines) and not _is_head(lines[i]):
            l = lines[i].strip()
            i += 1
            if not l:
                continue
            if m["mutaplasmid_type_id"] is None:
                mp = _type_by_name(ds, l)
                if mp is None:
                    raise EftError(f"unknown mutaplasmid '{l}'")
                m["mutaplasmid_type_id"] = mp
                continue
            for kv in l.split(","):
                kv = kv.strip()
                p = kv.rfind(" ")
                if p < 0:
                    continue
                aid = ds.a(kv[:p].strip())
                if aid:
                    v = _rust_float(kv[p + 1:].strip())
                    if v is not None:
                        m["attributes"][str(aid)] = v
        out[n] = m
    return out, first


def _mutated_type(ds, m):
    if m["mutaplasmid_type_id"] is not None:
        mu = ds.muta.get(m["mutaplasmid_type_id"])
        if mu is not None:
            for x in mu.get("mapping") or []:
                if m["base_type_id"] in x.get("inputs", ()):
                    return x["output"]
    return m["base_type_id"]


def _copy_mut(m):
    return None if m is None else {"base_type_id": m["base_type_id"], "mutaplasmid_type_id": m["mutaplasmid_type_id"],
                                   "attributes": dict(m["attributes"])}


def _tinfo(ds, tid, name):
    ti = ds.tidx(tid)
    if ti < 0:
        raise EftError(f"unknown item '{name}'")
    return ti


def _default_request(ship):
    return {
        "schema_version": 1,
        "ship": {"type_id": ship, "mode_type_id": None},
        "character": {"skills": {"default_level": None, "levels": {}}, "security_status": None},
        "modules": [], "drones": [], "fighters": [], "implants": [], "boosters": [], "cargo": [],
        "fleet": {"buffs": [], "booster_fits": []},
        "projected": [],
        "environment": {"effect_type_ids": [], "system_security": None},
        "damage_pattern": None, "target_profile": None, "overrides": [],
        "options": {"nos_no_target_cap": False, "factor_reload": False, "default_spool": None, "rah": None,
                    "include_attributes": None, "sources": False, "validate": True,
                    "cap_sim": {"reload": False, "stagger": False, "max_time_s": None}},
    }


def parse(ds, text):
    muts, first_mut = _parse_mutations(ds, text)
    body = [l.strip() for l in _lines(text)[:first_mut]]
    body = [l for l in body if l]
    if not body:
        raise EftError("empty EFT")
    h = body[0].lstrip("[").rstrip("]")
    ship_name = h.split(",")[0].strip()
    ship = _type_by_name(ds, ship_name)
    if ship is None:
        raise EftError(f"unknown ship '{ship_name}'")
    req = _default_request(ship)
    for line in body[1:]:
        if line.startswith("[Empty"):
            continue
        offline = False
        for suf in ("/OFFLINE", "/offline"):
            if line.endswith(suf):
                line, offline = line[:-len(suf)].strip(), True
                break
        line, mref = _mut_ref(line)
        mutation = None
        if mref is not None:
            if mref not in muts:
                raise EftError(f"mutation [{mref}] not defined")
            mutation = muts[mref]
        # "Name xN" -> drone / fighter / cargo
        pos = line.rfind(" x")
        if pos >= 0:
            n = _rust_uint(line[pos + 2:].strip())
            if n is not None:
                name = line[:pos].strip()
                tid = _type_by_name(ds, name)
                if tid is None:
                    raise EftError(f"unknown item '{name}'")
                if mutation is not None:
                    tid = _mutated_type(ds, mutation)
                ti = _tinfo(ds, tid, name)
                cat = int(ds.t_cat[ti])
                if cat == 18:
                    req["drones"].append({"type_id": tid, "quantity": n, "active": n, "mutation": _copy_mut(mutation)})
                elif cat == 87:
                    req["fighters"].append({"type_id": tid, "quantity": n, "active": True, "abilities": None})
                else:
                    req["cargo"].append({"type_id": tid, "quantity": n})
                continue
        name, _, charge = line.partition(",")
        name = name.strip()
        charge = charge.strip() if "," in line else None
        tid = _type_by_name(ds, name)
        if tid is None:
            raise EftError(f"unknown item '{name}'")
        if mutation is not None:
            tid = _mutated_type(ds, mutation)
        ti = _tinfo(ds, tid, name)
        cat = int(ds.t_cat[ti])
        if cat == 20:
            if ds.type_attr(ti, 1087) is not None:  # boosterness
                req["boosters"].append({"type_id": tid, "side_effects": []})
            else:
                req["implants"].append(tid)
        elif cat == 18:
            req["drones"].append({"type_id": tid, "quantity": 1, "active": 1, "mutation": _copy_mut(mutation)})
        elif cat == 8:
            req["cargo"].append({"type_id": tid, "quantity": 1})
        else:
            if int(ds.t_group[ti]) == 1306:  # T3D mode
                req["ship"]["mode_type_id"] = tid
                continue
            slot = infer_slot(ds, ti)
            charge_id = None
            if charge is not None:
                charge_id = _type_by_name(ds, charge)
                if charge_id is None:
                    raise EftError(f"unknown charge '{charge}'")
            active_capable = any((ds.eff_info.get(e) or {}).get("category") == 1 for e, _ in ds.t_effects[ti]) or \
                (ds.type_attr(ti, 6) or 0.0) != 0.0
            if offline:
                state = "offline"
            elif active_capable and slot not in ("rig", "subsystem"):
                state = "active"
            else:
                state = "online"
            req["modules"].append({"type_id": tid, "slot": slot, "state": state, "charge_type_id": charge_id,
                                   "mutation": _copy_mut(mutation), "spool": None})
    return req


def export(ds, req, name="EXCT fit"):
    """req: a FitRequest dict (raw JSON); validated with the normal request parser first"""
    r = parse_request(req)  # raises RequestError (BAD_REQUEST)

    def n(tid):
        ti = ds.tidx(tid)
        return ds.t_name[ti] if ti >= 0 else str(tid)

    out = [f"[{n(r['ship']['type_id'])}, {name}]\n"]
    muts = []

    def tag(m):
        if m is None:
            return ""
        muts.append(m)
        return f" [{len(muts)}]"

    raw_mods = req.get("modules") or []
    for slot in SLOT_ORDER:
        any_ = False
        for k, m in enumerate(r["modules"]):
            s = m["slot"]
            if s is None:
                ti = ds.tidx(m["type_id"])
                s = infer_slot(ds, ti) if ti >= 0 else None
            if s != slot:
                continue
            any_ = True
            mu = m["mutation"]
            out.append(n(mu["base_type_id"]) if mu is not None else n(m["type_id"]))
            if m["charge_type_id"] is not None:
                out.append(f", {n(m['charge_type_id'])}")
            if raw_mods[k].get("state") == "offline":
                out.append(" /OFFLINE")
            out.append(tag(mu))
            out.append("\n")
        if any_:
            out.append("\n")
    for d in r["drones"]:
        mu = d["mutation"]
        nm = mu["base_type_id"] if mu is not None else d["type_id"]
        out.append(f"{n(nm)} x{d['quantity']}{tag(mu)}\n")
    raw_f = req.get("fighters") or []
    for k, f in enumerate(r["fighters"]):
        q = raw_f[k].get("quantity") if isinstance(raw_f[k], dict) else None
        out.append(f"{n(f['type_id'])} x{q if q is not None else 1}\n")
    if r["implants"] or r["boosters"]:
        out.append("\n")
        for i in r["implants"]:
            out.append(f"{n(i)}\n")
        for b in r["boosters"]:
            out.append(f"{n(b['type_id'])}\n")
    if r["cargo"]:
        out.append("\n")
        for c in r["cargo"]:
            out.append(f"{n(c['type_id'])} x{c['quantity']}\n")
    if muts:
        out.append("\n")
        for k, m in enumerate(muts):
            out.append(f"[{k + 1}] {n(m['base_type_id'])}\n")
            if m.get("mutaplasmid_type_id") is not None:
                out.append(f"  {n(m['mutaplasmid_type_id'])}\n")
            kv = []
            for a in sorted(m.get("attributes") or {}):
                v = m["attributes"][a]
                try:
                    an = ds.attr_name.get(int(a), a)
                except ValueError:
                    an = a
                kv.append(f"{an} {fmt_f64(float(v))}")
            if kv:
                out.append("  " + ", ".join(kv) + "\n")
    return "".join(out)


__all__ = ["EftError", "parse", "export", "RequestError"]
