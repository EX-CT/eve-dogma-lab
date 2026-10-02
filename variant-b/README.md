# eve-dogma-vb — EX-CT dogma lab, variant B (compiled flat modifier graph, Rust)

Stateless EVE Online fitting engine: one JSON `FitRequest` in → one JSON `FitStats` out, same contract as
[eve-dogma-rs](https://github.com/EX-CT/eve-dogma-rs) and [eve-dogma-bench](https://github.com/EX-CT/eve-dogma-bench).
Architecture and trade-offs: [DESIGN.md](DESIGN.md). Status: [PROGRESS.md](PROGRESS.md).

```bash
cargo build --release
D=/workspace/exct-eve/data/dataset-3569502.json.gz      # or $EVE_DOGMA_DATASET / ./dataset.json.gz
./target/release/eve-dogma-vb --dataset $D calc req.json              # FitRequest -> FitStats
./target/release/eve-dogma-vb --dataset $D batch [--threads N] < reqs.jsonl   # JSONL, parallel, ordered
./target/release/eve-dogma-vb --dataset $D serve-stdio                 # JSONL RPC (calc|eft_parse|eft_export|search|type|meta)
./target/release/eve-dogma-vb --dataset $D eft fit.eft --calc --skills 5
./target/release/eve-dogma-vb --dataset $D bench-phases req.json -n 1000   # per-phase timing + graph size
EVE_DOGMA_DATASET=$D cargo test --release                             # oracle + EFT parity, API, snapshot, unit tests
python3 tools/diff_vs_a.py A_BIN ./target/release/eve-dogma-vb $D ../eve-dogma-bench/cases   # full diff vs A
```

Bench manifest: [bench.yaml](bench.yaml). Results: [results/](results/) (latest official: `results/official-0752`,
bench 1.8.0+33db85a: 326/326 cases, 21051/21051 values, eft export 326/326, 0.054 ms/fit, 8084 fits/s, cold 10 ms;
perf numbers are from a shared box at load ≈ 7–8 on 8 cores and vary between runs).

Tests: `tests/*.rs` (integration, see DESIGN.md#verification) and `src/tests/<module>.rs` (unit). Runtime
dependencies: serde, serde_json, bincode, flate2 (zlib-rs), memmap2, mimalloc.

Perf tooling: `tools/ir_corpus.sh` (callgrind instructions over the corpus, deterministic), `tools/startup.py`
(cold-process median), `tools/batchtime.py BIN threads...`. Env: `EVE_DOGMA_NO_CACHE`, `EVE_DOGMA_CACHE=DIR`,
`VB_LOAD_TIMING=1`, `VB_CAPSIM_REF=1` (reference capacitor simulation).

License: LGPL-3.0-or-later ([LICENSE](LICENSE) = LGPL-3.0 text, [LICENSE.GPL-3.0](LICENSE.GPL-3.0) = the GPL-3.0 it
supplements; policy: eve-fit-docs/LICENSING.md; shared modules derived from eve-dogma-rs; capacitor simulation and RAH follow Pyfa/eos,
LGPL). EVE Online data © CCP hf.
