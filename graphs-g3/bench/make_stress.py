#!/usr/bin/env python3
# SPDX-License-Identifier: LGPL-3.0-or-later
"""stress corpora for bench/identity_check.sh: usage make_stress.py CASES_DIR OUT_DIR
stress:  every case + a dense (397 pts, incl. one negative x) and a descending variant
stress2: every damage case against 3 target fits, plus rescaled/reversed x and y = dps+volley+damage
stress3: every application_profile case x 7 targets x ammo tiers x target speeds x settings, 400 points"""
import glob
import json
import os
import sys

C, O = sys.argv[1], sys.argv[2]
cases = {os.path.basename(f)[:-5]: json.load(open(f)) for f in sorted(glob.glob(os.path.join(C, "*.json")))}
with open(os.path.join(O, "stress.jsonl"), "w") as out:
    for name, r in cases.items():
        out.write(json.dumps(r) + "\n")
        xs = r["x"]["values"]
        hi = max(xs) or 1.0
        for k, n in ((1, 397), (2, 61)):
            vals = [hi * 1.1 * i / (n - 1) - (hi * 0.02 if i == 0 else 0) for i in range(n)]
            if k == 2:
                vals = sorted(set([round(v, 1) for v in vals] + xs))[::-1]
            out.write(json.dumps(dict(r, x=dict(r["x"], values=vals))) + "\n")
tgts = [r["target"] for r in cases.values() if isinstance(r.get("target"), dict) and r["target"].get("fit")]
with open(os.path.join(O, "stress2.jsonl"), "w") as out:
    for name, r in sorted(cases.items()):
        if r["graph"] != "damage":
            continue
        for t in tgts[:3]:
            for xs in (r["x"]["values"], [v * 0.37 for v in r["x"]["values"]][::-1]):
                out.write(json.dumps(dict(r, target=t, x=dict(r["x"], values=xs))) + "\n")
            out.write(json.dumps(dict(r, target=t, y=["dps", "volley", "damage"])) + "\n")
uniq = []
for r in cases.values():
    if r.get("target") and r["target"] not in uniq:
        uniq.append(r["target"])
with open(os.path.join(O, "stress3.jsonl"), "w") as out:
    for r in [r for r in cases.values() if r["graph"] == "application_profile"]:
        for t in uniq[:6] + [None]:
            for q in ("all", "t1", "navy"):
                for sp in (0, 50, 100, 250):
                    for st in ({}, {"apply_projected": False}, {"ignore_resists": True}):
                        r2 = json.loads(json.dumps(r))
                        if t is not None:
                            r2["target"] = t
                        r2.setdefault("params", {})
                        r2["params"]["ammo_quality"] = q
                        r2["params"]["tgt_speed_pct"] = sp
                        r2["settings"] = dict(r2.get("settings") or {}, **st)
                        r2["x"]["values"] = [i * 250.0 for i in range(400)]
                        out.write(json.dumps(r2) + "\n")
