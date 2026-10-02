# Variant H: results

Corpus: eve-dogma-bench @ b687270 (249 cases,
13 812 values, Pyfa expected). Dataset: dataset-3569502.json.gz. All runs were on the shared box, which is noisy.

## Correctness

| | cases | values |
|---|---|---|
| vs Pyfa (bench `run.py`) | **249/249** | **13812/13812 (100.00 %)** |
| vs Variant A @ 0f79b18 (every output leaf, abs 1e-6 / rel 1e-9) | **249/249 identical** | |

Every bench group (application, capacitor, defense, fitting, navigation, offense, tank, targeting) is at 100 %.
The full scorecard is in [results/scorecard.md](results/scorecard.md).

## Speed

These numbers are from the official harness, `python3 bench.py --only H --quick`, run at 04:24 CST on the
pushed branch with mimalloc. It writes to [results/scorecard.md](results/scorecard.md).

| metric | H | A (bench results/A) |
|---|---|---|
| latency, one fit (exct_rifter), ms/calc | **0.550** | ~1.3–1.43 |
| batch throughput, fits/s | **1102** (corpus x1) | 460 (corpus x5) |
| startup + one calc, ms | **18.3** | 150.1 |
| one process per case, median ms | **15.4** | 216.5 |
| deterministic | yes | yes |

The H cold start assumes the derived cache has been warmed. `bench.yaml`'s build step runs `meta` once to
write it. Without the cache, cold start is about 120 ms.

A local re-check (`batch` over 996 fits, best of 3, same box) gave H 1564 fits/s and A 647 fits/s.
