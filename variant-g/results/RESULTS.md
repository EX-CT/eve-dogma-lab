# Variant G results (2026-10-03 06:34 CST)

## Bench 1.8.0 (EX-CT/eve-dogma-bench @ 3da9671 = 1.8.0 frozen, official full run `bench.py --only G` in the shared checkout, 326 cases)
- cases fully correct vs Pyfa: **326/326**; values: **21051/21051** (all 8 groups 100 %); engine errors 0
- one process per case (cold start + calc), median: 121.1 ms
- batch throughput (corpus x5, 1630 requests): 333 fits/s
- latency one fit (exct_rifter, warm, n=500): 2.14 ms/calc
- startup + one calc: 121.8 ms
- deterministic: True
- EFT export (informational column, `rpc_cmd` in bench.yaml): 326/326 identical to Pyfa
- Run against the pushed branch (engine commit c657d1a; an earlier scratch-copy full run gave 130 ms / 340 fits/s / 2.21 ms). Box load ≈ 7 during the run; perf numbers are noisy.
- Raw scorecard: `results/bench/scorecard.{md,json}`, `failures.json` (empty).

## Older corpora (tests/run_tests.py, same tree)
| bench | cases | values |
|---|---|---|
| 1.8.0 | 326/326 | 21051/21051 |
| 1.7.0 | 306/306 | 19621/19621 |
| 1.6.0 | 297/297 | 19103/19103 |
| 1.5.0 | 295/295 | 18978/18978 |
| 1.4.0 | 289/289 | 18591/18591 |
| 1.3.0 | 249/249 | 13812/13812 |

## Versus the reference engine (eve-dogma-rs, variant A @ c629fb8)
- `tests/compare_ref.py`: 326/326 cases identical, 0/99571 leaf mismatches (rel tol 1e-6, `meta.engine` excluded);
  `results/compare_ref.json`.
- A's bench README figures (1.8.0, load ≈10): 0.51 ms/fit, 1 429 fits/s, 147 ms cold.

## Where the time goes (G)
- cold start: ~80–90 ms is Python + `import numpy`, ~10 ms column-cache unpickle, ~7 ms first calc.
- batch: capacitor simulation (sequential recurrence) ≈ 25 %, vectorised dogma evaluation ≈ 27 %,
  per-fit Python stats ≈ 20 %, registration + item setup ≈ 15 %.
- `tests/run_tests.py`: Pyfa parity, batch == single, determinism, error cases, EFT import/export: ALL OK.
