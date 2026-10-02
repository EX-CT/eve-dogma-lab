# Scorecard: e-dev

- command: `/workspace/exct-eve/lab-e/variant-e/target/release/eve-dogma-e calc --dataset /workspace/exct-eve/data/dataset-3569502.json.gz`, batch: `/workspace/exct-eve/lab-e/variant-e/target/release/eve-dogma-e batch --dataset /workspace/exct-eve/data/dataset-3569502.json.gz`
- cases fully correct: **104/249**
- values correct: **11770/13812** (85.22 %)
- engine errors: 0

| group | ok | total | % |
|---|---|---|---|
| application | 0 | 1989 | 0.0 |
| capacitor | 848 | 871 | 97.4 |
| defense | 4480 | 4480 | 100.0 |
| fitting | 2241 | 2241 | 100.0 |
| navigation | 1241 | 1243 | 99.8 |
| offense | 978 | 996 | 98.2 |
| tank | 989 | 996 | 99.3 |
| targeting | 993 | 996 | 99.7 |

| perf | value |
|---|---|
| one process per case, median ms (cold start + calc) | 202.4 |
| batch throughput (corpus x1) fits/s | 567 |
| latency one fit (exct_rifter) ms/calc | 2.121 |
| startup + one calc ms | 253.3 |
| deterministic | True |

Worst metrics:

- w11.falloff_m: 0/67
- w11.optimal_m: 0/67
- w11.tracking: 0/67
- w12.falloff_m: 0/65
- w12.optimal_m: 0/65
- w12.tracking: 0/65
- w13.falloff_m: 0/54
- w13.optimal_m: 0/54
- w13.tracking: 0/54
- w14.falloff_m: 0/43
- w14.optimal_m: 0/43
- w14.tracking: 0/43
- w15.falloff_m: 0/39
- w15.optimal_m: 0/39
- w15.tracking: 0/39
