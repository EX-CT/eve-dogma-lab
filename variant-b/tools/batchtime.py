#!/usr/bin/env python3
"""median wall ms of `BIN batch --threads T` over the bench corpus (/tmp/corpus.jsonl)."""
import subprocess, sys, time, statistics, os, json, glob
b = sys.argv[1] if len(sys.argv) > 1 else "./target/release/eve-dogma-vb"
cdir = os.environ.get("CASES", "/workspace/exct-eve/eve-dogma-bench/cases")
lines = [json.dumps((lambda d: d.get("request", d))(json.load(open(f)))) for f in sorted(glob.glob(cdir + "/*.json"))]
inp = ("\n".join(lines) + "\n").encode()
for th in (sys.argv[2:] or ["1", "8"]):
    r = []
    for _ in range(int(os.environ.get("N", "9"))):
        s = time.perf_counter(); subprocess.run([b, "--dataset", os.environ["EVE_DOGMA_DATASET"], "batch", "--threads", th], input=inp, capture_output=True); r.append(time.perf_counter() - s)
    m = statistics.median(r)
    print(f"threads {th}: {m*1000:.1f} ms  {len(lines)/m:.0f} fits/s (min {min(r)*1000:.1f} ms)")
