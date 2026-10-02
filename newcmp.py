#!/usr/bin/env python3
"""compare H vs local Pyfa oracle on arbitrary request dir: newcmp.py ORACLE.jsonl REQDIR [binary]"""
import json, subprocess, sys, pathlib
sys.path.insert(0, "/workspace/exct-eve/eve-dogma-bench/tools")
from metrics import METRICS, extract, close, from_pyfa
orc = {}
for line in open(sys.argv[1]):
    d = json.loads(line)
    if "stats" in d: orc[d["file"]] = d["stats"]
R = pathlib.Path(sys.argv[2])
binary = sys.argv[3] if len(sys.argv) > 3 else "/workspace/exct-eve/lab-h/variant-h/target/release/eve-dogma-h"
names = sorted(orc)
inp = "".join(json.dumps(json.loads((R / n).read_text())) + "\n" for n in names)
r = subprocess.run([binary, "--dataset", "/workspace/exct-eve/data/dataset-3569502.json.gz", "batch"], input=inp, capture_output=True, text=True)
EXTRA = {"stank.armor": "/defense/tank/sustained/armor_repair", "stank.shield": "/defense/tank/sustained/shield_repair",
         "stank.hull": "/defense/tank/sustained/hull_repair", "jam_chance": "/targeting/jam_chance_percent"}
okc = tot = okv = totv = 0
for n, o in zip(names, r.stdout.splitlines()):
    s = orc[n]; h = json.loads(o)
    try:
        exp = from_pyfa(s)
    except Exception as e:
        print(n, "from_pyfa fail", e); continue
    st = s.get("sustainable_tank")
    if st: exp.update({"stank.armor": st["armorRepair"], "stank.shield": st["shieldRepair"], "stank.hull": st["hullRepair"]})
    if s.get("jam_chance") is not None: exp["jam_chance"] = s["jam_chance"]
    bad = []
    for k, want in exp.items():
        ptr = EXTRA.get(k) or (METRICS[k][0] if k in METRICS else None)
        if ptr is None: continue
        got = extract(h, ptr); totv += 1
        if close(got, want): okv += 1
        else: bad.append(f"{k}: H {got} pyfa {want}")
    tot += 1
    if bad: print(n, "|", "; ".join(bad)[:700])
    else: okc += 1
print(f"cases {okc}/{tot} values {okv}/{totv}")
