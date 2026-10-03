# PROGRESS — variant B

Updated: 2026-10-03 04:40 (Asia/Shanghai)

## Done
- New engine (`src/engine.rs`): flat raw-modifier list → packed-key sort → CSR node graph with compile-time constant
  folding → iterative on-demand DFS evaluation; staged recompilation for bursts and RAH.
- Skill folding with dataset-level proof of validity and automatic fallback; lazy per-(skill, level, structure) probes.
- Index-list target selection, dense attribute metadata, Pyfa-compatible everything (shared stats/capsim/eft).
- Feature parity with A @ 0e5a1ce: fleet booster fits, projected fits/modules/charges/drones, remote reps, neuts,
  nos, cap transfers, missile range.
- Fast dataset load (~65 ms vs ~140 ms), parallel ordered `batch`.
- Tests: oracle parity (249 cases, 13 812 values) green; full-output diff vs A: 249/249 byte-identical.
- Bench (`eve-dogma-bench/run.py`, shared noisy box): 249/249 cases, 13 812/13 812 values, cold start median 97 ms,
  batch 2 955 fits/s (all cores) / ~1 000–2 400 fits/s (1 thread, box load varies), rifter ≈ 0.29 ms/calc single
  thread (A ≈ 1.1–1.3 ms on the same run).

- Instruction counts vs A: see results/instructions.md (rifter 4.5×, hyperion 3.2×, fleet boosters 4.4× fewer).
- Robustness: packed sort key falls back to tuple sort for >65 535 items; errors per contract (BAD_REQUEST, UNKNOWN_TYPE, UNKNOWN_METHOD).

## Next
- `sweep`: multi-lane evaluation of K variants on one compiled plan.
- Cut remaining per-fit allocations (item effect lists, req_skills) and stats-layer string lookups.
- Track A's new features (diff tool flags them immediately).

## 2026-10-03 ~05:10 CST — snapshot cache
- bincode dataset snapshot keyed by SHA-256 of dataset bytes: process cold calc ~25 ms (was ~90 ms).
- Parity: 289/289 bench cases byte-identical vs A@0e5a1ce (with and without cache); oracle test passes.

## 2026-10-03 ~05:30 CST — ported A@086dcb4 + A@ae4bfb0
- sustainable tank, cap booster forced reload, ECM jam chance, projected fighters, fighter self abilities, drone/fighter application fields.
- tests/ synced from A@ae4bfb0. Parity: 289/289 bench cases byte-identical vs A@ae4bfb0; oracle test passes.

## 2026-10-03 ~05:35 CST — cold start + capsim
- snapshot: per-type lazy decode (`TypeTable`, dense id index), lazy names (zh / type_by_name), mmap'd cache file,
  dataset leaked at exit. Process cold calc (Rifter) ~25 ms -> ~8 ms.
- capsim fast path: per-stream static data + events packed into u128 keys in a std BinaryHeap (same comparisons ->
  same pops and same final heap layout, so results are bit-identical; randomised test vs the reference port),
  lazy comparator, exp() memo. Vexor-type fits ~1.15 ms -> ~0.6 ms wall.
- Parity: 289/289 byte-identical vs A@ae4bfb0; oracle 18 591 values.

## 2026-10-03 ~05:50 CST — contract 1.4.1/1.4.2 (A@e552cb9), cold start
- Ported: exit-2 error JSON, options default validate=true, interim search spec, Pyfa-exact EFT export (eft.rs/request.rs
  synced), dataset categories, projected tracking/guidance disruptors; mimalloc global allocator (as A).
- Lazy effects / dbuffs / mutaplasmids tables, zero-copy name indexes (FNV open addressing in the snapshot),
  precomputed skills_foldable: cold calc ~8 ms -> ~6.5 ms.
- bench.yaml: rpc_cmd (eft export column). Parity: 295/295 byte-identical vs A@e552cb9, eft_export 295/295,
  search outputs identical; oracle + eft_export_parity tests pass.

