"""eve-dogma-g CLI (contract v1): calc | batch | serve-stdio | meta | type | search"""
import gc
import json
import os
import sys
import time

USAGE = """eve-dogma-g <command> [--dataset PATH] [args]

  calc [FILE]          FitRequest JSON (file or stdin) -> FitStats JSON
  batch [--chunk N]    JSONL FitRequests on stdin -> JSONL FitStats (evaluated N at a time as one NumPy batch)
  serve-stdio          JSONL RPC {"id","method","params"} -> {"id","result"};
                       methods calc, eft_parse {text}, eft_export {fit,name?}, meta, type, search
  eft [FILE] [--calc] [--skills N]   EFT text (file or stdin) -> FitRequest JSON (--calc: FitStats)
  meta                 dataset info
  type ID|NAME         type with base attributes
  search QUERY         search types by name
  bench FILE [-n N]    time N calculations of one request (single and batched)

Dataset: --dataset PATH, or $EVE_DOGMA_DATASET, or ./dataset.json.gz"""


def _load(path):
    from . import dataset
    p = path or os.environ.get("EVE_DOGMA_DATASET") or "dataset.json.gz"
    try:
        ds = dataset.load(p)
        gc.freeze()  # start-up objects (modules, dataset) never become garbage: keep them out of GC passes
        gc.enable()
        return ds
    except (OSError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        sys.exit(3)


def _meta(ds):
    from .stats import ENGINE
    return {"engine": ENGINE, "schema_version": 1, "sde_build": ds.build, "sde_release_date": ds.release_date,
            "dataset_sha256": ds.sha256, "types": len(ds.t_id), "attributes": len(ds.attr_name), "effects": len(ds.eff_info)}


def _type_info(ds, key):
    try:
        tid = int(key)
    except (TypeError, ValueError):
        tid = ds.type_by_name.get(str(key).strip().lower())
    ti = ds.tidx(tid) if tid is not None else -1
    if ti < 0:
        return {"error": {"code": "UNKNOWN_TYPE", "message": str(key)}}
    return {"type_id": tid, "name": ds.t_name[ti], "name_zh": ds.names_zh.get(str(tid)),
            "group": ds.group_name.get(int(ds.t_group[ti])), "group_id": int(ds.t_group[ti]),
            "category_id": int(ds.t_cat[ti]), "published": bool(ds.t_pub[ti]), "mass": float(ds.t_mass[ti]),
            "volume": float(ds.t_vol[ti]), "capacity": float(ds.t_cap[ti]), "slot": ds.t_slot[ti],
            "attributes": {ds.attr_name.get(a, str(a)): v for a, v in ds.type_attrs(ti).items()},
            "effects": [{"id": e, "name": ds.effect_name.get(e), "default": d} for e, d in ds.t_effects[ti]]}


_SEARCH_KINDS = {6: "ship", 7: "module", 8: "charge", 18: "drone", 87: "fighter", 20: "implant", 32: "subsystem",
                 16: "skill"}


def _search(ds, q, limit=20, kinds=None):
    """contract (interim): published ships/modules/charges/drones/fighters/implants/boosters/subsystems/skills,
    case-insensitive on the English or Chinese name, exact > prefix > substring, ties by typeID; limit 20"""
    ql = q.strip().lower()
    hits = []
    for ti, nm in enumerate(ds.t_name):
        if not ds.t_pub[ti]:
            continue
        cat = int(ds.t_cat[ti])
        k = _SEARCH_KINDS.get(cat)
        if k is None:
            continue
        if cat == 20 and "Booster" in (ds.group_name.get(int(ds.t_group[ti])) or ""):
            k = "booster"
        if kinds is not None and k not in kinds:
            continue
        tid = int(ds.t_id[ti])
        en = nm.lower()
        zh = (ds.names_zh.get(str(tid)) or "").lower()
        if en == ql or (zh and zh == ql):
            r = 0
        elif en.startswith(ql) or (zh and zh.startswith(ql)):
            r = 1
        elif ql in en or (zh and ql in zh):
            r = 2
        else:
            continue
        hits.append((r, tid, k, ti))
    hits.sort()
    return [{"type_id": tid, "name": ds.t_name[ti], "name_zh": ds.names_zh.get(str(tid)), "kind": k,
             "match": ("exact", "prefix", "substring")[r], "group": ds.group_name.get(int(ds.t_group[ti])),
             "category_id": int(ds.t_cat[ti]), "meta_level": ds.t_meta.get(ti), "slot": ds.t_slot[ti]}
            for r, tid, k, ti in hits[:limit]]


def main(argv=None):
    args = list(sys.argv[1:] if argv is None else argv)
    dpath = None
    if "--dataset" in args:
        p = args.index("--dataset")
        dpath = args[p + 1] if p + 1 < len(args) else None
        del args[p:p + 2]
    if not args or args[0] in ("-h", "--help", "help"):
        print(USAGE)
        return
    cmd, rest = args[0], args[1:]
    from .calc import calc, calc_json_lines, dumps
    if cmd == "calc":
        src = open(rest[0]).read() if rest and rest[0] != "-" else sys.stdin.read()
        ds = _load(dpath)
        try:
            req = json.loads(src)
        except json.JSONDecodeError as e:
            out = {"error": {"code": "BAD_JSON", "message": str(e), "path": ""}}
        else:
            out = calc(ds, req)
        sys.stdout.write(dumps(out) + "\n")
        if isinstance(out, dict) and "error" in out and len(out) == 1:
            sys.stdout.flush()
            sys.exit(2)  # contract 1.4.1: a calc error still prints the error JSON on stdout, exit code 2
    elif cmd == "batch":
        chunk = 256
        if "--chunk" in rest:
            chunk = int(rest[rest.index("--chunk") + 1])
        ds = _load(dpath)
        lines = [l for l in sys.stdin.read().splitlines() if l.strip()]
        out = calc_json_lines(ds, lines, chunk)
        sys.stdout.write("".join(o + "\n" for o in out))
    elif cmd == "serve-stdio":
        ds = _load(dpath)
        for line in sys.stdin:
            if not line.strip():
                continue
            try:
                m = json.loads(line)
            except json.JSONDecodeError as e:
                sys.stdout.write(dumps({"id": None, "error": {"code": "BAD_JSON", "message": str(e)}}) + "\n")
                sys.stdout.flush()
                continue
            mid, meth, params = m.get("id"), m.get("method") or "calc", m.get("params")
            if meth == "calc":
                res = calc(ds, params)
            elif meth == "eft_parse":
                from . import eft
                try:
                    t_ = params.get("text") if isinstance(params, dict) else None
                    res = eft.parse(ds, t_ if isinstance(t_, str) else "")
                except eft.EftError as e:
                    res = {"error": {"code": "EFT_PARSE", "message": str(e)}}
            elif meth == "eft_export":
                from . import eft
                p_ = params if isinstance(params, dict) else {}
                try:
                    res = {"text": eft.export(ds, p_.get("fit"), p_.get("name") if isinstance(p_.get("name"), str)
                                              else "EXCT fit")}
                except eft.RequestError as e:
                    res = {"error": {"code": "BAD_REQUEST", "message": e.message}}
            elif meth == "meta":
                res = _meta(ds)
            elif meth == "type":
                res = _type_info(ds, (params or {}).get("id"))
            elif meth == "search":
                pp = params or {}
                lim = pp.get("limit")
                kinds = pp.get("kinds")
                res = _search(ds, pp.get("query") if isinstance(pp.get("query"), str) else "",
                              lim if isinstance(lim, int) and not isinstance(lim, bool) and lim >= 0 else 20,
                              [x for x in kinds if isinstance(x, str)] if isinstance(kinds, list) else None)
            else:
                res = {"error": {"code": "UNKNOWN_METHOD", "message": str(meth)}}
            sys.stdout.write(dumps({"id": mid, "result": res}) + "\n")
            sys.stdout.flush()
    elif cmd == "eft":
        from . import eft
        skills = None
        if "--skills" in rest:
            k = rest.index("--skills")
            skills = rest[k + 1] if k + 1 < len(rest) else None
            del rest[k:k + 2]
        do_calc = "--calc" in rest
        rest = [a for a in rest if a != "--calc"]
        src = open(rest[0]).read() if rest and rest[0] != "-" else sys.stdin.read()
        ds = _load(dpath)
        try:
            req = eft.parse(ds, src)
        except eft.EftError as e:
            print(f"error: {e}", file=sys.stderr)
            sys.exit(2)
        if skills is not None:
            try:
                lv = int(skills)
                req["character"]["skills"]["default_level"] = lv if 0 <= lv <= 255 else None
            except ValueError:
                req["character"]["skills"]["default_level"] = None
        out = calc(ds, req) if do_calc else req
        print(json.dumps(out, indent=2, sort_keys=True, ensure_ascii=False))
    elif cmd == "meta":
        print(json.dumps(_meta(_load(dpath)), indent=1))
    elif cmd == "type":
        print(json.dumps(_type_info(_load(dpath), " ".join(rest)), indent=1, ensure_ascii=False))
    elif cmd == "search":
        print(json.dumps(_search(_load(dpath), " ".join(rest)), indent=2, ensure_ascii=False, sort_keys=True))
    elif cmd == "bench":
        n = int(rest[rest.index("-n") + 1]) if "-n" in rest else 200
        ds = _load(dpath)
        req = json.load(open(rest[0]))
        from .calc import calc_many
        calc(ds, req)
        t = time.perf_counter()
        for _ in range(n):
            calc(ds, req)
        single = (time.perf_counter() - t) * 1000 / n
        t = time.perf_counter()
        calc_many(ds, [req] * n)
        batched = (time.perf_counter() - t) * 1000 / n
        print(json.dumps({"n": n, "single_ms_per_calc": single, "batched_ms_per_calc": batched}))
    else:
        print(USAGE, file=sys.stderr)
        sys.exit(2)
