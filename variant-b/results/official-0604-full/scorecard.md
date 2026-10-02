# Scorecard: B

- command: `./target/release/eve-dogma-vb --dataset /workspace/exct-eve/data/dataset-3569502.json.gz calc`, batch: `./target/release/eve-dogma-vb --dataset /workspace/exct-eve/data/dataset-3569502.json.gz batch`
- cases fully correct: **306/306**
- values correct: **19621/19621** (100.00 %)
- engine errors: 0

| group | ok | total | % |
|---|---|---|---|
| application | 3566 | 3566 | 100.0 |
| capacitor | 1074 | 1074 | 100.0 |
| defense | 5506 | 5506 | 100.0 |
| fitting | 2754 | 2754 | 100.0 |
| navigation | 1825 | 1825 | 100.0 |
| offense | 1224 | 1224 | 100.0 |
| tank | 2142 | 2142 | 100.0 |
| targeting | 1530 | 1530 | 100.0 |

| perf | value |
|---|---|
| one process per case, median ms (cold start + calc) | 7.4 |
| batch throughput (corpus x5) fits/s | 4326 |
| latency one fit (exct_rifter) ms/calc | 0.126 |
| startup + one calc ms | 21.1 |
| deterministic | True |
