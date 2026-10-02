# Scorecard: variant-g

- command: `/workspace/exct-eve/lab-g/variant-g/bin/eve-dogma-g --dataset /workspace/exct-eve/data/dataset-3569502.json.gz calc`, batch: `/workspace/exct-eve/lab-g/variant-g/bin/eve-dogma-g --dataset /workspace/exct-eve/data/dataset-3569502.json.gz batch`
- cases fully correct: **249/249**
- values correct: **11823/11823** (100.00 %)
- engine errors: 0

| group | ok | total | % |
|---|---|---|---|
| capacitor | 871 | 871 | 100.0 |
| defense | 4480 | 4480 | 100.0 |
| fitting | 2241 | 2241 | 100.0 |
| navigation | 1243 | 1243 | 100.0 |
| offense | 996 | 996 | 100.0 |
| tank | 996 | 996 | 100.0 |
| targeting | 996 | 996 | 100.0 |

| perf | value |
|---|---|
| one process per case, median ms (cold start + calc) | 222.0 |
| batch throughput (corpus x5) fits/s | 170 |
| latency one fit (exct_rifter) ms/calc | 3.703 |
| startup + one calc ms | 240.9 |
| deterministic | True |
