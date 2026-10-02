# Combined scorecard

Generated 2026-10-03 06:06 CST — corpus: 306 cases, expected values from Pyfa (see README).

Bench version 1.7.0+a5bb40e (see CHANGELOG.md). Rows may come from different runs: see measured_at/bench_version in combined.json; perf numbers are only comparable at similar load.

| variant | status | cases ok | values ok | accuracy % | application | capacitor | defense | fitting | navigation | offense | tank | targeting | ms/fit | fits/s (batch) | cold ms | deterministic | eft export | bench | measured |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| A eve-dogma-rs main (Rust, lazy memoised modifier graph) | ok | 306/306 | 19621/19621 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 0.551 | 1634 | 124 | True | 306/306 | 1.7.0+a814f99 | 2026-10-03 05:57 CST |
| C Go | ok | 297/297 | 19103/19103 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 0.245 | 3687 | 62 | True | 297/297 | 1.6.0+a814f99 | 2026-10-03 05:46 CST |
| E Pyfa-faithful Rust port | ok | 306/306 | 19621/19621 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 0.216 | 2993 | 12 | True | 306/306 | 1.7.0+a5bb40e | 2026-10-03 06:04 CST |
| F codegen Rust/WASM | ok | 306/306 | 19621/19621 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 0.081 | 10072 | 3 | True | 306/306 | 1.7.0+a5bb40e | 2026-10-03 06:03 CST |
| H Rust ECS | ok | 297/297 | 19103/19103 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 0.308 | 2863 | 6 | True | 297/297 | 1.6.0+a814f99 | 2026-10-03 05:51 CST |
| I Rust salsa incremental | ok | 306/306 | 19621/19621 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 0.179 | 880 | 19 | True | 306/306 | 1.7.0+a5bb40e | 2026-10-03 06:04 CST |
| J C++20 | ok | 306/306 | 19621/19621 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 0.045 | 16286 | 2 | True | 306/306 | 1.7.0+a5bb40e | 2026-10-03 06:04 CST |
| K Kotlin/JVM or C# | ok | 297/306 | 19588/19621 | 99.83 | 99.5 | 100.0 | 99.9 | 100.0 | 99.7 | 99.8 | 100.0 | 99.9 | 1.016 | 960 | 107 | True | 306/306 | 1.7.0+a5bb40e | 2026-10-03 06:04 CST |
