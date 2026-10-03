# Variant F: performance plan (branch `variant-f-perf`)

Base: `af1c04b` (= variant-f-formats head; calc output byte-identical to the scored `bc84e2b`).
Goal: close the gap to J (C++20) on the bench `ms/fit` column: **J 0.043 ms/fit** vs **F 0.064–0.083 ms/fit**
(official combined scorecard 0.083 at 07:30 CST; our own runs 0.026–0.081 depending on box load).
The plan is written from profiles and code reading only (09:45–10:00 CST measurements, no new runs while writing).

## 1. What the bench actually measures

`run.py` (bench 1.8.0) `ms/fit` = `latency_one_fit` = (time of `batch` with **500 × exct_rifter** − time with 1 ×) / 499,
through a Python pipe. F's native `batch` is multi-threaded (N = cores, ordered output), so the column is
"rifter throughput on a shared 8-core box". The `fits/s` column is the 326-case corpus × 5 through the same pipe.

Measured on the box (load ≈ 2, best of 7, `/tmp/fp-base` = af1c04b release):

| `EVE_DOGMA_THREADS` | cold (1 fit) | rifter latency metric |
|---|---|---|
| 1 | 2.09 ms | 0.116 ms/fit |
| 2 | 2.41 ms | 0.070 ms/fit |
| 4 | 2.67 ms | 0.046 ms/fit |
| 8 | 2.79 ms | 0.035 ms/fit |

So on an idle box F already matches J. Under the official run's load (6–9 on 8 cores) the threads get starved and the number falls back
towards the single-thread cost. **The lever that survives load is single-thread cost per rifter fit: ~110 µs.**
With 8 threads at load ≈ 8, that's roughly 110 / (8 − load share) ≈ 0.06–0.08 ms, which matches the scorecard.
Target: **≤ 60 µs single-thread per rifter fit** (≈ 0.04 ms/fit under official load).

Single-thread breakdown for exct_rifter. `bench` gives the phases; the `batch` difference gives parse + IO:

| phase | µs | share | source |
|---|---|---|---|
| request parse (serde `FitRequest`) + line IO + write | ~20–24 | 20 % | batch(1 thread) 110 µs − bench 86 µs |
| `Fit::build` (items, folded skills, `register_all`) | ~40 | 36 % | `bench` build_us |
| `compute_stats` (incl. capsim) | ~29 | 26 % | `bench` stats_us |
| JSON serialisation (`J` tree → String, ~6.8 KB/fit) | ~20 | 18 % | `bench` serialize_us |

callgrind on exct_rifter × 300 (`/tmp/rifter.cg`; ≈ 1.57 M Ir per calc). The rows below are inclusive unless marked:

| function | share | notes |
|---|---|---|
| `Fit::build` | 44 % | — |
| `register_all` | 34 % | — |
| `apply_skill` (generated + engine) | 29 % | ≈ 435 calls/calc |
| `Fit::push` | 20 % | ≈ 450 calls/calc |
| `Fit::ensure` | 20 % | ≈ 500 calls/calc; sorted-Vec insert ⇒ memmove 5 % |
| `Fit::get` | 15 % | — |
| `type_attr` | 10 % | binary search in the static per-type attr list; self 6.6 % |
| `compute_stats` | 37.5 % | — |
| capsim `Ev` Vec growth | 5 % | ≈ 130 `grow_one`/calc |
| `J::write` | 14 % | — |
| `write_str` | 6 % | — |
| `drop_glue<(Cow<str>, J)>` | 4 % | — |
| key index sort | ~2.7 % | — |
| malloc/realloc/free (self) | ~7 % | — |
| memcpy (self) | 5 % | — |

Corpus profile (326 fits, `/tmp/fp.cg`, 483 M Ir): calc 80 %; compute_stats 55 % (capsim 23 %, `EvHeap::push` 9 %, exp 3.5 %);
build 24 %; JSON 14 %; serde parse 3.7 %; malloc ~5 %.

## 2. Ideas, ordered by gain ÷ risk

"Parity risk" means the risk of changing any output byte. The gate requires byte-identical batch output
(sha256 `214f6192…e847a` over `/tmp/allcases.jsonl`). An idea that changes the **order or grouping of float operations**
(e.g. the modifier fold order in `combine`, stacking-penalty sort, the capsim event order) is high risk.
Pure data-structure, allocation and IO changes are zero risk as long as the order of modifiers on each attribute is unchanged.

