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
IGNORE = [x for x in os.environ.get("IGNORE", "").split(",") if x]  # path prefixes (regex) to skip, e.g. new upstream WIP fields
import re
from concurrent.futures import ThreadPoolExecutor
files = sorted(glob.glob(sys.argv[1] if len(sys.argv) > 1 else "testdata/requests/*.json"))
with ThreadPoolExecutor(8) as ex:
    res = list(ex.map(lambda f: (f, run(RS, f), run(GO, f)), files))
tot = 0; bad = 0
for f, r, g in res:
    out = []; walk(r, g, "", out); tot += 1
    out = [o for o in out if not any(re.match(x, o[0]) for x in IGNORE)]
    if out:
        bad += 1
        print(os.path.basename(f), len(out), out[:6])
print(f"{tot} requests, {bad} with full-output differences (rs vs go)")
