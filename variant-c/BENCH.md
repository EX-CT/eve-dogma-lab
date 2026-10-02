# Variant C bench results

Harness: `EX-CT/eve-dogma-bench` **1.8.0** (0969967): 326 cases, 21 051 Pyfa-expected values, contract 1.4.3.
Official run: `python3 bench.py --only C` at 2026-10-03 06:13 CST on commit `f5ef9d9`. Result: 326/326,
0.075 ms/calc, 8 024 fits/s, cold 28 ms. At 1.7.0 (`24129ee`) the run gave 0.081 ms/calc, 10 013 fits/s and
cold 25 ms.
Raw scorecard: [bench-results/scorecard.md](bench-results/scorecard.md) / `.json`.

## Correctness

| | result |
|---|---|
| cases fully correct | **326 / 326** |
| values correct | **21 051 / 21 051 (100 %)** |
| EFT export (Pyfa-exact, `rpc_cmd` column) | **326 / 326** |
| engine errors | 0 |
| deterministic | yes |
| full-output diff vs eve-dogma-rs (all 326 bench requests, every field) | 0 differences |

## Performance

The shared 8-core box ran at a load average of 8–15 from other workers all session, so wall-clock numbers
swing by 3–4× between runs. Use the best run and the instruction counts to compare.

| metric | best official run (05:27, load ≈8) | latest official run (05:46, load ≈14) | start of session (04:27) |
|---|---|---|---|
| latency, one fit (exct_rifter, warm, ms/calc) | **0.062** | 0.245 | 0.368 |
| batch throughput, corpus ×5, fits/s | **9 406** | 3 687 | 1 896 |
| cold: one process per case, median ms | **49** | 62 | 192 |
| startup + one calc, ms | — | 75 | 150 |

Measurements that don't depend on load:

| | value |
|---|---|
| instructions per calc, Rifter (callgrind, `tools/instr.sh`) | ≈1.4 M (eve-dogma-rs: 2.7–3.5 M) |
| startup + one calc, min of 20 | 27 ms (53 ms before cache v4; 283 ms at `2726c7e`) |
| request decode | 7 µs/fit (35 µs with encoding/json) |
| allocations, whole corpus (`BenchmarkAllCasesJSON`) | 72.5 k (≈98.5 k before typed rows/kobj) |
| capsim, Vexor (20 320 iterations, which must match Pyfa exactly) | ≈795 µs (980 µs before pushPop) |

Peers in the same combined table (05:46): J C++ 0.060 ms / 14 047 fits/s / cold 5 ms; A Rust 0.432 ms /
1 777 fits/s.

## Notes
* `batch` (which the bench measures) never uses the serve-mode response memo. Only `serve-stdio` and
  `serve-http` memoise identical requests (`EVE_DOGMA_MEMO=0` disables it).
* The cold-start figure includes reading the binary dataset cache (`$EVE_DOGMA_CACHE_DIR` or
  `<user cache dir>/eve-dogma-go/<sha256-prefix>.bin`). It is keyed by the sha256 of the dataset file and
  checked with a crc32c trailer. `EVE_DOGMA_CACHE=off` disables it.
* Batch parallelism: `batch` decodes each JSONL line and fans the work out to N goroutines that share the
  immutable `Dataset`. Output order is preserved. `-j 1` gives the single-threaded figure.
