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
EVE_DOGMA_DATASET=$D cargo test --release                             # Pyfa oracle parity
python3 tools/diff_vs_a.py A_BIN ./target/release/eve-dogma-vb $D ../eve-dogma-bench/cases   # full diff vs A
```

Bench manifest: [bench.yaml](bench.yaml). Results: [results/](results/).

License: LGPL-3.0-or-later (shared modules derived from eve-dogma-rs; capacitor simulation and RAH follow Pyfa/eos,
LGPL). EVE Online data © CCP hf.
