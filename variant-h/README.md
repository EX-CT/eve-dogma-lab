# Variant H — Rust ECS dogma engine (hecs)

Stateless EVE Online fitting engine implementing the EXCT contract v1 (`eve-dogma-rs/docs/contract.md`):
one JSON `FitRequest` on stdin → one JSON `FitStats` on stdout.

## Build

```bash
cd variant-h
cargo build --release          # Rust >= 1.85
```

## Run

```bash
./target/release/eve-dogma-h --dataset /workspace/exct-eve/data/dataset-3569502.json.gz calc < request.json > response.json
./target/release/eve-dogma-h --dataset /workspace/exct-eve/data/dataset-3569502.json.gz batch < requests.jsonl > responses.jsonl
```

Dataset: `--dataset PATH`, else `$EVE_DOGMA_DATASET`, else `./dataset.json.gz`, else the shared box copy.
Other commands: `serve-stdio` (JSONL RPC, methods `calc`, `meta`), `meta`, `bench FILE -n N`.

See [DESIGN.md](DESIGN.md) for the architecture and [RESULTS.md](RESULTS.md) for scores.
