# Variant H: results

## Official bench (`bench.py --only H`, full run)

Bench **1.8.0+0969967** (frozen for the 10:20 CST evaluation; 326 cases, 21 051 Pyfa-expected values, adding abyssal
weather / AoE cloud beacons, incursion system effects, burst projectors and Standup weapon disruptors). Official run in the
shared bench checkout at 07:57 CST on commit 9fbeb2a (bench 1.8.0+33db85a, corpus unchanged since 0969967);
earlier runs: 07:28 CST on 65e1eda, 07:01 CST on ab420d1 and 06:19 CST on b430abe, both 326/326. Scorecards are committed in [`scorecards/`](scorecards/):
[`bench-1.8.0.md`](scorecards/bench-1.8.0.md) and [`bench-1.7.0.md`](scorecards/bench-1.7.0.md), each with a `.json`.

| | cases | values |
|---|---|---|
| vs Pyfa (bench 1.8.0) | **326/326** | **21051/21051 (100.00 %)** |
| vs Pyfa (bench 1.7.0, 06:12 CST, f4d4b88) | 306/306 | 19621/19621 |
| vs Pyfa (bench 1.6.0, 05:51 CST) | 297/297 | 19103/19103 |
| eft export column (EFT export vs Pyfa `exportEft` after `fill()`, informational) | **326/326** | |

| perf (bench harness, shared box under load) | H @ 1.8.0 07:57 (9fbeb2a) | H @ 1.8.0 07:28 (65e1eda) | H @ 1.8.0 07:01 (ab420d1) | H @ 1.8.0 06:19 | H @ 1.7.0 06:12 | H @ 1.6.0 05:51 | A @ 1.6.0 05:42 |
|---|---|---|---|---|---|---|---|
| ms/fit (exct_rifter latency) | **0.237** | 0.381 | 0.506¹ | 0.281 | 0.255 | 0.308 | 0.432 |
| batch throughput, fits/s | **2469** | 1934 | 2885 | 2662 | 3033 | 2863 | 1777 |
| cold ms (one process per case, median) | **5.0** | 6.7 | 5.2 | 4.8 | 6.2 | 8.9 | 138 |
| startup + one calc, ms | **7.3** | 20.2 | 7.9 | 6.2 | 6.0 | 7.8 | |
| deterministic | yes | yes | yes | yes | yes | yes | yes |

¹ Load average was 7.4 during that run. Timed back to back under the same load, ab420d1 does 2887 fits/s
(0.346 ms/fit including startup), against 2528 fits/s for the earlier reference build, so the latency figure is
load noise, not a regression. The 07:28 run had a load average of about 8; so did the 07:57 run.

9fbeb2a is a speed-only change (841edda + 9fbeb2a): a fast path in `LazyTable` lookups, pre-sized ship/character
attribute maps and calc memo, a typed request parse on the first attempt, skill levels built from the pre-sorted
published-skill list without a per-request sort, and request type ids collected by a serde `Serializer` instead of a
`serde_json::Value` round-trip. Outputs are byte-identical to de77832 on 13 039 requests, malformed ones included.
Instructions per 500 Rifter calcs (callgrind, including startup) dropped from 1188.5M to 1026.1M (−14 %); back-to-back
A/B under the same load: latency median 0.257 → 0.212 ms, batch throughput 2749 → 3241 fits/s.

The 1.7.0 → 1.8.0 throughput difference is within run-to-run noise on the loaded box: the new code paths only run for
fits that have beacons or projected bursts.

Cold-start gains since 1.5.0 (a cold rifter calc went from ~5.3 ms to ~2.7 ms wall when measured directly; `meta`
takes 1.5 ms, against 1.1 ms for a bare exec of the binary): the derived cache and the dataset (for the cache-key hash) are now mmapped instead of
read; lazy tables are zero-copy (ids, offsets and the dense index are read from the mapping, and record cells are
allocated in 64-record chunks on first touch); and release builds are stripped with fat LTO and `panic = "abort"` (2.2 MB binary instead of 29 MB). The
`profiling` cargo profile keeps debuginfo for callgrind.