## 2026-10-03 ~06:20 CST — in-place snapshot tables
- LazyTable reads its header arrays in place from the mmap (no per-entry parsing/allocation at load), slots
  zero-allocated; cache key from file identity instead of hashing the gz. Process cold calc ~6.5 ms -> ~5 ms
  (median, loaded box). validate attr ids cached per dataset; rounding JSON formatter (no tidy pass).
- Parity: 297/297 bench cases byte-identical vs A@e552cb9, eft_export 306/306, tests pass.

## 06:30 CST — bench 1.8.0 (326 cases)
- Ported A 9f8579c (weather/AoE cloud beacons incl. drone buffs and unpenalised weather buffs, incursion system
  effects, burst projectors web/paint/damp/neut/ECM/track at full strength, Standup weapon disruptor, Breach
  Control). 326/326 byte-identical vs A@c629fb8; official quick run 326/326 cases, 21051/21051 values, eft 326/326.
- Snapshot v8: `Prepared` (attr meta, published skills, T3D modes, validate ids) and the skill-fold table with all
  12 (structure, level) probe values precomputed are stored in the snapshot (fold table lazily decoded per skill).
  Saves Prepared::new (~2.8M instr) and the per-process skill probes (~1.2M instr) in every cold process:
  single-calc ~9.5M -> ~5.5M instructions; cold median ~0.5-1 ms lower at load ~7. Output identical (diff vs A
  with and without cache).
- Tried: LSD radix sort for the modifier order — slower than pdqsort on the mostly presorted keys (+2% instr); reverted.
- 06:50 snapshot v10: groups and attrs are lazy tables too (no eager String/HashMap decode), the Names section is
  borrowed from the mapping instead of copied (~0.9 MB memcpy + page faults), AttrMeta is a repr(C) 24-byte record
  table read in place, and the start-up per-entry validation walks of the lazy tables / name indexes are gone
  (lookups re-check ids; entry bytes are cut with bounds-checked slices). Single cold calc (callgrind, rifter):
  9.5M -> 3.6M instructions this session; cold median ~6.4 -> ~4.6 ms at load ~7. Identical output (diff vs A with
  and without the cache, cargo test).
