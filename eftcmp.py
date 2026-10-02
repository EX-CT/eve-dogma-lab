#!/usr/bin/env python3
import json, subprocess, glob
D = "/workspace/exct-eve/data/dataset-3569502.json.gz"
H = "/workspace/exct-eve/lab-h/variant-h/target/release/eve-dogma-h"; A = "/workspace/exct-eve/eve-dogma-rs/target/release/eve-dogma"
same = diff = rt_ok = 0
for f in sorted(glob.glob("/workspace/exct-eve/eve-dogma-rs/tests/fits/*.eft")):
    h = subprocess.run([H, "--dataset", D, "eft", f, "--skills", "5"], capture_output=True, text=True)
    a = subprocess.run([A, "--dataset", D, "eft", f, "--skills", "5"], capture_output=True, text=True)
    try:
        hj, aj = json.loads(h.stdout), json.loads(a.stdout)
    except Exception as e:
        print(f, "PARSE FAIL", h.stdout[:200], a.stdout[:100]); diff += 1; continue
    if hj == aj: same += 1
    else:
        diff += 1
        ks = [k for k in set(hj) | set(aj) if hj.get(k) != aj.get(k)]
        print(f.split("/")[-1], ks, [(json.dumps(hj.get(k))[:250], json.dumps(aj.get(k))[:250]) for k in ks][:2])
    # round trip through H export
    rpc = json.dumps({"id": 1, "method": "eft_export", "params": {"fit": hj}}) + "\n"
    t = json.loads(subprocess.run([H, "--dataset", D, "serve-stdio"], input=rpc, capture_output=True, text=True).stdout)["result"]["text"]
    rpc = json.dumps({"id": 2, "method": "eft_parse", "params": {"text": t, "skills": 5}}) + "\n"
    back = json.loads(subprocess.run([H, "--dataset", D, "serve-stdio"], input=rpc, capture_output=True, text=True).stdout)["result"]
    # Pyfa-format export is lossy by design: no T3D mode line, drones/fighters/cargo/implants re-sorted,
    # mutation lines list every rolled attribute, fighter squads written with an explicit size
    def norm(r):
        r = json.loads(json.dumps(r)); r["ship"]["mode_type_id"] = None
        for k in ("modules", "drones", "fighters", "cargo", "implants", "boosters"):
            for x in r.get(k, []):
                if isinstance(x, dict):
                    if x.get("mutation"): x["mutation"]["attributes"] = {}
                    if k == "fighters": x["quantity"] = None
            r[k] = sorted(json.dumps(x, sort_keys=True) for x in r.get(k, []))
        return r
    if norm(back) == norm(hj): rt_ok += 1
    else: print("ROUNDTRIP", f.split("/")[-1], t[:300])
print(f"eft import identical to A: {same}/{same+diff}; H export->import round trip: {rt_ok}/{same+diff}")