The box is shared and loaded, so wall times are noisy. The instruction counts below are the stable measure.
The H cold start assumes the derived cache (`dataset.hcache`) has been warmed; `bench.yaml`'s build step runs
`meta` once to write it. Without the cache, cold start is about 120 ms.

## WebAssembly build and evaluator dry run

- `wasm/` + `web/` (commit da98a4d): `node web/test-node.mjs` gives 326/326 cases and 21051/21051 values vs Pyfa,
  with output byte-identical to the native CLI on all 326. Dataset load takes 0.55 s, then 1.8 ms per calc (Node 20).
- `tools/evaluate.py --dry-run --runs 1 --quick --only H` (bench 33db85a, 07:40 CST, load 7.7) on da98a4d: gate
  passed (326/326). Maintainability 0.90 (34 tests passing, README/DESIGN/LICENSE present, LGPL-3.0-or-later),
  features 1.00 (EFT export 326/326, eft_parse 20/20, RPC / search / type 1.00), portability 1.00 (code). Perf in
  that run: 0.382 ms/calc, 2345 fits/s, cold 4.8 ms.

## Extra self-checks (beyond the bench corpus)

These use a local, uncommitted copy of the GPL Pyfa oracle as a black box. Requests for A's EFT-based specs were
built with A's `eft` importer.

| set | result |
|---|---|
| A's 23 newest cases (sustain_*, ecm_*, pfighter_*), all bench metrics + `stank.*` + `jam_chance` | **23/23 cases, 1429/1429 values** |
| all 127 EFT fits in A's tests/fits, bench metrics | 123/127. The 4 misses are exactly the bench's `known_divergences.json` entries |
| bench corpus, Pyfa `sustainableTank` + capUsed + capRecharge | 248/249. The one miss is esf_items_7 (structure module on a ship, a known divergence) |
| **module sweep**: every published module, local (overheated/active) and projected, oracle-comparable types | **5014/5019 cases, 302469/302474 values**. All 5 misses are SDE warp-status modifiers that Pyfa's handlers omit (see below) |
| **implant / booster sweep**: every implant and booster on Hyperion and Cerberus | **2230/2230 cases, 168365/168365** |
| boosters with every side effect selected, every published drone (local, active) | **170/170, 13680/13680** |
| every published fighter on a Nidhoggur (default abilities) | **94/94, 5224/5224** |
| **charge sweep**: every published charge in a matching launcher / turret / module | **1035/1035 cases** (61507 + 1943 values) |
| projected drones and fighters (every published type, Hyperion and Rifter targets, 5 km) | **480/480, 25908/25908** |
| **hull sweep**: every published ship, empty (T3D in a mode) | 421/423. The 2 misses are Pyfa eve.db agility data (below) |
| beacons, bursts, scripted bubbles, TD drones, RTC, special modules and states (`cases_v18`) | 301/301 |
| every bench case at skill level 0 and 3 | 626/652. All 26 misses are bench `known_divergences` |
| every bench case with modules reversed, all overheated, and both | 771/786. All 15 misses are known divergences |
| every T3D mode and T3C subsystem | 66/66 |
| every mutaplasmid at its high and low roll | 815/834. The 19 misses are low rolls clamped to Pyfa eve.db ranges (e.g. 0.938 vs SDE 0.9375) |
| bench `pending-1.9.0.md` cases (breacher pods, Tengu overheat order, neut/nos vs MWD, EWAR drones) | 6/6 |

Types that are missing from Pyfa's eve.db (newer than Pyfa's data) can't be compared and are left out of these sweeps.

Bugs these sweeps found, all fixed: Pyfa evaluation order (pre-assign, additions, one unpenalized product, penalized
chains, post-assign) for capsim `int()` truncation; capital MJFG; superweapon, slash, cone-DoT and HOG speed / warp
handlers; projected mutadaptive RR at full spool; module state fallback (`isValidState`); the damage cycle of web
drones (Orbweaver: `speed`, not the web `duration`); breacher pods (`pure` damage, `kind: "breacher"`, only the strongest pod
in fit totals); attribute caps that apply only to calculated values (a mutated DC with hull resonance above 1); and
overheat effects that read `overload*` attributes in fit order (pending-1.9.0 Tengu case).

