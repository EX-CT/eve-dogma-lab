#!/usr/bin/env python3
"""Empty x.values -> empty series, never an error (DESIGN.md "Empty x.values").

usage: tests/test_empty_x.py DATASET CASES_DIR   (CASES_DIR = eve-dogma-bench graphs/cases)
   or: python3 -m unittest discover -s tests   with EVE_DOGMA_DATASET and EVE_DOGMA_GRAPH_CASES set
       (skipped when they are not set)
Every corpus case is re-run with x.values = [] and every y series valid for its graph/axis (so the time_s
parameter, target-fit and application-profile paths are all covered). It runs in-process, through
graph-batch, through `graph` (single, exit 0) and through serve-stdio."""
import glob
import json
import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path[:0] = [ROOT, os.path.join(os.path.dirname(ROOT), "variant-g")]
from evedogma_g import dataset  # noqa: E402
from g3.graph import AXES, Engine  # noqa: E402

EXE = os.path.join(ROOT, "bin", "eve-dogma-g3")


def check(req, res):
    assert "error" not in res, (req["graph"], req["x"]["axis"], res)
    assert res["x"] == [] and res["x_axis"] == req["x"]["axis"] and res["graph"] == req["graph"], res
    for y in req["y"]:
        assert res["series"].get(y) == [], (y, res)
    for k, v in res["series"].items():
        assert v == [], (k, res)


def run(DPATH, CASES):
    reqs = []
    for f in sorted(glob.glob(os.path.join(CASES, "*.json"))):
        if os.path.basename(f).startswith("err_"):  # contract 0.2 error cases: invalid requests stay errors
            continue
        r = json.load(open(f))
        r["x"]["values"] = []
        reqs.append(r)
        r2 = json.loads(json.dumps(r))
        r2["y"] = list(AXES[r["graph"]][r["x"]["axis"]])
        reqs.append(r2)

    ds = dataset.load(DPATH)
    for cache in (True, False):
        eng = Engine(ds, cache=cache, dataset_path=DPATH)
        for r in reqs:
            check(r, eng.graph(r))

    out = subprocess.run([EXE, "graph-batch", "--dataset", DPATH], input="".join(json.dumps(r) + "\n" for r in reqs),
                         capture_output=True, text=True, check=True).stdout.splitlines()
    assert len(out) == len(reqs)
    for r, line in zip(reqs, out):
        check(r, json.loads(line))

    p = subprocess.run([EXE, "graph", "--dataset", DPATH], input=json.dumps(reqs[0]), capture_output=True, text=True)
    assert p.returncode == 0, p
    check(reqs[0], json.loads(p.stdout))

    rpc = "".join(json.dumps({"id": k, "method": "graph", "params": r}) + "\n" for k, r in enumerate(reqs[:6]))
    out = subprocess.run([EXE, "serve-stdio", "--dataset", DPATH], input=rpc, capture_output=True, text=True,
                         check=True).stdout.splitlines()
    for k, line in enumerate(out):
        m = json.loads(line)
        assert m["id"] == k, m
        check(reqs[k], m["result"])

    return len(reqs)


class EmptyX(unittest.TestCase):
    def test_empty_x_all_graphs(self):
        d, c = os.environ.get("EVE_DOGMA_DATASET"), os.environ.get("EVE_DOGMA_GRAPH_CASES")
        if not d or not c or not os.path.isdir(c):
            self.skipTest("EVE_DOGMA_DATASET / EVE_DOGMA_GRAPH_CASES not set")
        self.assertGreater(run(d, c), 0)


if __name__ == "__main__":
    n = run(sys.argv[1], sys.argv[2])
    print(f"empty x.values: {n} requests x (in-process cache/no-cache, graph-batch) + graph + rpc: OK")
