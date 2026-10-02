# Variant G results (2026-10-03 04:25 CST)

## Bench (EX-CT/eve-dogma-bench @ b687270, `bench.py --only G --quick`, 249 cases)
- cases fully correct vs Pyfa: **249/249**; values: **13812/13812** (all 8 groups 100 %); engine errors 0
- one process per case (cold start + calc), median: 228 ms
- batch throughput (corpus x1): 181 fits/s
- latency one fit (exct_rifter, warm): 2.4 ms/calc
- startup + one calc: 200 ms
- deterministic: True
- Run in a scratch copy of the bench (/tmp/g-bench) against the pushed branch, so nothing is written into the bench repo.

## Versus the reference engine (eve-dogma-rs, variant A @ 0e5a1ce)
- `tests/compare_ref.py`: 249/249 cases identical, 0/71507 leaf mismatches (rel tol 1e-6, `meta.engine` excluded).
- Reference A per the coordinator: 249/249, 13812/13812, ~1.3 ms/fit. A's earlier bench README figures: 574 fits/s, 151 ms cold.

## Where the time goes (G)
- cold start: ~65–130 ms is `import numpy` (box load ~5), ~33 ms column-cache unpickle, ~13 ms first calc.
- batch: capacitor simulation (sequential recurrence) > per-fit stats in Python > vectorised dogma evaluation.
- `tests/run_tests.py`: Pyfa parity 249/249, batch == single 249/249, determinism, error cases: ALL OK.
