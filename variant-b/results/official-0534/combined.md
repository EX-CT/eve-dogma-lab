# Combined scorecard

Generated 2026-10-03 05:34 CST — corpus: 295 cases, expected values from Pyfa (see README).

Bench version 1.5.0+2d6f9cb (see CHANGELOG.md). Rows may come from different runs: see measured_at/bench_version in combined.json; perf numbers are only comparable at similar load.

| variant | status | cases ok | values ok | accuracy % | application | capacitor | defense | fitting | navigation | offense | tank | targeting | ms/fit | fits/s (batch) | cold ms | deterministic | eft export | bench | measured |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| A eve-dogma-rs main (Rust, lazy memoised modifier graph) | ok | 249/249 | 13812/13812 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 1.430 | 460 | 216 | True | – | ? | ? |
| B data-oriented Rust | ok | 295/295 | 18978/18978 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 0.097 | 7530 | 9 | True | 295/295 | 1.5.0+2d6f9cb | 2026-10-03 05:33 CST |
| E Pyfa-faithful Rust port | ok | 295/295 | 18978/18978 | 100.00 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 0.503 | 2880 | 24 | True | – | 1.5.0+2d6f9cb | 2026-10-03 05:31 CST |
| K Kotlin/JVM or C# | ok | 292/295 | 18945/18978 | 99.83 | 99.1 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 100.0 | 1.382 | 483 | 89 | True | – | 1.5.0+2d6f9cb | 2026-10-03 05:29 CST |
