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
$B graph --dataset $D < graph_request.json           # one GraphRequest -> one GraphResult (CONTRACT-GRAPHS 0.2)
$B graph-batch --dataset $D < graphs.jsonl > out.jsonl
```

RPC methods (contract 1.4.3): `calc` (FitRequest), `eft_parse` `{text}` -> FitRequest, `eft_export` `{fit,name?}` ->
`{text}`, `search` `{query,limit?=20,kinds?}` -> `[{type_id,name,name_zh,group,category_id,kind,slot,meta_level,match}]`,
`type` `{id}` (type id or name) -> `{type_id,name,name_zh,group,group_id,category_id,published,mass,volume,capacity,
meta_level,slot,attributes{name:value},effects[{id,name,default}]}` (unknown -> `UNKNOWN_TYPE` error), `meta`.
`eft_parse` output is identical to the reference engine (eve-dogma-rs) on all 326 bench EFT texts; `search` results
match it on the probe queries.

RPC `graph` takes a GraphRequest as params and returns the GraphResult.

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

## Graphs (eve-dogma-bench `graphs-round2`, CONTRACT-GRAPHS 0.2, Pyfa graph oracle)

All 10 graph types: Pyfa's 9 public graphs `damage`, `application_profile`, `ewar`, `remote_reps`, `capacitor`,
`shield_regen`, `mobility`, `warp_time`, `lock_time`, plus the hidden `ecm_burst` graph (0.2). Also from 0.2: the %
damage axes, ewar/RR target fits, empty `x.values`, and the error codes. Code: `src/graphs/`, notes in DESIGN.md.

| graph | cases | values |
|---|---|---|
| application_profile | 10/10 | 120/120 (charge ids, informational: 103/120) |
| capacitor | 14/14 | 173/173 |
| damage | 60/60 | 918/918 |
| ecm_burst | 11/11 | 218/218 |
| ewar | 22/22 | 337/337 |
| lock_time | 7/7 | 82/82 |
| mobility | 10/10 | 259/259 |
| remote_reps | 11/11 | 151/151 |
| shield_regen | 5/5 | 66/66 |
| warp_time | 8/8 | 93/93 |
| errors (expected error code) | 20/20 | 20/20 |
| **total** | **178/178** | **2437/2437** |

Extra oracle probes (not part of 0.2 scoring), no code change needed:
- G2's random probes (`graphs-g2/testdata/oracle-probes.jsonl`): 7493/7493.
- The bench's pending 0.3 probes (`graphs/pending/probes-0.3.jsonl`: sentries in follow_target, breacher range, bomb
  reactivation gaps, fighters vs fast targets, application-profile projected-cache grid, XL navy tier): 1367/1367.

Same result via `graph-batch`, one-process-per-case `graph`, and RPC `graph` (`run_graphs.py --batch-cmd / --cmd /
--rpc-cmd`). 178 requests in one `graph-batch` process: ~120 ms incl. 8 ms start-up (≈0.6 ms per request, release
build). Under 0.1 the score was 111/111 cases and 1843/1843 values. Results: `bench/graphs-scorecard.md` / `.json`. Bench manifest keys: `graph_cmd`, `graph_batch_cmd`.
