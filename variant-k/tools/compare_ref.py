#!/usr/bin/env python3
"""Full-output diff of variant K against the reference engine (variant A), over a set of FitRequest JSON files.
   python3 tools/compare_ref.py [--k CMD] [--ref CMD] [glob ...]
Compares every field (numbers with the bench tolerance), prints a per-case summary and the first diffs."""
import argparse, glob, json, os, subprocess, sys

# Paths come from the environment (no machine-specific defaults): EVE_DOGMA_DATASET, EVE_DOGMA_REF_BIN, EVE_DOGMA_BENCH.
DS = os.environ.get("EVE_DOGMA_DATASET", "dataset-3569502.json.gz")
REF = os.environ.get("EVE_DOGMA_REF_BIN", "eve-dogma")
BENCH = os.environ.get("EVE_DOGMA_BENCH", "../eve-dogma-bench")
ap = argparse.ArgumentParser()
ap.add_argument("--k", default=f"./bin/eve-dogma-k --dataset {DS} batch")
ap.add_argument("--ref", default=f"{REF} --dataset {DS} batch")
ap.add_argument("--show", type=int, default=8)
ap.add_argument("--ignore", default="/meta/engine")
ap.add_argument("globs", nargs="*", default=[os.path.join(BENCH, "cases", "*.json")])
a = ap.parse_args()
files = sorted(f for g in a.globs for f in glob.glob(g))
reqs = [json.dumps(json.load(open(f))) for f in files]
inp = "".join(r + "\n" for r in reqs)
run = lambda c: subprocess.run(c, shell=True, input=inp, capture_output=True, text=True).stdout.splitlines()
ko, ro = run(a.k), run(a.ref)
ignore = set(a.ignore.split(","))

def diff(x, y, p, out):
    if p in ignore:
        return
    if isinstance(x, dict) and isinstance(y, dict):
        for k in sorted(set(x) | set(y)):
            if f"{p}/{k}" in ignore: continue
            if k not in x: out.append(f"{p}/{k}: missing in K (ref {json.dumps(y[k])[:80]})")
            elif k not in y: out.append(f"{p}/{k}: extra in K ({json.dumps(x[k])[:80]})")
            else: diff(x[k], y[k], f"{p}/{k}", out)
    elif isinstance(x, list) and isinstance(y, list):
        if len(x) != len(y): out.append(f"{p}: len {len(x)} != ref {len(y)}")
        for i, (u, v) in enumerate(zip(x, y)): diff(u, v, f"{p}/{i}", out)
    elif isinstance(x, bool) or isinstance(y, bool) or not isinstance(x, (int, float)) or not isinstance(y, (int, float)):
        if x != y: out.append(f"{p}: {json.dumps(x)[:80]} != ref {json.dumps(y)[:80]}")
    elif abs(x - y) > max(1e-6, 1e-9 * abs(y)):
        out.append(f"{p}: {x} != ref {y}")

same = 0
for f, k, r in zip(files, ko, ro):
    out = []
    diff(json.loads(k), json.loads(r), "", out)
    name = f.split("/")[-1]
    if not out:
        same += 1
    else:
        print(f"{name}: {len(out)} diffs"); [print("   ", d) for d in out[:a.show]]
print(f"identical (tol 1e-9 rel): {same}/{len(files)}  (K lines {len(ko)}, ref lines {len(ro)})")
