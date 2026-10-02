# variant-e — Pyfa-faithful Rust port

A stateless EVE Online fitting engine that **mechanically transpiles Pyfa's `eos/effects.py`** (2,400 effect
handlers) into Rust and ports Pyfa's calculation core (`ModifiedAttributeDict`, `Fit.calculateModifiedAttributes`,
module/drone/fighter stats, `capSim.py`) line by line. Implements the eve-dogma contract v1.2
(`eve-dogma-rs/docs/contract.md`).

**License: GPL-3.0-or-later** (it is a derivative work of Pyfa, GPL-3.0). See `LICENSE`.

## Build & run

```sh
cargo build --release                       # Rust 1.85+, edition 2024
B=./target/release/eve-dogma-e
D=/workspace/exct-eve/data/dataset-3569502.json.gz   # or $EVE_DOGMA_DATASET
$B calc --dataset $D < request.json > response.json  # one FitRequest -> one FitStats
$B calc --dataset $D request.json                    # same, from a file
$B batch --dataset $D < requests.jsonl > responses.jsonl
$B serve-stdio --dataset $D                          # JSONL RPC: {"id","method":"calc|meta","params"}
$B meta --dataset $D
```

Errors come back as JSON `{"error":{"code","message","path"}}` (exit code 1 for `calc`).

## Regenerating the transpiled handlers

```sh
python3 tools/pyfa2rs.py ../../ref/pyfa/eos/effects.py ../../data/dataset-3569502.json.gz src/generated/effects.rs
```

## Results (eve-dogma-bench 1.3.0, 249-case Pyfa-oracle corpus)

| cases | values | ms/fit (warm) | batch fits/s | cold start + calc |
|---|---|---|---|---|
| 249/249 | 13,812/13,812 (100 %) | 0.57 | 1015 | 160 ms |

Full scorecard: `bench/scorecard.md` / `bench/scorecard.json`. Bench manifest: `bench.yaml`.
Dev loop: `tools/devbench.sh` (scores the local build with `run.py`).
