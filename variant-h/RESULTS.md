# Variant H: results

## Official bench: `python3 bench.py --only H --quick`

Run at 04:57 CST with bench 1.4.0 (65fab29: 289 cases, 18 591 Pyfa-expected values, including sustained tank, jam
chance, warp scramble and drone/fighter application). The full scorecard is in [results/scorecard.md](results/scorecard.md).

| | cases | values |
|---|---|---|
| vs Pyfa (bench 1.4.0) | **289/289** | **18591/18591 (100.00 %)** |
| vs Pyfa (bench 1.3.0, 249 cases, 04:45 CST) | 249/249 | 13812/13812 |

Every group is at 100 %.

| perf (same harness) | H @ 1.4.0 (box load ~15) | H @ 1.3.0 (04:45) | A (bench results/A) |
|---|---|---|---|
| latency, one fit (exct_rifter), ms/calc | 0.801 | **0.417** | ~1.3–1.43 |
| batch throughput, fits/s | 1251 | **1542** | 460 (corpus x5) |
| startup + one calc, ms | 19.9 | 19.1 | 150.1 |
| one process per case, median ms | 19.4 | 20.5 | 216.5 |
| deterministic | yes | yes | yes |

The 1.4.0 run was taken at load average ~15 on the shared box. Instruction counts (callgrind, below) moved only
+0.9 % between the two runs, so the latency difference is load, not code.

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

## Helpers vs A

- `eft` import of all 129 fits in A's tests/fits: **129/129 identical** FitRequests.
- H `eft_export` → `eft_parse` round trip: 129/129.
- Export text of the 289 bench cases matches A's apart from blank lines (A adds an extra blank line before boosters
  when there are no implants) and the T3D mode line. H exports the mode; A drops it.
- `type` output is identical to A's. `search` matches on 8 of 9 sample queries; for Chinese queries the ordering of
  equal-rank hits differs slightly.
