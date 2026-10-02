# Scorecard: B

- command: `./target/release/eve-dogma-vb --dataset /workspace/exct-eve/data/dataset-3569502.json.gz calc`, batch: `./target/release/eve-dogma-vb --dataset /workspace/exct-eve/data/dataset-3569502.json.gz batch`
- cases fully correct: **295/295**
- values correct: **18978/18978** (100.00 %)
- engine errors: 0

| group | ok | total | % |
|---|---|---|---|
| application | 3501 | 3501 | 100.0 |
| capacitor | 1035 | 1035 | 100.0 |
| defense | 5308 | 5308 | 100.0 |
| fitting | 2655 | 2655 | 100.0 |
| navigation | 1759 | 1759 | 100.0 |
| offense | 1180 | 1180 | 100.0 |
| tank | 2065 | 2065 | 100.0 |
| targeting | 1475 | 1475 | 100.0 |

| perf | value |
|---|---|
| one process per case, median ms (cold start + calc) | 8.7 |
| batch throughput (corpus x1) fits/s | 7530 |
| latency one fit (exct_rifter) ms/calc | 0.097 |
| startup + one calc ms | 10.7 |
| deterministic | True |
