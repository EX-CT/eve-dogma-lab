# G3: vectorised grid engine for the Pyfa graphs (round 2)

G3 implements the round-2 graph contract (`eve-dogma-bench@graphs-round2`, `graphs/CONTRACT-GRAPHS.md`, draft 0.1)
on top of variant G's dogma engine (`../variant-g`, untouched). It covers all 9 Pyfa graphs: damage,
application_profile, mobility, lock_time, warp_time, shield_regen, capacitor, ewar and remote_reps.

**Status: 111/111 cases and 1843/1843 sample values correct** (bench `graphs-round2` @ b010e97). The details are
in `bench/scorecard.md`.

## Usage

```sh
bin/eve-dogma-g3 --dataset dataset.json.gz graph request.json       # one GraphRequest -> GraphResult (stdout)
bin/eve-dogma-g3 --dataset dataset.json.gz graph-batch < req.jsonl  # JSONL in -> JSONL out, in order
bin/eve-dogma-g3 --dataset dataset.json.gz serve-stdio              # JSON-RPC; methods graph, calc, meta, ...
bin/eve-dogma-g3 --no-cache ... graph-batch                         # disable the per-fit cache (same output)
```

Every other command (`calc`, `batch`, `meta`, `type`, ...) is forwarded to variant G, so the 1.8.0 stats contract
still works through this binary. `EVE_DOGMA_DATASET` and `EVE_DOGMA_G3_NO_CACHE=1` are honoured. The tool needs
Python ≥ 3.10 and NumPy. The bench manifest is `bench.yaml`.

## Scoring

```sh
cd eve-dogma-bench   # branch graphs-round2
python3 graphs/run_graphs.py --name G3 --cwd <lab>/graphs-g3 \
  --batch-cmd "<lab>/graphs-g3/bin/eve-dogma-g3 graph-batch --dataset <dataset.json.gz>"
python3 <lab>/graphs-g3/bench/perf.py <dataset.json.gz> graphs/cases   # perf figures, see bench/perf-latest.txt
```

## Performance

Measured 2026-10-03 on the shared 8-core box (Python 3.13, NumPy 2.5), taking the CPU-time minimum of several runs.

| metric | value |
|---|---|
| corpus, 111 requests / 1274 points, cold fit cache (in-process) | 318 ms → ~4.0 k points/s |
| corpus, warm fit cache | 45 ms → ~28 k points/s |
| `run_graphs.py` wall time, one `graph-batch` process incl. start-up | 0.55 s |
| dense damage vs distance, 500 points (Kronos): cold fit / warm fit | 3.3 ms / 0.42 ms |
| damage vs distance, 100 k points, warm fit | 52 ms → ~1.9 M points/s |
| cold start + one damage request (process wall) | ~180 ms |
| `--no-cache` output identical to cached output | 111/111 |

Small requests are dominated by per-request overhead such as validation, target set-up and time-axis schedules.
Large distance, speed and signature grids run at NumPy speed.

Architecture, the cache and the Pyfa mapping are described in `DESIGN.md`. This is a behavioural reimplementation:
no Pyfa code is copied.
