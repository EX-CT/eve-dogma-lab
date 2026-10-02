# Combined scorecard

Generated 2026-10-03 06:21 CST — corpus: 326 cases, expected values from Pyfa (see README).

Bench version 1.8.0+3da9671 (see CHANGELOG.md). Rows may come from different runs: see measured_at/bench_version in combined.json; perf numbers are only comparable at similar load.

| variant | status | cases ok | values ok | accuracy % | application | capacitor | defense | fitting | navigation | offense | tank | targeting | ms/fit | fits/s (batch) | cold ms | deterministic | eft export | bench | measured |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| A eve-dogma-rs main (Rust, lazy memoised modifier graph) | ok | 326/326 | 21051/21051 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 0.507 | 1429 | 147 | True | 326/326 | 1.8.0+a5bb40e | 2026-10-03 06:10 CST |
| C Go | ok | 326/326 | 21051/21051 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 0.075 | 8024 | 28 | True | 326/326 | 1.8.0+0969967 | 2026-10-03 06:13 CST |
| E Pyfa-faithful Rust port | ok | 306/306 | 19621/19621 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 0.216 | 2993 | 12 | True | 306/306 | 1.7.0+a5bb40e | 2026-10-03 06:04 CST |
| F codegen Rust/WASM | ok | 326/326 | 21051/21051 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 0.064 | 10507 | 4 | True | 326/326 | 1.8.0+0969967 | 2026-10-03 06:14 CST |
| H Rust ECS | ok | 299/326 | 20850/21051 | 99.05 | 97.6 | 99.8 | 98.8 | 100.0 | 99.3 | 99.2 | 99.9 | 99.8 | 0.284 | 2797 | 5 | True | 326/326 | 1.7.0+a5bb40e | 2026-10-03 06:10 CST |
| I Rust salsa incremental | ok | 326/326 | 21051/21051 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 0.181 | 1435 | 19 | True | 326/326 | 1.8.0+3da9671 | 2026-10-03 06:19 CST |
| J C++20 | ok | 326/326 | 21051/21051 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 0.043 | 8650 | 7 | True | 326/326 | 1.8.0+3da9671 | 2026-10-03 06:18 CST |
| K Kotlin/JVM or C# | ok | 306/326 | 20873/21051 | 99.15 | 97.8 | 99.8 | 98.9 | 100.0 | 99.6 | 99.4 | 99.9 | 99.8 | 1.092 | 1069 | 61 | True | 326/326 | 1.7.0+a5bb40e | 2026-10-03 06:10 CST |
