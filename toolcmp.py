import json, subprocess, glob
D = "/workspace/exct-eve/data/dataset-3569502.json.gz"
H = "/workspace/exct-eve/lab-h/variant-h/target/release/eve-dogma-h"; A = "/workspace/exct-eve/eve-dogma-rs/target/release/eve-dogma"
reqs = []
for q in ["rifter", "warrior", "gunnery", "装甲", "Pulsar", "large shield extender", "hail", "Strong Blue", "x"]:
    reqs.append({"method": "search", "params": {"query": q}})
for t in ["587", "Warrior II", "Damage Control II", "Svipul Sharpshooter Mode", "nope"]:
    reqs.append({"method": "type", "params": {"id": t}})
for f in sorted(glob.glob("/workspace/exct-eve/eve-dogma-bench/cases/*.json")):
    reqs.append({"method": "eft_export", "params": {"fit": json.load(open(f))}})
inp = "".join(json.dumps({"id": i, **r}) + "\n" for i, r in enumerate(reqs))
run = lambda b: [json.loads(l) for l in subprocess.run([b, "--dataset", D, "serve-stdio"], input=inp, capture_output=True, text=True).stdout.splitlines() if l.startswith("{")]
h, a = run(H), run(A)
from collections import Counter
c = Counter()
for r, x, y in zip(reqs, h, a):
    ok = x["result"] == y["result"]
    c[(r["method"], ok)] += 1
    if not ok and c[(r["method"], False)] <= 3:
        print(r["method"], json.dumps(r["params"])[:80], "\n  H:", json.dumps(x["result"], ensure_ascii=False)[:400], "\n  A:", json.dumps(y["result"], ensure_ascii=False)[:400])
print(sorted(c.items()))
print("----- first diffs")
seen = Counter()
for r, x, y in zip(reqs, h, a):
    if x["result"] == y["result"]: continue
    seen[r["method"]] += 1
    if seen[r["method"]] > 4: continue
    if r["method"] == "eft_export":
        s1, s2 = x["result"]["text"], y["result"]["text"]
        i = next((k for k in range(min(len(s1), len(s2))) if s1[k] != s2[k]), min(len(s1), len(s2)))
        print("EXPORT H:", repr(s1[max(0,i-60):i+120])); print("       A:", repr(s2[max(0,i-60):i+120]))
    elif r["method"] == "type":
        X, Y = x["result"], y["result"]
        if "attributes" in X and "attributes" in Y:
            print("TYPE", r["params"], "H-only attrs", sorted(set(X["attributes"]) - set(Y["attributes"])), "A-only", sorted(set(Y["attributes"]) - set(X["attributes"])),
                  {k: (X.get(k), Y.get(k)) for k in set(X) | set(Y) if k != "attributes" and X.get(k) != Y.get(k)})
        else: print("TYPE", r["params"], X, Y)
    else:
        hn = [z["name"] for z in x["result"]]; an = [z["name"] for z in y["result"]]
        print("SEARCH", r["params"], len(hn), len(an), [n for n in hn if n not in an][:5], [n for n in an if n not in hn][:5], hn == an)
