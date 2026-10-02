# Scorecard: G-local

- command: `./bin/eve-dogma-g --dataset /workspace/exct-eve/data/dataset-3569502.json.gz calc`, batch: `./bin/eve-dogma-g --dataset /workspace/exct-eve/data/dataset-3569502.json.gz batch`
- cases fully correct: **289/289**
- values correct: **18591/18591** (100.00 %)
- engine errors: 0

| group | ok | total | % |
|---|---|---|---|
| application | 3428 | 3428 | 100.0 |
| capacitor | 1015 | 1015 | 100.0 |
| defense | 5200 | 5200 | 100.0 |
| fitting | 2601 | 2601 | 100.0 |
| navigation | 1723 | 1723 | 100.0 |
| offense | 1156 | 1156 | 100.0 |
| tank | 2023 | 2023 | 100.0 |
| targeting | 1445 | 1445 | 100.0 |

| perf | value |
|---|---|
| one process per case, median ms (cold start + calc) | 184.5 |
| batch throughput (corpus x1) fits/s | 179 |
| latency one fit (exct_rifter) ms/calc | 2.533 |
| startup + one calc ms | 111.4 |
| deterministic | True |
