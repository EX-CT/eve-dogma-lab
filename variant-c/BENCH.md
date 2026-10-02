# Variant C bench results

Harness: `EX-CT/eve-dogma-bench` (249 cases, 13 812 Pyfa-expected values), run 2026-10-03 ~04:27 CST on the
shared 8-core box (load average 9–12 from other workers' benches, so absolute timings are noisy).
Raw scorecard: [bench-results/scorecard.md](bench-results/scorecard.md) / `.json`.

## Correctness

| | result |
|---|---|
| cases fully correct | **249 / 249** |
| values correct | **13 812 / 13 812 (100 %)** |
| engine errors | 0 |
| deterministic | yes |
| full-output diff vs eve-dogma-rs (all 249 bench requests, every field) | 0 differences |

## Performance

Scorecard run at `8459d9d` (parallel loader). Load average was about 10.

| metric | variant C | variant A (eve-dogma-rs, same harness) |
|---|---|---|
| latency, one fit (exct_rifter, warm, ms/calc) | 0.368 (0.19–0.29 in earlier, quieter runs) | 1.42 |
| batch throughput, corpus ×5, fits/s (`-j NumCPU`) | 1 896 (2 460–2 550 measured by hand) | 587 |
| batch throughput, `-j 1`, fits/s | 650–1 140 (noisy) | — |
| startup + one calc, ms | 150 (dataset load alone ≈125) | 182 |
| one process per case, median ms | 192 | — |

Before the parallel loader (`2726c7e`): startup + one calc 283 ms, one process per case 368 ms,
batch 1 363 fits/s.

Go micro benchmarks (`go test -bench`, quiet box): Rifter 0.59 ms, Vexor 1.9 ms, Tengu 0.43 ms,
Nidhoggur 0.63 ms, Hyperion 0.52 ms; all 249 cases average about 0.45 ms/fit when the box is quiet
(0.74 ms under load). Dataset load: about 125 ms wall clock, using all cores.

Batch parallelism: `batch` decodes each JSONL line and fans the work out to N goroutines that share the
immutable `Dataset`. Output order is preserved. `-j 1` gives the single-threaded figure.
