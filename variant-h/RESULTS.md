# Variant H: results

## Official bench: `python3 bench.py --only H --quick`

Run at 04:45 CST with bench 1.3.0+8e02018 (249 cases, 13 812 Pyfa-expected values). The full scorecard is in
[results/scorecard.md](results/scorecard.md).

| | cases | values |
|---|---|---|
| vs Pyfa (bench) | **249/249** | **13812/13812 (100.00 %)** |

Every group (application, capacitor, defense, fitting, navigation, offense, tank, targeting) is at 100 %.

| perf (same harness) | H | A (bench results/A) |
|---|---|---|
| latency, one fit (exct_rifter), ms/calc | **0.417** | ~1.3–1.43 |
| batch throughput, fits/s | **1542** | 460 (corpus x5) |
| startup + one calc, ms | **19.1** | 150.1 |
| one process per case, median ms | **20.5** | 216.5 |
| deterministic | yes | yes |

The H cold start assumes the derived cache (`dataset.hcache`) has been warmed. `bench.yaml`'s build step runs
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

Every output leaf was compared, abs 1e-6 / rel 1e-9, against A's working-tree build of 04:4x CST.

- Bench corpus: **249/249 identical**.
- The 23 newest cases: **23/23 identical**.

## Instruction counts (callgrind, 249-fit batch incl. startup)

| step | Ir |
|---|---|
| per-access `World::get` (before views; estimate from the earlier ~7 M/calc profile) | ~1.75 G |
| views + mimalloc | 1.27 G |
| prebuilt skill slots, query_mut security pass | 1.07 G |
| capsim ranks, batch skill spawn, in-place tidy | 1.01 G |
| component queries for incoming effects | **0.997 G** |
