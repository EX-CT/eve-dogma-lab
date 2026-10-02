# Variant I — incremental dogma engine on salsa (Rust)

Implements the eve-dogma stateless contract (EX-CT/eve-dogma-rs `docs/contract.md`): one FitRequest JSON in,
one FitStats JSON out. Attribute evaluation, modifier registration and modifier layers are salsa queries;
`batch` / `serve-stdio` keep one salsa database across requests, so related requests only recompute what changed.

```bash
cargo build --release
D=/workspace/exct-eve/data/dataset-3569502.json.gz
./target/release/eve-dogma-i calc --dataset $D < request.json > response.json
./target/release/eve-dogma-i batch --dataset $D < requests.jsonl > responses.jsonl   # incremental session
./target/release/eve-dogma-i batch --fresh --dataset $D < requests.jsonl             # new database per request
./target/release/eve-dogma-i serve-stdio --dataset $D                                 # JSONL RPC
./target/release/eve-dogma-i bench req.json -n 1000 [--fresh]
```

License: LGPL-3.0-or-later. Request/stat/capsim/EFT modules and the modifier semantics are derived from
EX-CT/eve-dogma-rs (LGPL-3.0-or-later, variant A); the engine core (`spec.rs`, `engine.rs`, `session.rs`) is new.
No Pyfa (GPL) code is included; Pyfa is only used as a black-box oracle by the bench.

Bench results: see `bench/` (eve-dogma-bench scorecards).

## Dataset cache (disclosure)
Loading the dataset JSON.gz takes ~140 ms, so variant I keeps a *derived* binary cache, which the brief allows ("you may add a derived cache").
The first load writes `$TMPDIR/eve-dogma-i-<version>-<hash>.bin`, keyed by a hash of the exact dataset file bytes, and later processes read it in ~10 ms.
A corrupt or stale cache is ignored and rewritten. `EVE_I_NO_CACHE=1` disables the cache and `EVE_I_CACHE_DIR=...` moves it.
Cold-start numbers in the scorecard are measured with a warm cache, because the first bench process creates it.
