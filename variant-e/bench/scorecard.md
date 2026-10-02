# Scorecard: e-dev

- command: `/workspace/exct-eve/lab-e/variant-e/target/release/eve-dogma-e calc --dataset /workspace/exct-eve/data/dataset-3569502.json.gz`, batch: `/workspace/exct-eve/lab-e/variant-e/target/release/eve-dogma-e batch --dataset /workspace/exct-eve/data/dataset-3569502.json.gz`
- cases fully correct: **249/249**
- values correct: **13812/13812** (100.00 %)
- engine errors: 0

| group | ok | total | % |
|---|---|---|---|
| application | 1989 | 1989 | 100.0 |
| capacitor | 871 | 871 | 100.0 |
| defense | 4480 | 4480 | 100.0 |
| fitting | 2241 | 2241 | 100.0 |
| navigation | 1243 | 1243 | 100.0 |
| offense | 996 | 996 | 100.0 |
| tank | 996 | 996 | 100.0 |
| targeting | 996 | 996 | 100.0 |

| perf | value |
|---|---|
| one process per case, median ms (cold start + calc) | 149.7 |
| batch throughput (corpus x1) fits/s | 989 |
| latency one fit (exct_rifter) ms/calc | -8.453 |
| startup + one calc ms | 166.1 |
| deterministic | True |
