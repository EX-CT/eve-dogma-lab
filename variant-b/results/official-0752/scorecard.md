# Scorecard: B

- command: `./target/release/eve-dogma-vb --dataset /workspace/exct-eve/data/dataset-3569502.json.gz calc`, batch: `./target/release/eve-dogma-vb --dataset /workspace/exct-eve/data/dataset-3569502.json.gz batch`
- cases fully correct: **326/326**
- values correct: **21051/21051** (100.00 %)
- engine errors: 0

| group | ok | total | % |
|---|---|---|---|
| application | 3943 | 3943 | 100.0 |
| capacitor | 1147 | 1147 | 100.0 |
| defense | 5866 | 5866 | 100.0 |
| fitting | 2934 | 2934 | 100.0 |
| navigation | 1945 | 1945 | 100.0 |
| offense | 1304 | 1304 | 100.0 |
| tank | 2282 | 2282 | 100.0 |
| targeting | 1630 | 1630 | 100.0 |

| perf | value |
|---|---|
| one process per case, median ms (cold start + calc) | 9.9 |
| batch throughput (corpus x5) fits/s | 8084 |
| latency one fit (exct_rifter) ms/calc | 0.054 |
| startup + one calc ms | 17.9 |
| deterministic | True |
