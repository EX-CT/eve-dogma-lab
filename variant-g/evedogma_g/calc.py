"""Public API: calc_many(ds, requests) evaluates many FitRequests as one NumPy batch."""
import json

from . import engine
from .request import RequestError, parse
from .stats import FitStats


def _err(code, message, path=""):
    return {"error": {"code": code, "message": message, "path": path}}


def _debug_err(e):
    """Rust Debug formatting of the reference EngineError (used in warnings)"""
    return f'EngineError {{ code: "{e.code}", message: "{e.message}", path: "{e.path}" }}'


def _batch(ds, parsed):
    """parsed requests -> (batch, values, fits-or-errors). Fleet booster fits of every request are evaluated
    first as one extra batch; their active burst modules become constant buff offers."""
    boosters = []  # (owner request index, booster index, request)
    for k, p in enumerate(parsed):
        if isinstance(p, RequestError):
            continue
        for j, b in enumerate(p["fleet"]["booster_fits"]):
            bb = dict(b)
            bb["fleet"] = {"buffs": b["fleet"]["buffs"], "booster_fits": []}
            boosters.append((k, j, bb))
    offers = {}
    warn = {}
    if boosters:
        bbatch, bvals, bfits = _batch(ds, [b[2] for b in boosters])
        pairs = [(ds.a(i), ds.a(v)) for i, v in engine.WARFARE_PAIRS]
        for (k, j, _), f in zip(boosters, bfits):
            if isinstance(f, RequestError):
                warn.setdefault(k, []).append(f"fleet.booster_fits[{j}]: {_debug_err(f)}")
                continue
            lst = offers.setdefault(k, [])
            for i in f.modules:
                if bbatch.meta[i]["state"] < engine.ACTIVE:
                    continue
                for ida, vala in pairs:
                    bid = int(bvals.get(i, ida)) if bvals.has(i, ida) else 0
                    if bid:
                        lst.append((bid, bvals.get(i, vala)))
    batch = engine.Batch(ds)
    fits = []
    for k, p in enumerate(parsed):
        if isinstance(p, RequestError):
            fits.append(p)
            continue
        try:
            f = batch.add_fit(p)
        except RequestError as e:
            fits.append(e)
            continue
        f.booster_offers = offers.get(k, [])
        f.booster_warnings = warn.get(k, [])
        fits.append(f)
    vals = engine.run(batch) if batch.fits else None
    return batch, vals, fits


def calc_many(ds, reqs):
    """reqs: list of parsed-JSON request objects (dicts) -> list of response dicts (same order)"""
    parsed = []
    for r in reqs:
        try:
            parsed.append(parse(r))
        except RequestError as e:
            parsed.append(e)
    batch, vals, fits = _batch(ds, parsed)
    out = []
    for f in fits:
        if isinstance(f, RequestError):
            out.append(_err(f.code, f.message, f.path))
        else:
            out.append(FitStats(batch, f, vals).compute())
    return out


def calc(ds, req):
    return calc_many(ds, [req])[0]


def dumps(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False)


def calc_json_lines(ds, lines, chunk=256):
    """JSONL in -> JSONL out, evaluated in batches of `chunk` requests"""
    res = []
    for s in range(0, len(lines), chunk):
        part = lines[s:s + chunk]
        reqs, bad = [], {}
        for j, line in enumerate(part):
            try:
                reqs.append(json.loads(line))
            except json.JSONDecodeError as e:
                bad[j] = _err("BAD_JSON", str(e))
                reqs.append(None)
        good = [j for j in range(len(part)) if j not in bad]
        outs = calc_many(ds, [reqs[j] for j in good])
        m = dict(zip(good, outs))
        m.update(bad)
        res.extend(dumps(m[j]) for j in range(len(part)))
    return res