- 07:10 capsim fast path updates the current event in place (BinaryHeap::peek_mut, one sift-down) exactly like
  eve-dogma-rs 1db626a, so the heap layout (and EVE's stability sum order) now follows A rather than the old
  pop/push port: corpus 592.8M -> 567.1M instr (326 cases, batch --threads 1). 326/326 byte-identical vs A on the
  corpus and on a factor_reload-flipped variant corpus; the randomized fast-vs-reference test now allows 1e-12
  relative on eve_stable only (layout-dependent sum), all other fields bit-exact.
- 06:57 compile pre-sizes per-item node lists and the node table (exact counts, no regrowth; -8.4M), NameIndex
  reads through a cached base pointer (no Arc<Blob> enum deref per word), raw_cycle_ms burst-duration ids resolved
  once per stats call: corpus 567.1M -> 550.1M instr. Identical vs A (corpus + reload variants).
- 07:25 py_round2 fast path away from ties (A 8122ddd; rifter 1.44M -> 1.37M instr/fit). `batch` is now a
  streaming pipeline (reader -> bounded job queue -> N workers -> ordered writer thread) instead of 64*N-line
  chunks with a barrier per chunk and serial output: corpus x5 median 17.8k -> 19.9k fits/s (8 threads, same box,
  interleaved runs). Output byte-identical (md5 of the corpus batch at 1 and 4 threads == previous build).
  New test `snapshot_roundtrip`: snapshot-loaded dataset == parsed dataset on every tests/cases request.

## 07:50 CST — maintainability
- Tests 6 -> 33 (cargo test count): shared `tests/common`; `tests/api.rs` (errors, shape, skills, determinism, calc_many, EFT round
  trip of all 139 fits); `snapshot_roundtrip` fixed (it compared BAD_REQUEST errors: tests/cases are EFT+patch
  specs, not FitRequests); unit tests for the formula helpers, rounding, capsim behaviour, SHA-256, hashes; unit
  tests live in `src/tests/<module>.rs`.
- Runtime deps 8 -> 6: `sha2` replaced by the in-tree `sha256_hex` (now streaming, no input copy; same cache keys),
  `rustc-hash` by `src/hash.rs` (same integer mixing, so u32-keyed maps behave as before). Removed a stray
  `src/main.rs.orig`. Output identical (diff vs A 326/326 on corpus and reload variants; corpus batch md5 equal).
- Tried a direct JSON `Value` writer instead of serde's serializer: +1.1% instructions, reverted.

## 08:30 CST — speed (instructions, callgrind, batch --threads 1)
| step | rifter x300 | corpus (326) |
|---|---|---|
| start (5874d0e effect flags) | 409.2M | 528.9M |
| `out::J` output document instead of `serde_json::Value` | 361.4M | 486.7M |
| `TypeInfo::attr` binary search (attrs sorted at load) | 344.4M | 471.6M |
| consuming writer, in-place key sort, static keys unescaped | 338.4M | 465.9M |
| folded skills: stacking flag once per modifier | 336.7M | 464.1M |
Every step: diff vs A 326/326 byte-identical (bench corpus + factor_reload variants), corpus batch md5 unchanged,
cargo test green. Tried and reverted: hand-written string escape fast path (slower than serde_json's table).

## 08:45–09:45 CST — single-core per-calc latency (eve3 now measures (T_N−T_1)/(N−1) under taskset)
Instructions per run (callgrind, `batch --threads 1`); every step diff vs A 326/326 byte-identical (bench + reload
variants), corpus batch md5 unchanged, `cargo test --release` green.
| step | commit | rifter x300 | corpus (326) |
|---|---|---|---|
| (previous) compile sorted copy | 2dd7e13 | 331.1M | 458.4M |
| `attr_id!`/`effect_id!`: literal name lookups memoised per call site + dataset generation | 6f8c2f8 | 325.1M | 454.5M |
| output key sort: leading-byte compare before memcmp | 166a294 | 320.9M | 450.7M |
| validate: skill levels by binary search in the sorted skill list (no per-fit map) | 733b4b3 | 311.0M | 440.6M |
| compile: node bases via per-item cursors over patch/type_attrs | f14e554 | 298.0M | 425.5M |
| item request paths (error pointers) built lazily | 0b9b8a6 | 295.4M | 423.5M |
| CLI allocator: word-aligned layouts to `mi_malloc` (MiMalloc always took the aligned path) | a8179ed | 289.9M | 417.8M |
| compile: stable LSD radix sort on a compact (item, attr, op) key | b0bd286 | 287.3M | 415.2M |
| validate: merge canFitShip* ids against sorted module attrs, no vectors | ccbb56b | 280.6M | 409.5M |
| `out::own()`: move built sub-documents into `jv!` instead of deep clones | 779e043 | 273.2M | 401.6M |
Rifter: −17.5% instructions vs 2dd7e13. Also: capsim LCM saturating multiply (9efbe86; debug-build overflow in the
randomised capsim test), unit tests for `out` / stacking penalty / capsim (34 → 51 tests).
Tried and reverted: skip-sort when already ordered (+0.4%), dense ship attr→node table in `Fit::get` (+3%).
evaluate.py dry run at bcd80df (`results/eval-dryrun-0945`): gate pass 326/326, 21051/21051; single-core latency
0.099 ms/calc (5 pinned samples, n=4120), 21513 fits/s, cold 4.7 ms; maint 0.92 (tests 0.94 with 51 tests, deps
0.45 for 6 runtime deps), features 1.0, portability 0.5 (docs only: no wasm32 toolchain on the box to build/verify
a real WASM target), total 0.923 (B alone, dry run).
