import json, subprocess, sys, os, glob
RS = "/workspace/exct-eve/eve-dogma-rs/target/release/eve-dogma"
GO = "./bin/eve-dogma-go"
def run(b, f):
    return json.loads(subprocess.run([b, "calc", f], capture_output=True, text=True).stdout)
def walk(a, b, p, out):
    if isinstance(a, dict) and isinstance(b, dict):
        for k in set(a) | set(b):
            if p + "/" + k in ("/meta/engine",): continue
            walk(a.get(k, "<missing>"), b.get(k, "<missing>"), p + "/" + k, out)
    elif isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b): out.append((p, f"len {len(a)} vs {len(b)}")); return
        for i, (x, y) in enumerate(zip(a, b)): walk(x, y, f"{p}/{i}", out)
    elif isinstance(a, (int, float)) and isinstance(b, (int, float)) and not isinstance(a, bool) and not isinstance(b, bool):
        if abs(a - b) > max(1e-6, 1e-9 * abs(a)): out.append((p, f"{a} vs {b}"))
    elif a != b:
        out.append((p, f"{a!r} vs {b!r}"))
from concurrent.futures import ThreadPoolExecutor
files = sorted(glob.glob("testdata/requests/*.json"))
with ThreadPoolExecutor(8) as ex:
    res = list(ex.map(lambda f: (f, run(RS, f), run(GO, f)), files))
tot = 0; bad = 0
for f, r, g in res:
    out = []; walk(r, g, "", out); tot += 1
    if out:
        bad += 1
        print(os.path.basename(f), len(out), out[:6])
print(f"{tot} requests, {bad} with full-output differences (rs vs go)")
