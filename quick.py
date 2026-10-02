#!/usr/bin/env python3
"""fast correctness check: batch mode over the bench corpus, prints failures. usage: quick.py [binary] [glob]"""
import json, subprocess, sys, pathlib
B = pathlib.Path("/workspace/exct-eve/eve-dogma-bench")
sys.path.insert(0, str(B / "tools"))
from metrics import METRICS, extract, close
binary = sys.argv[1] if len(sys.argv) > 1 else "/workspace/exct-eve/lab-h/variant-h/target/release/eve-dogma-h"
pat = sys.argv[2] if len(sys.argv) > 2 else "*.json"
cases = sorted((B / "cases").glob(pat))
cases = [c for c in cases if (B / "expected" / c.name).exists()]
inp = "".join(json.dumps(json.loads(c.read_text())) + "\n" for c in cases)
r = subprocess.run([binary, "--dataset", "/workspace/exct-eve/data/dataset-3569502.json.gz", "batch"], input=inp, capture_output=True, text=True)
outs = r.stdout.splitlines()
ok_cases = vals = ok_vals = 0
for c, o in zip(cases, outs):
    exp = json.loads((B / "expected" / c.name).read_text())
    resp = json.loads(o)
    bad = []
    for k, want in exp["values"].items():
        if k in exp.get("excluded", {}):
            continue
        got = extract(resp, METRICS[k][0])
        vals += 1
        if close(got, want):
            ok_vals += 1
        else:
            bad.append(f"{k}: got {got} want {want}")
    if bad:
        print(c.stem, "|", "; ".join(bad)[:600])
    else:
        ok_cases += 1
print(f"cases {ok_cases}/{len(cases)} values {ok_vals}/{vals}")
