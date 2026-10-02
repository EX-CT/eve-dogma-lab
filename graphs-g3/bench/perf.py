#!/usr/bin/env python3
"""G3 perf harness. usage: bench/perf.py DATASET CASES_DIR  (CASES_DIR = eve-dogma-bench graphs/cases)

Measures (CPU time, min of N runs; the shared box is noisy):
  a) points/s over the whole graph corpus in one process (cold fit cache, then warm cache)
  b) one dense interactive request: damage vs distance, 500 points (cold fit / warm fit)
  c) cold start + one request: wall time of a fresh `eve-dogma-g3 graph` process
  d) large-grid kernel throughput: damage vs distance, 100k points, warm fit
  e) --no-cache output identical to cached output over the corpus
"""
import glob
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path[:0] = [ROOT, os.path.join(os.path.dirname(ROOT), "variant-g")]
from evedogma_g import dataset  # noqa: E402
from g3.graph import Engine  # noqa: E402

DPATH, CASES = sys.argv[1], sys.argv[2]
ds = dataset.load(DPATH)
reqs = [json.load(open(f)) for f in sorted(glob.glob(os.path.join(CASES, "*.json")))]
npts = sum(len(r["x"]["values"]) for r in reqs)
cpu = time.process_time


def best(fn, n=5):
    t = []
    for _ in range(n):
        t0 = cpu()
        fn()
        t.append(cpu() - t0)
    return min(t)


def corpus_cold():
    e = Engine(ds, dataset_path=DPATH)
    for r in reqs:
        e.graph(r)


warm_eng = Engine(ds, dataset_path=DPATH)
for r in reqs:
    warm_eng.graph(r)
tc = best(corpus_cold, 3)
tw = best(lambda: [warm_eng.graph(r) for r in reqs])
print(f"a) corpus: {len(reqs)} requests, {npts} points; cold-cache {tc*1000:.0f} ms = {npts/tc:,.0f} points/s; "
      f"warm-cache {tw*1000:.1f} ms = {npts/tw:,.0f} points/s")

base = json.load(open(os.path.join(CASES, "dmg_dist_kronos_ideal.json")))
dense = dict(base, x=dict(base["x"], values=[i * 100.0 for i in range(500)]))
t_cold = best(lambda: Engine(ds, dataset_path=DPATH).graph(dense))
e = Engine(ds, dataset_path=DPATH)
e.graph(dense)
t_warm = best(lambda: e.graph(dense), 20)
print(f"b) dense damage/distance 500 pts (kronos): cold fit {t_cold*1000:.1f} ms, warm fit {t_warm*1000:.2f} ms")

big = dict(base, x=dict(base["x"], values=[i * 1.5 for i in range(100000)]))
e.graph(big)
t_big = best(lambda: e.graph(big), 3)
print(f"d) damage/distance 100k pts warm: {t_big*1000:.0f} ms = {100000/t_big:,.0f} points/s (incl. JSON-ready lists)")

tl = []
for name in sorted(os.path.basename(f) for f in glob.glob(os.path.join(CASES, "*.json"))):
    r = json.load(open(os.path.join(CASES, name)))
    if r["x"]["axis"] != "time_s":
        continue
    hi = max(r["x"]["values"])
    rr = dict(r, x=dict(r["x"], values=[hi * k / 499 for k in range(500)]))
    e.graph(rr)
    tl.append((best(lambda: e.graph(rr), 5), name))
tsum = sum(t for t, _ in tl)
print(f"f) time axis, 500 points, warm fit: {len(tl)} requests, total {tsum*1000:.1f} ms = "
      f"{500*len(tl)/tsum:,.0f} points/s; slowest " + ", ".join(f"{n[:-5]} {t*1000:.1f} ms" for t, n in sorted(tl)[::-1][:3]))

exe = os.path.join(ROOT, "bin", "eve-dogma-g3")
walls = []
for _ in range(3):
    t0 = time.perf_counter()
    subprocess.run([exe, "graph", "--dataset", DPATH, os.path.join(CASES, "dmg_dist_kronos_ideal.json")],
                   check=True, capture_output=True)
    walls.append(time.perf_counter() - t0)
print(f"c) cold start + one damage request (process wall): {min(walls)*1000:.0f} ms")

nc = Engine(ds, cache=False, dataset_path=DPATH)
same = sum(json.dumps(nc.graph(r), sort_keys=True) == json.dumps(warm_eng.graph(r), sort_keys=True) for r in reqs)
print(f"e) --no-cache identical output: {same}/{len(reqs)}")
