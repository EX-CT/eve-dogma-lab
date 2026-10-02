# Scorecard: e-dev

- command: `/workspace/exct-eve/lab-e/variant-e/target/release/eve-dogma-e calc --dataset /workspace/exct-eve/data/dataset-3569502.json.gz`, batch: `/workspace/exct-eve/lab-e/variant-e/target/release/eve-dogma-e batch --dataset /workspace/exct-eve/data/dataset-3569502.json.gz`
- cases fully correct: **221/249**
- values correct: **13759/13812** (99.62 %)
- engine errors: 0

| group | ok | total | % |
|---|---|---|---|
| application | 1989 | 1989 | 100.0 |
| capacitor | 848 | 871 | 97.4 |
| defense | 4480 | 4480 | 100.0 |
| fitting | 2241 | 2241 | 100.0 |
| navigation | 1241 | 1243 | 99.8 |
| offense | 978 | 996 | 98.2 |
| tank | 989 | 996 | 99.3 |
| targeting | 993 | 996 | 99.7 |

| perf | value |
|---|---|
| one process per case, median ms (cold start + calc) | 187.3 |
| batch throughput (corpus x1) fits/s | 1005 |
| latency one fit (exct_rifter) ms/calc | 0.763 |
| startup + one calc ms | 185.7 |
| deterministic | True |

Worst metrics:

- cap_stable_percent: 116/125
- cap_stable: 235/249
- drone_dps: 240/249
- drone_volley: 240/249
- tank.armor: 245/249
- max_velocity: 246/248
- tank.shield: 247/249
- max_target_range: 248/249
- scan_resolution: 248/249
- scan_strength: 248/249
- tank.hull: 248/249
