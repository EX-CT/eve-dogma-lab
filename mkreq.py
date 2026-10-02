#!/usr/bin/env python3
"""convert A's {eft, patch} test specs into full FitRequest JSON (uses A's `eft` importer as a tool)"""
import json, subprocess, sys, pathlib
A = "/workspace/exct-eve/eve-dogma-rs"; BIN = A + "/target/release/eve-dogma"
eft = lambda p: json.loads(subprocess.run([BIN, "--dataset", "/workspace/exct-eve/data/dataset-3569502.json.gz", "eft", A + "/tests/" + p, "--skills", "5"], capture_output=True, text=True).stdout)
out = pathlib.Path(sys.argv[1]); out.mkdir(exist_ok=True)
for f in sys.argv[2:]:
    spec = json.load(open(f))
    if "eft" not in spec:
        (out / pathlib.Path(f).name).write_text(json.dumps(spec)); continue
    req = eft(spec["eft"]); patch = dict(spec.get("patch", {}))
    if spec.get("booster_efts"):
        patch["fleet"] = {"buffs": [], "booster_fits": [eft(e) for e in spec["booster_efts"]]}
    for pe in spec.get("projected_efts", []):
        patch["projected"] = patch.get("projected", []) + [{"kind": "fit", "fit": eft(pe["eft"]), "amount": pe.get("amount", 1), "distance_m": pe.get("distance_m")}]
    req.update(patch)
    (out / pathlib.Path(f).name).write_text(json.dumps(req))
