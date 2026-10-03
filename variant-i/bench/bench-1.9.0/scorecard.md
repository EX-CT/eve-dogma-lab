# Scorecard: I19

- command: `/workspace/exct-eve/lab-i/variant-i/target/release/eve-dogma-i calc --dataset /workspace/exct-eve/data/dataset-3569502.json.gz`, batch: `/workspace/exct-eve/lab-i/variant-i/target/release/eve-dogma-i batch --dataset /workspace/exct-eve/data/dataset-3569502.json.gz`
- cases fully correct: **329/331**
- values correct: **22037/22046** (99.96 %)
- engine errors: 0

| group | ok | total | % |
|---|---|---|---|
| application | 4012 | 4012 | 100.0 |
| capacitor | 1166 | 1166 | 100.0 |
| defense | 5951 | 5956 | 99.9 |
| fitting | 2979 | 2979 | 100.0 |
| navigation | 1975 | 1975 | 100.0 |
| offense | 1982 | 1986 | 99.8 |
| tank | 2317 | 2317 | 100.0 |
| targeting | 1655 | 1655 | 100.0 |

| perf | value |
|---|---|
| one process per case, median ms (cold start + calc) | 19.0 |
| batch throughput (corpus x5) fits/s | 1976 |
| latency one fit (exct_rifter) ms/calc | 0.140 |
| startup + one calc ms | 21.2 |
| deterministic | True |

Worst metrics:

- ehp.shield: 330/331
- res.shield.em: 330/331
- res.shield.explosive: 330/331
- res.shield.kinetic: 330/331
- res.shield.thermal: 330/331
- weapon_dps: 330/331
- weapon_pure_dps: 330/331
- weapon_pure_volley: 330/331
- weapon_volley: 330/331
