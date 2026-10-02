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
