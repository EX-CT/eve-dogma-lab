# variant-e — Pyfa-faithful Rust port

A stateless EVE Online fitting engine that **mechanically transpiles Pyfa's `eos/effects.py`** (2,400 effect
handlers) into Rust and ports Pyfa's calculation core (`ModifiedAttributeDict`, `Fit.calculateModifiedAttributes`,
module/drone/fighter stats, `capSim.py`) line by line. Implements the eve-dogma contract v1.5 + rulings 1.4.1/1.4.2
(`eve-dogma-bench/CONTRACT.md`).

**License: GPL-3.0-or-later** (it is a derivative work of Pyfa, GPL-3.0). See `LICENSE`.

## Build & run

```sh
cargo build --release                       # Rust 1.85+, edition 2024
B=./target/release/eve-dogma-e
D=/workspace/exct-eve/data/dataset-3569502.json.gz   # or $EVE_DOGMA_DATASET
$B calc --dataset $D < request.json > response.json  # one FitRequest -> one FitStats
$B calc --dataset $D request.json                    # same, from a file
$B batch --dataset $D < requests.jsonl > responses.jsonl
$B serve-stdio --dataset $D                          # JSONL RPC: {"id","method":"calc|eft_parse|eft_export|search|type|meta","params"}
$B eft fit.txt --dataset $D [--calc]                 # EFT text -> FitRequest (or FitStats with --calc)
$B search "Shield Ext" --dataset $D                  # type search (interim spec: exact > prefix > substring, en/zh)
$B type 587 --dataset $D                             # type info by id or name
$B meta --dataset $D
```

RPC methods (contract 1.4.3): `calc` (FitRequest), `eft_parse` `{text}` -> FitRequest, `eft_export` `{fit,name?}` ->
`{text}`, `search` `{query,limit?=20,kinds?}` -> `[{type_id,name,name_zh,group,category_id,kind,slot,meta_level,match}]`,
`type` `{id}` (type id or name) -> `{type_id,name,name_zh,group,group_id,category_id,published,mass,volume,capacity,
meta_level,slot,attributes{name:value},effects[{id,name,default}]}` (unknown -> `UNKNOWN_TYPE` error), `meta`.
`eft_parse` output is identical to the reference engine (eve-dogma-rs) on all 326 bench EFT texts; `search` results
match it on the probe queries.

Errors come back as JSON `{"error":{"code","message","path"}}` (exit code 2 for `calc`, 3 if the dataset cannot be loaded).

## Regenerating the transpiled handlers

```sh
python3 tools/pyfa2rs.py ../../ref/pyfa/eos/effects.py ../../data/dataset-3569502.json.gz src/generated/effects.rs
```

## Results (eve-dogma-bench 1.8.0+0969967, 326-case Pyfa-oracle corpus, official `bench.py --only E`)

| cases | values | ms/fit (warm) | batch fits/s | cold start + calc | EFT export vs Pyfa |
|---|---|---|---|---|---|
| 326/326 | 21,051/21,051 (100 %) | 0.124 | 4227 | 7.6 ms | 326/326 |

Cold start uses a bincode cache of the parsed dataset (`$EVE_DOGMA_E_CACHE`, else `$XDG_CACHE_HOME/eve-dogma-e` or `~/.cache/eve-dogma-e`). The cache is keyed by a hash of the dataset bytes and rebuilt automatically when it is missing. Type records stay serialised in the cache and are decoded lazily on first use, so a one-fit `calc` only decodes the types it touches.

Contract rulings applied: explicit `fleet.buffs` override booster fits and the fit's own bursts per buff id (aggregated min/max by the buff's aggregate mode); projected `amount` = that many copies (also for projected fits); `use_gj_s` per contract 1.4.2 (identical to A on the corpus).

Full scorecard: `bench/scorecard.md` / `bench/scorecard.json`. Bench manifest: `bench.yaml`.
Dev loop: `tools/devbench.sh` (scores the local build with `run.py`).
