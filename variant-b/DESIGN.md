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
   is thread-local. `batch` evaluates chunks on all cores (`--threads N`), output order preserved, byte-identical to
   `--threads 1`.
8. **Fast cold start.** Dataset JSON is parsed with a borrowed-key visitor straight into `Vec<(id, T)>` (no key
   `String`s, no intermediate `HashMap`s), SHA-256 (only reported in `meta`) runs on a second thread with SHA-NI
   (`sha2`), gzip via `zlib-rs`. Load ≈ 65 ms vs ≈ 140 ms.

## What is shared with A (and why)

The request/response types (`request.rs`), dataset structs (`data.rs`, loader rewritten), EFT import/export
(`eft.rs`), the capacitor simulator (`capsim.rs`) and the stats formulas (`stats.rs`) are taken from eve-dogma-rs
(LGPL-3.0, same org) so that the comparison isolates the **modifier engine architecture**; `engine.rs` (≈1.6k lines)
is a fresh implementation. Every special case (AB/MWD, MJD, bastion, structures, T3D default mode, RAH, bursts,
booster fits, projected fits, remote reps/neuts) is re-expressed in the compile/evaluate model.

## Verification

* `cargo test --release`: Pyfa oracle parity (`tests/oracle/pyfa_expected.json`, same corpus as the bench).
* `tools/diff_vs_a.py A_BIN B_BIN DATASET CASES`: **full-output differential test** against A on every bench case —
  every field of FitStats, not only the Pyfa-checked metrics. Current: 249/249 byte-identical (except `meta.engine`).
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
* The capacitor simulator dominates some fits (e.g. 1 ms of the Vexor's 1.3 ms); it is Pyfa-exact event simulation
  shared with A and was not optimised.
* WASM build (no threads) is straightforward: no native deps besides zlib-rs/sha2 (both pure Rust).

## Cold start: dataset snapshot cache
`data::load_path` hashes the raw `.json.gz` bytes (SHA-256) and looks for
`<cache>/ds-v1-<hash>.bin` (bincode). Hit: deserialize (~10 ms) instead of
gunzip + JSON parse + index build (~65-85 ms). Miss: normal load, then write the
snapshot atomically (tmp + rename). Cache dir: `$EVE_DOGMA_CACHE`, else
`$XDG_CACHE_HOME/eve-dogma-vb`, else `~/.cache/eve-dogma-vb`, else tmp.
`EVE_DOGMA_NO_CACHE=1` disables it. Keyed by content hash, so a changed
dataset can never serve stale data. `VB_LOAD_TIMING=1` prints load phases.

### Snapshot format v7 (in-place tables)
File = magic `EVEDVB04` + 8-aligned, length-prefixed sections: bincode(eager part: groups, categories, attrs,
skills, skills_foldable), bincode(names: zh + type_by_name, decoded on first use), then lazily read tables for
types / effects / dbuffs / mutaplasmids (u32 header arrays used in place from the mmap: sorted ids, aux = group,
entry offsets, dense id -> pos index; each entry bincode-decoded on first `get`, published with a CAS into a
zero-initialised slot array) and two name indexes (FNV-style hash, open addressing) for attr/effect name -> id.
Cache key = SHA-256 of (absolute path, size, mtime, inode, device) of the dataset file — no content read on a hit.
