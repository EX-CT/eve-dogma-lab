# Variant H: results

Corpus: eve-dogma-bench @ 6533a02, plus the per-weapon metrics in the bench working tree (249 cases,
13 812 values, Pyfa expected). Dataset: dataset-3569502.json.gz. All runs were on the shared box, which is noisy.

## Correctness

| | cases | values |
|---|---|---|
| vs Pyfa (bench `run.py`) | **249/249** | **13812/13812 (100.00 %)** |
| vs Variant A @ 0f79b18 (every output leaf, abs 1e-6 / rel 1e-9) | **249/249 identical** | |

Every bench group (application, capacitor, defense, fitting, navigation, offense, tank, targeting) is at 100 %.
The full scorecard is in [results/scorecard.md](results/scorecard.md).

## Speed (bench `run.py` perf section, same box, same time)

| metric | H | A |
|---|---|---|
| batch throughput (corpus x5), fits/s | **1308** | 460 |
| latency, one fit (exct_rifter), ms/calc | **0.706** | 1.430 |
| startup + one calc, ms | **20.8** | 150.1 |
| one process per case, median ms | **19.8** | 216.5 |
| deterministic | yes | yes |

The H cold start assumes the derived cache has been warmed. `bench.yaml`'s build step runs `meta` once to
write it. Without the cache, cold start is about 120 ms.

A local re-check (`batch` over 996 fits, best of 3) gave H 1203 fits/s and A 693 fits/s.