| # | idea | where | expected gain (rifter, single thread) | parity risk |
|---|---|---|---|---|
| P1 | **Dense attribute index for ship + character** (items 0/1 hold ~160 dyn attrs each): replace the sorted `Vec<(attr, slot)>` + binary search + `insert` (memmove) with a direct `[u32; ATTR_N]` slot table (u16 attr ids are compiled dense). Other items keep the small sorted Vec, but with a linear scan for n ≤ 8. | `engine.rs` `ensure` / `slot_of` / `set_base` | −8…−12 µs (ensure 20 % + memmove 5 % of build) | none (slot numbering and modifier list order unchanged) |
| P2 | **Stream JSON directly** instead of building the `J` tree: generated writers with keys pre-sorted at build time, no `Cow` keys, no per-object index sort, no tree drop. Keep `J` for the non-hot paths (`to_value_raw`, stats text export). | `j.rs`, `stats.rs` output section | −8…−12 µs (most of the 20 µs serialise phase) | low: same `round6` + zmij per float, same key order. Verify with the sha256 gate. |
| P3 | **Reuse per-thread scratch** (`thread_local!` arena): `Fit.items`, `slots`, `mods`, `dyn_attrs` buffers, capsim `EvHeap`/`awaiting`, output `String` with capacity 8 KB. `clear()` instead of reallocating each calc. | `engine.rs` `Fit::build`, `capsim.rs`, `lib.rs::calc_json` | −5…−8 µs (malloc 7 % + RawVec grow + capsim grow 5 %) | none |
| P4 | **mimalloc as the native global allocator** (vendored crate, MIT; not on wasm32). Measured: corpus ×100 to /dev/null 0.0279 → 0.0259 ms/fit, ×5 0.035 → 0.031; rifter `bench` build 37–44 → 35 µs, serialise 19–22 → 17–18 µs. | `main.rs`, `Cargo.toml` (target-gated dep) | −3…−7 % (overlaps P3; keep it only if it still pays after P3) | none |
| P5 | **Skill fold pre-filter**: `apply_folded_skills` calls `apply_skill` for every trained skill (≈ 435/calc) even when the skill's generated modifiers have no target in this fit. At build time, emit for each skill a bitmask of the target kinds it touches (ship / char / by-skill / by-group). At runtime, skip a skill when none of its `by_skill`/`by_group` ranges are non-empty and it has no ship/char modifiers. Also hoist `range_of` results out of the generated loops. | `build.rs` codegen, `engine.rs` | −5…−10 µs (apply_skill self + call overhead ≈ 10–15 % of build) | none if skipped skills had no push (pure no-op elision); the push order of the remaining skills is unchanged |
| P6 | **Borrowed request parse**: `FitRequest` with `&str`/`Cow` and small fixed enums, plus skipping unknown fields without allocation. Pre-size `Vec`s (CargoReq grow seen in profile). | `request.rs` | −3…−6 µs of the ~20 µs parse/IO | none (input side only) |
| P7 | **Batch IO**: single-thread path flushes per line; the parallel path uses `Mutex<Receiver>` + `BTreeMap` reorder. Switch to per-worker chunking (k lines per job) with a ring of `Option<String>` slots, and write via one `BufWriter` with flush only when the reader has caught up. | `main.rs::batch` | −2…−5 µs effective at 8 threads; reduces contention under load | none |
| P8 | **`type_attr` fast path**: cache the base value in the slot when it is created (already done for ensured attrs). For read-only `get` of an unmodified attr, add a per-type `ATTR_N`-bit presence bitmap so absent attrs skip the binary search, or use a perfect-hash (CHD) table generated per type. | `build.rs`, `data.rs` | −3…−5 µs (type_attr 10 %) | none |
| P9 | **capsim micro-work**: reuse `key()` Vec buffers at each period wrap (they allocate twice per wrap now); reuse the `good` Vec in the injector loop; pre-size `EvHeap` to the number of drains + injectors. **Do not** change the event comparison, the `exp` memo, or the `powi(2)` form. | `capsim.rs` | corpus −3…−6 % (capsim 23 % of corpus); rifter −1…−2 µs | none for buffer reuse. Any change to the event ordering or the recharge formula is **high risk**: excluded. |
| P10 | Early-out "cap stable" detection (skip the sim when recharge at peak ≥ total drain) | `capsim.rs` | corpus up to −10 % | **high**: Pyfa's reported stable % comes from the sim; analytic shortcuts change the last ulp. Only if the result is provably bit-identical (e.g. the sim would end at the first wrap). Deferred; measure the share of stable fits first. |
| P11 | `compute_stats` sorts (damage-type / stacking order): use pre-sorted inputs or `sort_unstable` on small arrays with the same comparator | `stats.rs` | −1…−2 µs | low (stable vs unstable sort changes tie order ⇒ keep a stable sort wherever ties can carry different floats) |

Not planned: `-C target-cpu=native` (the official box CPU may differ; FMA contraction is off in Rust anyway, but it's not worth it);
fast-math / FMA (breaks parity); PGO (toolchain lacks llvm-profdata offline; revisit).
`lto = "fat"`, `codegen-units = 1` and `panic = "abort"` are already on.

Expected total, single-thread rifter: 110 µs → ~60–70 µs (P1–P6). Under official load that's ≈ 0.045–0.05 ms/fit.
Idle box at 8 threads ≈ 0.02 ms/fit.
WASM (single thread, wasmtime) gains the same proportion except P4/P7: 0.214 → ~0.13 ms/fit expected.

## 3. Procedure and gate (every step = one commit, pushed)

1. Implement one idea at a time in the order P1, P3, P2, P5, P6, P8, P9, P7, P4. Re-measure after each:
   - `bench cases/exct_rifter.json -n 20000` (phase µs)
   - batch single-thread rifter × 500
   - callgrind Ir/calc, the stable metric on a noisy box
2. Gate per commit, native and wasm32-wasip1:
   - `batch < /tmp/allcases.jsonl | sha256sum` must equal `214f6192639dfd6107a6d89d24139848d60f10182c84a49e071201f9a91e847a` (byte-identical calc output);
   - bench 1.8.0 `run.py` 326/326 and 21051/21051;
   - EFT export 326/326 (`check_eft_export.py`);
   - formats 100 % (`evaluate_formats.py --rpc`, 4779/4779 scored).
3. Record before/after in `RESULTS.md`, with the box load next to every perf number.
   `variant-f` itself is not touched: its fast-forward to af1c04b is handled separately.

Provenance: all ideas are original engineering on F's own code. Pyfa (GPL) is used only as an output oracle via the bench.
mimalloc (P4) is MIT-licensed.
