# Variant B — compiled flat modifier graph ("compile, then evaluate")

Branch `variant-b` of EX-CT/eve-dogma-lab, directory `variant-b/`. Same stateless CLI/JSON contract as
eve-dogma-rs (A) and [eve-dogma-bench](https://github.com/EX-CT/eve-dogma-bench) `CONTRACT.md`;
same dataset (`dataset-3569502.json.gz`). Crate `eve-dogma-vb`, binary `eve-dogma-vb`, library `eve_dogma`
(drop-in API: `calc`, `calc_json`, plus `calc_many`).

## The problem

Dogma = for every item in a fit (ship, character, ~500 skills, modules, charges, drones, implants, …) every
attribute value is `base` transformed by modifiers from effects of other items, grouped by operator in a fixed
order (PreAssign, PreMul, PreDiv, ModAdd, ModSub, PostMul, PostDiv, PostPercent, PostAssign), with stacking
penalties for non-stackable attributes from non-exempt sources, min/max attribute caps, and a few special
effects without `modifierInfo` (AB/MWD, MJD, slot/hardpoint modifiers, command bursts, RAH, projected EWAR).

A (eve-dogma-rs main) builds an object graph with a `FxHashMap<attr, Attr>` per item and evaluates lazily with
per-attribute memo cells and recursion. Variant B deliberately does the opposite:

## Architecture

```
FitRequest ──build──▶ items: Vec<Item>            (base attrs = sorted patch Vec over the type's sorted slice)
           ──register▶ raw: Vec<RawMod>            (target item, attr, op, penalised, source ref), flat append
           ──compile──▶ CSR graph                  nodes = distinct (item, attr) targets; mods sorted by
                                                   (item, attr, op, seq) via one packed u64 key
                                                   sources resolved to Node(idx) | Const(value)
           ──evaluate─▶ vals: Vec<f64>             iterative DFS post-order, each node once, on demand
           ──stats────▶ FitStats JSON              (same formulas as A, reading the graph)
```

1. **No hash maps in the hot path.** Item attributes are `&[(attr, value)]` slices borrowed from the dataset plus a
   small sorted patch vector (mutations, overrides, security, skill level). Node lookup per item is a binary search
   over a sorted `(attr, node)` vector. Attribute metadata (default, stackable, high-is-good, min/max cap, rounding)
   is a dense `Vec<AttrMeta>` indexed by attribute id.
2. **Compile step.** All modifiers are first collected into a flat list, then sorted once by a packed 64-bit key
   `item(16) | attr(20) | op+1(4) | registration seq(24)`. Each distinct `(item, attr)` target becomes a dense node;
   modifiers form a CSR array (`mod_start[n]..mod_start[n+1]`), already grouped by operator so evaluation is a single
   linear pass per node. Every source reference is resolved at compile time: an attribute that nobody modifies is
   **constant-folded** to its base value, so evaluation never looks anything up.
3. **Evaluation.** Iterative DFS (explicit stack, no recursion) computes a node after its dependencies; each node is
   evaluated at most once into a flat `Vec<f64>`. Evaluation is *on demand*: only nodes reachable from what the
   stats layer reads are computed (typically 80–180 of 500–900 nodes). Cycles (none in the current SDE) read the
   on-stack node's base value — the same rule as A's lazy cycle guard, so results agree even then. Stacking
   penalties use two fixed 32-slot stack buffers (heap spill only beyond 32 penalised modifiers of one operator).
4. **Skill folding (dataset-level precompilation).** A skill's own attributes are only modified by the skill itself
   (`Prepared::new` *proves* this for the loaded dataset: no effect has a char-location or char-location-group
   modifier that could reach a skill group; otherwise folding is switched off and skills are instantiated as items).
   So instead of ~500 skill items with self-modifier subgraphs, each skill becomes a set of **constant** outgoing
   modifiers whose values come from a per-(skill, level, structure) table, probed once per process (lazily, via the
   same engine on a one-item fit, so semantics are identical). Skills whose outgoing modifiers cannot reach any item
   in the fit (no unconditional target and none of their skill filters is required by a fitted item) are skipped.
   This removed ~95 % of items (≈530 → ≈30 for a frigate) and ~40 % of graph nodes per fit.
5. **Target selection over index lists.** `Location*` / `OwnerRequiredSkill` targets come from per-fit index vectors
   (ship-location items, owned items, character items) and sorted `(required skill, item)` pair lists (binary-search
   range), instead of scanning all items per modifier.
6. **Staged compilation for value-dependent registration.** Command-burst buff ids are PostAssigned by charges and
   the Reactive Armor Hardener needs evaluated resonances; both run on a compiled graph, append modifiers, and
   recompile (cheap: compile is O(m log m) on ~1k modifiers). Fleet booster fits and projected fits are separate
   recursive `Fit::build` calls (frozen values), like A.
7. **Batch parallelism.** Requests are independent, the dataset (incl. the `OnceLock` fold tables) is `Sync`, a `Fit`
   is thread-local. `batch` is a streaming pipeline (reader -> bounded job queue -> N workers -> ordered writer thread with a
   reorder buffer; `--threads N`, default = available cores), output order preserved, byte-identical to
   `--threads 1`.
8. **Fast cold start.** Dataset JSON is parsed with a borrowed-key visitor straight into `Vec<(id, T)>` (no key
   `String`s, no intermediate `HashMap`s), SHA-256 (only reported in `meta`) runs on a second thread (in-tree
   `data::sha256_hex`, no crypto dependency; hidden behind the JSON parse), gzip via `zlib-rs`. Load ≈ 65 ms vs ≈ 140 ms.

## What is shared with A (and why)

The request/response types (`request.rs`), dataset structs (`data.rs`, loader rewritten), EFT import/export
(`eft.rs`), the capacitor simulator (`capsim.rs`) and the stats formulas (`stats.rs`) are taken from eve-dogma-rs
(LGPL-3.0, same org) so that the comparison isolates the **modifier engine architecture**; `engine.rs` (≈2.3k lines)
is a fresh implementation. Every special case (AB/MWD, MJD, bastion, structures, T3D default mode, RAH, bursts,
booster fits, projected fits, remote reps/neuts) is re-expressed in the compile/evaluate model.

## Verification

* `cargo test --release` (needs the dataset, `$EVE_DOGMA_DATASET` or `./dataset.json.gz`; dataset tests skip without it):
  * `tests/oracle_parity.rs`: Pyfa oracle parity (`tests/oracle/pyfa_expected.json`, same corpus as the bench).
  * `tests/eft_export_parity.rs`: EFT export byte-identical to Pyfa's `exportEft`, plus re-parse.
  * `tests/snapshot_roundtrip.rs`: a dataset loaded from snapshot bytes gives byte-identical output to the parsed
    dataset on every `tests/cases` request (cache format is output-neutral).
  * `tests/api.rs`: error responses (BAD_REQUEST, UNKNOWN_TYPE + path), output shape, skill effects on a bare hull,
    every case error-free, `calc_many` (1 and 4 threads) == sequential, no state leaking between calls, EFT
    export -> parse round trip of every `tests/fits` fit.
  * unit tests in `src/tests/<module>.rs` (`#[path]` test modules): capsim fast path vs the reference port
    (3000 randomized cases) and behaviour, stacking penalty, range/lock-time/spool-up formulas, Python-compatible
    rounding (`py_round2`, `float_unerr7`, EFT float repr), JSON writer, SHA-256 vectors, snapshot name-hash
    golden values, Fx hasher.
  * `tests/common/mod.rs`: shared helpers (dataset, EFT -> request, `tests/cases` corpus).
* `tools/diff_vs_a.py A_BIN B_BIN DATASET CASES`: **full-output differential test** against A on every bench case —
  every field of FitStats, not only the Pyfa-checked metrics. Current: 326/326 byte-identical vs A@fc66eaf (except `meta.engine`), also on a factor_reload-flipped variant corpus.
* `tools/ir_corpus.sh`: callgrind instructions for `batch --threads 1` over the whole corpus (`/tmp/corpus.jsonl`) —
  the number every perf commit quotes (326 cases: 621M at bench 1.7 start of session -> 550M).
* `tools/ir.sh CASE`: deterministic cost metric (callgrind instructions per calc) for optimisation work, immune to
  the noisy shared box.
* `eve-dogma-vb bench-phases CASE -n N`: per-phase timing (build, register, compile, staged, stats, capsim) and graph
  size (nodes, modifiers, evaluated nodes).

## Trade-offs

* + Speed: ~4× lower latency and ~3–4× higher single-thread throughput than A on the same box; ×cores in batch.
* + Predictable memory: a handful of flat vectors per fit; no per-attribute heap cells.
* + The compiled graph is an explicit artefact (nodes/CSR) — suited for future SIMD/multi-lane evaluation of many
  variants of one fit (same plan, different base vectors), sensitivity analysis and "explain" (sources per node).
* − More moving parts than lazy evaluation: anything that needs a value *during registration* must be staged.
* − Skill folding relies on a dataset property; it is checked at load and falls back automatically, but a future SDE
  that breaks it loses the speed-up (still correct).
* − Lazy-on-demand evaluation keeps `get()` behind `RefCell`s, so a `Fit` is not `Sync` (batch parallelism is
  across fits, not inside one).

## Not done / next

* Multi-lane plan reuse (`sweep`: evaluate K skill/override variants of one fit on one compiled graph).
* Capacitor simulation: `capsim::simulate_fast` (packed u128 heap keys, static per-stream data, exact exp memo,
  in-place top update like A 1db626a) with `simulate_ref` (the plain Pyfa port) as fallback on NaN / overflow /
  `VB_CAPSIM_REF`. Still ~12% of corpus instructions (long simulations, e.g. 5000-iteration weather fits).
* JSON output is still built as a `serde_json::Value` tree (~20% of per-fit cost: BTreeMap inserts, drops,
  serialisation). A direct `Value` walker replacing serde's serializer was tried (07:20) and was not faster
  (+1% instructions); the win needs skipping the tree, i.e. writing sections straight to the output buffer.
* WASM build (no threads, no mmap) is straightforward: the only native-code dependency is mimalloc (optional);
  zlib-rs and the in-tree SHA-256/Fx hash are pure Rust.

## Cold start: dataset snapshot cache
`data::load_path` keys a cache file on SHA-256 of (absolute path, size, mtime, inode, device) of the dataset — no
content read on a hit — and memory-maps `<cache>/<key>-v<crate>-<SNAPSHOT_VERSION>.bin`. Miss: gunzip + JSON parse +
index build, then the snapshot is written atomically (tmp + rename). Cache dir: `$EVE_DOGMA_CACHE`, else
`$XDG_CACHE_HOME/eve-dogma-vb`, else `~/.cache/eve-dogma-vb`, else tmp. `EVE_DOGMA_NO_CACHE=1` disables it;
`VB_LOAD_TIMING=1` prints load phases.

### Snapshot format v10 (everything lazy or in place)
File = magic `EVEDVB04` + 13 8-aligned, length-prefixed sections:

| # | content | access |
|---|---|---|
| 0 | bincode: build, release date, sha256, categories, skills, skills_foldable | eager (tiny) |
| 1 | bincode Names (zh names, type_by_name) | borrowed from the mapping, decoded on first use |
| 2–5 | types / effects / dbuffs / mutaplasmids | `LazyTable` |
| 6–7 | attr / effect name -> id | `NameIndex` (open addressing, read in place) |
| 8 | bincode PreparedCore (published skills, T3D modes, validate attr ids) | eager (small) |
| 9 | skill folds with all 12 (structure, level) probe values precomputed | `LazyTable<Option<SkillFold>>` |
| 10–11 | groups / attributes | `LazyTable` |
| 12 | dense `AttrMeta` (`repr(C)`, 24-byte records) | read in place (bool bytes checked at load) |

`LazyTable`: u32 header arrays used in place (sorted ids, aux = group for types, entry offsets, dense id -> pos
index); each entry is bincode-decoded on first `get` and published with a CAS into a zeroed slot array. No
start-up walk over entries: lookups re-check ids and entry bytes are cut with bounds-checked slices, so a corrupt
file can only panic, never read out of bounds. Effect: a cold `calc` of a Rifter is ~3.6M instructions in total
(was 9.5M with v7, ~90 ms wall before the cache existed); remaining cold time is process start + page faults.