`cargo test --release` runs `tests/pyfa_parity.rs`, which covers all 326 bench cases and 21 051 Pyfa-expected values
grouped by family, plus the pending-1.9.0 cases, and `tests/unit.rs` (contract and helper behaviour).

### Known gaps (left on the SDE behaviour)

- **Warp-status modifiers that Pyfa omits**: Networked Sensor Array (already a bench `known_divergence`), Integrated
  Sensor Array 90475, Cynosural Field Generators 21096 / 28646 / 52694. The SDE gives them warpScrambleStatus
  modifiers that Pyfa's handlers do not apply. H applies the SDE.
- **Pyfa eve.db data drift**: Paladin and Golem base agility is 0.858 / 0.963 in Pyfa's eve.db against 0.0858 /
  0.0963 in the dataset, so align time is 10×. The T3C maxSubSystems difference is the same kind of issue and the bench
  already accepts it.
- **Breacher damage** follows eve-dogma-rs's additive contract: a `pure` key (present only when non-zero),
  `kind: "breacher"`, and only the strongest pod in the totals. With no target HP, it is the max damage per tick, at
  one tick per second. The bench contract has no breacher case yet; it is staged for 1.9.0.
- **Mutaplasmid ranges**: Pyfa's eve.db rounds some ranges (e.g. 0.938 against the SDE's 0.9375), so low rolls are
  clamped differently. H uses the dataset.

## Parity with Variant A

Every output leaf was compared, abs 1e-6 / rel 1e-9, against A's working-tree build of ~04:55 CST.

- Bench 1.4.0 corpus: **289/289 identical**.
- The 23 newest cases: **23/23 identical**.

## Instruction counts (callgrind, 249-fit batch incl. startup)

| step | Ir |
|---|---|
| per-access `World::get` (before views; estimate from the earlier ~7 M/calc profile) | ~1.75 G |
| views + mimalloc | 1.27 G |
| prebuilt skill slots, query_mut security pass | 1.07 G |
| capsim ranks, batch skill spawn, in-place tidy | 1.01 G |
| component queries for incoming effects | 0.997 G |
| + ECM/fighter abilities/EFT tools (same 249 fits) | 1.006 G |
| capsim re-arms the current event in place (peek_mut) | 0.939 G |
| skills whose modifiers cannot reach the request are not spawned | 0.755 G |
| capsim packed u128 event keys | 0.717 G |
| lazily decoded type table with dense id index (also cold start) | 0.676 G |

Every perf step above was checked byte-identical on all outputs: 749–755 fits (bench and local cases, plus each at
skill levels 0 and 3). The capsim fast path also has a randomised equivalence test against the generic simulator
(`cargo test`).

## Bomb launchers (local Pyfa oracle)

- 6 local cases (void bomb, 2x focused void, lockbreaker, lockbreaker + void, interdiction sphere, 2x surgical
  probe): **6/6 cases, 375/375 values** match Pyfa. Reference A doesn't model these yet: 0/6 identical to A (cap,
  sustained tank, jam chance, velocity).

## Helpers (contract v1.4.1)

- `eft` import of all 129 fits in A's tests/fits: **129/129 identical** FitRequests to A.
- `eft_export` is checked against Pyfa's own EFT exporter (`service/port/eft.py`, run as a black-box test tool)
  on 443 FitRequests (bench cases + local cases + a stress fit). **428/429 byte-identical** (14 fits don't
  build on Pyfa's older eve.db). The one diff: a fit with two boosters in the same booster slot, where Pyfa keeps
  only one.
  The format: two blank lines between sections, one between racks; lowercase `/offline`; drones in Pyfa's
  market-group order; fighters by group then name; implants and boosters by slot; cargo by
  (category, group, name); mutation lines list every rolled attribute; no trailing newline; no T3D mode line
  (Pyfa's exporter writes none).
- `eft_export` → `eft_parse` → `eft_export` on the same 443 fits: 443/443 identical text. Import follows Pyfa's
  section rule (an all-drone section is the drone bay; a drone in a mixed section is cargo).
- `search`: categories ship/module/charge/drone/fighter/implant/booster/subsystem/skill, published only;
  exact > prefix > substring over English and Chinese names; ties by type id ascending; default limit 20.
- `type` output is identical to A's.
