#!/usr/bin/env python3
"""compare H's sustained tank + cap rates against the local Pyfa oracle output (oracle_local/all.jsonl)"""
import json, subprocess, pathlib
B = pathlib.Path("/workspace/exct-eve/eve-dogma-bench/cases")
orc = {}
for line in open("/workspace/exct-eve/lab-h/oracle_local/all.jsonl"):
    d = json.loads(line)
    if "stats" in d: orc[d["file"]] = d["stats"]
names = sorted(orc)
inp = "".join(json.dumps(json.loads((B / n).read_text())) + "\n" for n in names)
r = subprocess.run(["/workspace/exct-eve/lab-h/variant-h/target/release/eve-dogma-h", "--dataset", "/workspace/exct-eve/data/dataset-3569502.json.gz", "batch"], input=inp, capture_output=True, text=True)
close = lambda a, b: a is not None and abs(a - b) <= max(1e-3, 1e-4 * abs(b))
ok = 0; bad = 0
for n, o in zip(names, r.stdout.splitlines()):
    h = json.loads(o); s = orc[n]
    if "defense" not in h: print(n, "ERR", o[:200]); bad += 1; continue
    st = h["defense"]["tank"]["sustained"]; c = h["capacitor"]
    pairs = [("stank.shield", st["shield_repair"], s["sustainable_tank"]["shieldRepair"]),
             ("stank.armor", st["armor_repair"], s["sustainable_tank"]["armorRepair"]),
             ("stank.hull", st["hull_repair"], s["sustainable_tank"]["hullRepair"]),
             ("cap_used", c["use_gj_s"], s["cap_used"]),
             ("cap_recharge", c["peak_recharge_gj_s"] + c["injected_gj_s"], s["cap_recharge"])]
    m = [f"{k}: H {a} pyfa {b}" for k, a, b in pairs if not close(a, b)]
    if m: bad += 1; print(n, "|", "; ".join(m))
    else: ok += 1
print(f"stank/cap ok {ok}/{ok+bad}")
