#!/usr/bin/env python3
"""Compare every leaf of variant H's FitStats with variant A's over the bench corpus (+ extra cases dir)."""
import json, subprocess, sys, pathlib, collections
D = "/workspace/exct-eve/data/dataset-3569502.json.gz"
H = "/workspace/exct-eve/lab-h/variant-h/target/release/eve-dogma-h"
A = "/workspace/exct-eve/eve-dogma-rs/target/release/eve-dogma"
dirs = sys.argv[1:] or ["/workspace/exct-eve/eve-dogma-bench/cases"]
cases = sorted(p for d in dirs for p in pathlib.Path(d).glob("*.json"))
inp = "".join(json.dumps(json.loads(c.read_text())) + "\n" for c in cases)
run = lambda b: subprocess.run([b, "--dataset", D, "batch"], input=inp, capture_output=True, text=True).stdout.splitlines()
ha, aa = run(H), run(A)
IGN = {"/meta/engine"}
def close(a, b):
    if isinstance(a, bool) or isinstance(b, bool):
        return a == b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return abs(a - b) <= max(1e-3, 1e-4 * abs(b))
    return a == b
def walk(a, b, p, out):
    if p in IGN:
        return
    if isinstance(a, dict) and isinstance(b, dict):
        for k in set(a) | set(b):
            walk(a.get(k), b.get(k), f"{p}/{k}", out)
    elif isinstance(a, list) and isinstance(b, list) and len(a) == len(b):
        for i, (x, y) in enumerate(zip(a, b)):
            walk(x, y, f"{p}/{i}", out)
    else:
        if not close(a, b):
            out.append((p, a, b))
tot_leaves = same_cases = 0
freq = collections.Counter()
for c, h, a in zip(cases, ha, aa):
    out = []
    walk(json.loads(h), json.loads(a), "", out)
    if not out:
        same_cases += 1
    else:
        print(c.stem, [(p, x, y) for p, x, y in out][:6])
        for p, _, _ in out:
            freq["/".join(x if not x.isdigit() else "N" for x in p.split("/"))] += 1
print(f"identical (within tol) to A: {same_cases}/{len(cases)}")
print(freq.most_common(20))
