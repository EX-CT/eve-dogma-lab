"""eve-dogma-g3 CLI: graph | graph-batch | serve-stdio (graph + every variant-g method); other commands are
forwarded to the variant-g CLI."""
import gc
import json
import os
import sys

USAGE = """eve-dogma-g3 <command> [--dataset PATH] [--no-cache] [args]

  graph [FILE]         GraphRequest JSON (file or stdin) -> GraphResult JSON (exit 2 on error)
  graph-batch          JSONL GraphRequests on stdin -> JSONL GraphResults (same order)
  serve-stdio          JSONL RPC; method "graph" plus the variant-g methods (calc, eft_parse, ...)
  calc | batch | eft | meta | type | search | bench   -> variant-g

--no-cache disables the per-fit memo (output is identical, only slower)."""


def _dumps(o):
    return json.dumps(o, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False)


def main(argv=None):
    args = list(sys.argv[1:] if argv is None else argv)
    no_cache = "--no-cache" in args or bool(os.environ.get("EVE_DOGMA_G3_NO_CACHE"))
    args = [a for a in args if a != "--no-cache"]
    dpath = None
    if "--dataset" in args:
        p = args.index("--dataset")
        dpath = args[p + 1] if p + 1 < len(args) else None
    rest = [a for k, a in enumerate(args) if not ("--dataset" in args and k in (args.index("--dataset"),
                                                                              args.index("--dataset") + 1))]
    if not rest or rest[0] in ("-h", "--help", "help"):
        print(USAGE)
        return 0
    cmd = rest[0]
    if cmd not in ("graph", "graph-batch", "serve-stdio"):
        from evedogma_g import cli as gcli
        return gcli.main(args)
    from evedogma_g import cli as gcli
    from .graph import Engine
    ds = gcli._load(dpath)
    eng = Engine(ds, cache=not no_cache)
    gc.disable()
    if cmd == "graph":
        src = open(rest[1]).read() if len(rest) > 1 and rest[1] != "-" else sys.stdin.read()
        try:
            req = json.loads(src)
        except json.JSONDecodeError as e:
            out = {"error": {"code": "BAD_JSON", "message": str(e), "path": ""}}
        else:
            out = eng.graph(req)
        sys.stdout.write(_dumps(out) + "\n")
        sys.stdout.flush()
        return 2 if "error" in out else 0
    if cmd == "graph-batch":
        w = sys.stdout.write
        for line in sys.stdin:
            if not line.strip():
                continue
            try:
                req = json.loads(line)
            except json.JSONDecodeError as e:
                out = {"error": {"code": "BAD_JSON", "message": str(e), "path": ""}}
            else:
                out = eng.graph(req)
            w(_dumps(out) + "\n")
        sys.stdout.flush()
        return 0
    # serve-stdio
    from evedogma_g.calc import calc
    for line in sys.stdin:
        if not line.strip():
            continue
        try:
            m = json.loads(line)
        except json.JSONDecodeError as e:
            sys.stdout.write(_dumps({"id": None, "error": {"code": "BAD_JSON", "message": str(e)}}) + "\n")
            sys.stdout.flush()
            continue
        mid, meth, params = m.get("id"), m.get("method") or "calc", m.get("params")
        if meth == "graph":
            res = eng.graph(params)
        elif meth == "calc":
            res = calc(ds, params)
        else:
            res = _g_method(gcli, ds, meth, params)
        sys.stdout.write(_dumps({"id": mid, "result": res}) + "\n")
        sys.stdout.flush()
    return 0


def _g_method(gcli, ds, meth, params):
    if meth == "meta":
        r = gcli._meta(ds)
        r["graphs"] = sorted(__import__("g3.graph", fromlist=["AXES"]).AXES)
        return r
    if meth == "type":
        return gcli._type_info(ds, (params or {}).get("id"))
    if meth == "search":
        pp = params or {}
        lim = pp.get("limit")
        kinds = pp.get("kinds")
        return gcli._search(ds, pp.get("query") if isinstance(pp.get("query"), str) else "",
                            lim if isinstance(lim, int) and not isinstance(lim, bool) and lim >= 0 else 20,
                            [x for x in kinds if isinstance(x, str)] if isinstance(kinds, list) else None)
    if meth in ("eft_parse", "eft_export"):
        from evedogma_g import eft
        if meth == "eft_parse":
            try:
                t_ = params.get("text") if isinstance(params, dict) else None
                return eft.parse(ds, t_ if isinstance(t_, str) else "")
            except eft.EftError as e:
                return {"error": {"code": "EFT_PARSE", "message": str(e)}}
        p_ = params if isinstance(params, dict) else {}
        try:
            return {"text": eft.export(ds, p_.get("fit"), p_.get("name") if isinstance(p_.get("name"), str)
                                       else "EXCT fit")}
        except eft.RequestError as e:
            return {"error": {"code": "BAD_REQUEST", "message": e.message}}
    return {"error": {"code": "UNKNOWN_METHOD", "message": str(meth)}}
