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
| one process per case, median ms (cold start + calc) | 6.3 |
| batch throughput (corpus x1) fits/s | 5180 |
| latency one fit (exct_rifter) ms/calc | 0.144 |
| startup + one calc ms | 9.1 |
| deterministic | True |
