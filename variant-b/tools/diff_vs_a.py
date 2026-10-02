#!/usr/bin/env python3
"""Full-output differential test: variant B vs eve-dogma-rs (A) on every bench case (all fields, not only
the Pyfa-checked metrics). Usage: diff_vs_a.py A_BIN B_BIN DATASET CASES_DIR"""
import json, subprocess, sys, glob, os
a_bin, b_bin, ds, cases = sys.argv[1:5]
files = sorted(glob.glob(os.path.join(cases, "*.json")))
reqs = [json.dumps(json.load(open(f)), separators=(",", ":")) for f in files]
inp = "\n".join(reqs) + "\n"
def run(b):
    out = subprocess.run([b, "--dataset", ds, "batch"], input=inp, capture_output=True, text=True, check=True).stdout
    return [json.loads(l) for l in out.splitlines()]
ra, rb = run(a_bin), run(b_bin)
def walk(x, y, path, diffs):
    if isinstance(x, dict) and isinstance(y, dict):
        for k in sorted(set(x) | set(y)):
            if path == "/meta" and k == "engine":
                continue
            walk(x.get(k), y.get(k), f"{path}/{k}", diffs)
    elif isinstance(x, list) and isinstance(y, list) and len(x) == len(y):
        for i, (p, q) in enumerate(zip(x, y)):
            walk(p, q, f"{path}/{i}", diffs)
    elif isinstance(x, (int, float)) and isinstance(y, (int, float)) and not isinstance(x, bool):
        if abs(x - y) > 1e-9 * max(1.0, abs(x)):
            diffs.append((path, x, y))
    elif x != y:
        diffs.append((path, x, y))
bad = 0
exact = 0
for f, x, y in zip(files, ra, rb):
    d = []
    walk(x, y, "", d)
    if d:
        bad += 1
        print(os.path.basename(f), len(d), d[:5])
    xs = dict(x); ys = dict(y); xs.get("meta", {}).pop("engine", None); ys.get("meta", {}).pop("engine", None)
    exact += json.dumps(xs, sort_keys=True) == json.dumps(ys, sort_keys=True)
print(f"{len(files)} cases, {len(files)-bad} identical within 1e-9, {exact} byte-identical (excluding meta.engine)")
