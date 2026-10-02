"""Public API: calc_many(ds, requests) evaluates many FitRequests as one NumPy batch."""
import json

from . import engine
from .request import RequestError, parse
from .stats import FitStats


def _err(code, message, path=""):
    return {"error": {"code": code, "message": message, "path": path}}


def calc_many(ds, reqs):
    """reqs: list of parsed-JSON request objects (dicts) -> list of response dicts (same order)"""
    out = [None] * len(reqs)
    batch = engine.Batch(ds)
    slots = []
    for k, r in enumerate(reqs):
        try:
            p = parse(r)
            batch.add_fit(p)
            slots.append(k)
        except RequestError as e:
            out[k] = _err(e.code, e.message, e.path)
    if batch.fits:
        vals = engine.run(batch)
        for fit, k in zip(batch.fits, slots):
            out[k] = FitStats(batch, fit, vals).compute()
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
