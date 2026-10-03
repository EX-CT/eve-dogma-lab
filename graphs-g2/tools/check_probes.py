#!/usr/bin/env python3
"""Replay extra oracle probes (testdata/app-probes.jsonl: GraphRequest + Pyfa graph-oracle result per line,
mostly application_profile around web ranges, tiers and random fits) through ./bin/graph-batch.
Usage: tools/check_probes.py [--dataset D]   Tolerance as in the bench: max(1e-3, 1e-4*|want|)."""
import json, subprocess, sys, os
here = os.path.dirname(os.path.abspath(__file__))
D = sys.argv[sys.argv.index("--dataset") + 1] if "--dataset" in sys.argv else os.environ.get("EVE_DOGMA_DATASET", "/workspace/exct-eve/data/dataset-3569502.json.gz")
rows = [json.loads(l) for l in open(os.path.join(here, "../testdata/app-probes.jsonl"))]
out = subprocess.run([os.path.join(here, "../bin/graph-batch"), "--dataset", D], input="".join(json.dumps(r["request"]) + "\n" for r in rows),
                     capture_output=True, text=True, check=True).stdout.splitlines()
tot = bad = 0
for r, line in zip(rows, out):
    got, want = json.loads(line), r["expected"]
    for k, w in want["series"].items():
        if k.endswith("_charge_type_id"):
            continue
        for x, a, b in zip(want["x"], w, got["series"][k]):
            tot += 1
            if (a is None) != (b is None) or (a is not None and abs(a - b) > max(1e-3, 1e-4 * abs(a))):
                bad += 1
                print(f"{r['probe']} {k}@{x}: got {b} want {a}")
print(f"{len(rows)} probes, {tot} values, {tot - bad} correct")
sys.exit(1 if bad else 0)
