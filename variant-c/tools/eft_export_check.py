#!/usr/bin/env python3
"""Check `eft_export` against Pyfa's exporter (testdata/oracle/eft_export_expected.jsonl, frozen by Variant A's
oracle/pyfa_eft_export.py). Known data divergence: T3 cruisers have maxSubSystems 5 in SDE 3569502 vs 4 in Pyfa's
eve.db, so one extra "[Empty Subsystem slot]" line is tolerated.  usage: eft_export_check.py [BIN] [--show N]"""
import json, os, subprocess, sys
here = os.path.dirname(os.path.abspath(__file__))
bin_ = sys.argv[1] if len(sys.argv) > 1 and not sys.argv[1].startswith("--") else os.path.join(here, "..", "bin", "eve-dogma-go")
show = int(sys.argv[sys.argv.index("--show") + 1]) if "--show" in sys.argv else 0
ds = os.environ.get("EVE_DOGMA_DATASET", "/workspace/exct-eve/data/dataset-3569502.json.gz")
exp = [json.loads(l) for l in open(os.path.join(here, "..", "testdata", "oracle", "eft_export_expected.jsonl"))]
inp = "".join(json.dumps({"id": i, "method": "eft_export", "params": {"fit": e["fit"], "name": e["name"]}}) + "\n" for i, e in enumerate(exp))
out = subprocess.run([bin_, "--dataset", ds, "serve-stdio"], input=inp, capture_output=True, text=True).stdout.splitlines()
ok = bad = 0
for e, line in zip(exp, out):
    ours = json.loads(line).get("result", {}).get("text")
    if ours is not None and (ours == e["text"] or ours.replace("\n[Empty Subsystem slot]", "", 1) == e["text"]):
        ok += 1
        continue
    bad += 1
    if bad <= show:
        print("MISMATCH", e["file"]); print("--- pyfa\n" + e["text"]); print("--- ours\n" + str(ours)); print()
print(f"eft_export parity: {ok}/{ok + bad}")
sys.exit(1 if bad else 0)
