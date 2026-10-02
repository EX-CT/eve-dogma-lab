# Variant H: results

## Official bench (`run.py`, the same harness as `bench.py`)

Bench **1.5.0** (contract 1.4.2: 295 cases, 18 978 Pyfa-expected values, including amount>1 projected fits with
tracking/guidance disruptors), run at 05:33 CST:

| | cases | values |
|---|---|---|
| vs Pyfa (bench 1.5.0) | **295/295** | **18978/18978 (100.00 %)** |
| vs Pyfa (bench 1.4.0, 04:57 CST) | 289/289 | 18591/18591 |
| EFT export vs Pyfa `exportEft` after `fill()` (`tools/check_eft_export.py`, informational) | **295/295** | |

| perf (same harness, box load ~13) | H @ 1.5.0 05:33 | H @ 1.4.0 04:57 | A (bench results/A) |
|---|---|---|---|
| latency, one fit (exct_rifter), ms/calc | **0.412** | 0.801 | 0.911 |
| batch throughput, fits/s | **2321** | 1251 | 912 |
| startup + one calc, ms | **11.1** | 19.9 | 119.3 |
| one process per case, median ms | **9.8** | 19.4 | 123.4 |
| deterministic | yes | yes | yes |

The box is shared and loaded, so wall times are noisy. The instruction counts below are the stable measure.
The H cold start assumes the derived cache (`dataset.hcache`) has been warmed; `bench.yaml`'s build step runs
`meta` once to write it. Without the cache, cold start is about 120 ms.

## Extra self-checks (beyond the bench corpus)

These use a local, uncommitted copy of the GPL Pyfa oracle as a black box. Requests for A's EFT-based specs were
built with A's `eft` importer.

| set | result |
|---|---|
| A's 23 newest cases (sustain_*, ecm_*, pfighter_*), all bench metrics + `stank.*` + `jam_chance` | **23/23 cases, 1429/1429 values** |
| all 127 EFT fits in A's tests/fits, bench metrics | 123/127. The 4 misses are exactly the bench's `known_divergences.json` entries |
| bench corpus, Pyfa `sustainableTank` + capUsed + capRecharge | 248/249. The one miss is esf_items_7 (structure module on a ship, a known divergence) |

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
