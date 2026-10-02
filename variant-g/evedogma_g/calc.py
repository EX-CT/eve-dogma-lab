"""Public API: calc_many(ds, requests) evaluates many FitRequests as one NumPy batch."""
import json

from . import capsim, engine
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
    subs = []  # (tag, owner request index, sub index, request): booster fits and projected source fits
    for k, p in enumerate(parsed):
        if isinstance(p, RequestError):
            continue
        for j, b in enumerate(p["fleet"]["booster_fits"]):
            bb = dict(b)
            bb["fleet"] = {"buffs": b["fleet"]["buffs"], "booster_fits": []}
            subs.append(("b", k, j, bb))
        for j, pr in enumerate(p["projected"]):
            if pr["kind"] == "fit" and pr["fit"] is not None:
                sr = dict(pr["fit"])
                sr["projected"] = []
                subs.append(("p", k, j, sr))
    offers, warn, frozen = {}, {}, {}
    if subs:
        sbatch, svals, sfits = _batch(ds, [x[3] for x in subs])
        pairs = [(ds.a(i), ds.a(v)) for i, v in engine.WARFARE_PAIRS]
        for (tag, k, j, _), f in zip(subs, sfits):
            if tag == "p":
                if isinstance(f, RequestError):
                    frozen.setdefault(k, {})[j] = _debug_err(f)
                    continue
                lst = []
                for i in f.items:
                    m = sbatch.meta[i]
                    if m["kind"] in (engine.MODULE, engine.FIGHTER):
                        copies = 1 if m["state"] >= engine.ACTIVE else 0
                    else:
                        copies = m["active_count"] if m["kind"] == engine.DRONE else 0
                    if copies:
                        lst.append((m["type_id"], copies, svals.item_dict(i), m["kind"], m["quantity"],
                                    m["fighter_abilities"]))
                frozen.setdefault(k, {})[j] = lst
                continue
            if isinstance(f, RequestError):
                warn.setdefault(k, []).append(f"fleet.booster_fits[{j}]: {_debug_err(f)}")
                continue
            lst = offers.setdefault(k, [])
            for i in f.modules:
                if sbatch.meta[i]["state"] < engine.ACTIVE:
                    continue
                for ida, vala in pairs:
                    bid = int(svals.get(i, ida)) if svals.has(i, ida) else 0
                    if bid:
                        lst.append((bid, svals.get(i, vala)))
    batch = engine.Batch(ds)
    fits = []
    for k, p in enumerate(parsed):
        if isinstance(p, RequestError):
            fits.append(p)
            continue
        try:
            f = batch.add_fit(p, frozen.get(k))
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
    # the capacitor-simulation memo is scoped to this batch (identical simulations of fits in the same batch run
    # once); nothing is carried over to later requests
    capsim._MEMO.clear()
    try:
        for f in fits:
            if isinstance(f, RequestError):
                out.append(_err(f.code, f.message, f.path))
            else:
                out.append(FitStats(batch, f, vals).compute())
    finally:
        capsim._MEMO.clear()
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
