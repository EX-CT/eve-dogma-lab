#!/usr/bin/env python3
"""median wall ms of `BIN --dataset DS calc` on one request (process cold start + calc)."""
import subprocess, sys, time, statistics, os
b = sys.argv[1] if len(sys.argv) > 1 else "./target/release/eve-dogma-vb"
req = open(sys.argv[2] if len(sys.argv) > 2 else "/tmp/rifter_req.json", "rb").read()
ds = os.environ["EVE_DOGMA_DATASET"]
r = []
for _ in range(int(os.environ.get("N", "30"))):
    s = time.perf_counter(); subprocess.run([b, "--dataset", ds, "calc"], input=req, capture_output=True); r.append((time.perf_counter() - s) * 1000)
print(f"median {statistics.median(r):.2f} ms  min {min(r):.2f} ms")
