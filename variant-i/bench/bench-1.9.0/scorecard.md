# Scorecard: I19

- command: `/workspace/exct-eve/lab-i/variant-i/target/release/eve-dogma-i calc --dataset /workspace/exct-eve/data/dataset-3569502.json.gz`, batch: `/workspace/exct-eve/lab-i/variant-i/target/release/eve-dogma-i batch --dataset /workspace/exct-eve/data/dataset-3569502.json.gz`
- cases fully correct: **331/331**
- values correct: **22046/22046** (100.00 %)
- engine errors: 0

| group | ok | total | % |
|---|---|---|---|
| application | 4012 | 4012 | 100.0 |
| capacitor | 1166 | 1166 | 100.0 |
| defense | 5956 | 5956 | 100.0 |
| fitting | 2979 | 2979 | 100.0 |
| navigation | 1975 | 1975 | 100.0 |
| offense | 1986 | 1986 | 100.0 |
| tank | 2317 | 2317 | 100.0 |
| targeting | 1655 | 1655 | 100.0 |

| perf | value |
|---|---|
| one process per case, median ms (cold start + calc) | 18.9 |
| batch throughput (corpus x5) fits/s | 1819 |
| latency one fit (exct_rifter) ms/calc | 0.123 |
| startup + one calc ms | 21.3 |
| deterministic | True |
