# G3 design: vectorised grid engine with a per-fit cache

## Layout

| file | role |
|---|---|
| `g3/cli.py` | `graph`, `graph-batch` (JSONL) and `serve-stdio` (RPC `graph` plus G's methods); everything else goes to G's CLI |
| `g3/graph.py` | `Engine`: request validation (schema, graph, axis, y, finite x), dispatch, JSON shaping (`null` for non-finite), per-request error isolation (`INTERNAL`) |
| `g3/ctx.py` | `Ctx` wraps one variant-G calc of a FitRequest with typed accessors (modified attributes, effects, groups, active modules, drones/fighters, cycle times, ranges, missile flight data) plus a `memo` dict for derived tables. `FitCache` holds the contexts |
| `g3/common.py` | vectorised primitives: range factors, `unerr`, Pyfa-style stacking penalty over point arrays, shield/cap regen curves |
| `g3/simple.py` | lock_time, warp_time, mobility, shield_regen, capacitor, ewar, remote_reps |
| `g3/damage.py` | damage graph: dealer tables, target model (profile or fit), tackle (webs/painters/scram), application kernels per weapon kind, time schedules |
| `g3/appprof.py` | application_profile: charge enumeration, quality filter, transition search, vectorised kernels |

## Grid engine

Every graph is a function f(x; fit, target, params) evaluated over the whole `x.values` array at once:

1. **Fit stage.** One G calc per distinct FitRequest yields modified attributes. `Ctx` turns them into
   struct-of-arrays tables, one row per damage dealer, EWAR source, RR module or charge candidate.
2. **Projection stage.** The target's speed and signature are functions of distance because of webs, painters and
   scram-vs-MWD. They are computed as arrays with the stacking penalty applied column-wise.
3. **Kernel stage.** Branch-free NumPy expressions over [rows × points]. Examples: turret chance-to-hit with the
   wrecking curve, missile application min(1, S/E, (S·Ve/(E·v))^drf), fighter/drone/bomb variants, the
   lock-time `asinh` formula, the mobility exponential and the shield/cap regen curves. Branches become
   `np.where`, and errors are suppressed and then mapped to `null` at output.
4. **Output.** `tolist()` once, then non-finite values become `null`.

Sequential parts stay scalar. They are computed once and reused:
- **Time axis.** The damage and RR schedules are built once per (fit, settings) up to the largest requested time.
  Each x is then a lookup into that table.
- **Capacitor.** A capacitor simulation with a change log, reused across x values.
- **Subwarp.** Warp time uses a second, modified-request context and is cached like any other fit.

## Per-fit cache

`FitCache` is an LRU cache with 64 entries. The key is a variant tag plus the canonical JSON of the FitRequest
(sorted keys, no whitespace). Target fits, subwarp variants and the launcher "temporary charge" variant are cached
the same way under their own tags. Derived tables (time schedules, projected tables, application-profile transitions)
live in `Ctx.memo`, keyed by every parameter that affects them. A different axis, target or sample set therefore
reuses the fit without risking stale data. The dataset is fixed per process, which keeps the key deterministic.
`--no-cache` builds a fresh context for each request, and the bench checks that its output matches the cached run
on all 111 cases.

## Pyfa mapping (behaviour)

| graph | Pyfa source of semantics | notes |
|---|---|---|
| damage | `fitDamageStats` | distance/time/speed/sig axes; turrets, missiles, drones (incl. mobile drone mode and control range), fighters (abilities), smartbombs, bombs, breachers, Vorton/disintegrator spool; target resists/profile or fit; projected webs/TPs |
| application_profile | `fitApplicationProfile` | dominant weapon kind (turrets win ties), one group per module type, valid charges = published types in chargeGroup1–4 that fit capacity and size, `ammo_quality` filter (all / t1 / navy), best charge sampled on getSampleStep grid with 10 m bisection, launcher priority tie-break and range-out sentinel, projected (speed, sig) table with linear interpolation |
| mobility | `fitMobility` | speed/distance/momentum/bump over time |
| lock_time | `fitLockTime` | vs target signature |
| warp_time | `fitWarpTime` | subwarp speed from a modified fit |
| shield_regen | `fitShieldRegen` | amount/percent axes |
| capacitor | `fitCapacitor` | regen curve and simulated cap over time |
| ewar | `fitEwarStats` | webs, painters, ECM, damps, TDs, GDs, neuts vs distance |
| remote_reps | `fitRemoteReps` | per-time schedule and distance falloff |

Application profile specifics:
- Charge stats come from **base** charge attributes scaled by the ratio modified/base of the loaded charge. These
  multipliers capture skills, ship bonuses and modules. For a launcher with no charge loaded, the first valid
  charge is loaded in a cached variant fit.
- Transitions are found by a vectorised evaluation of the full [charges × grid] matrix; only bisection midpoints are
  evaluated one at a time. Each x then uses the charge of the last transition ≤ x.
- `<y>_charge_type_id` series are produced. They are informational: equal-stat faction charges tie, and Pyfa
  picks by set order.
- Meta groups are not in G's column cache. On first use they are read from the dataset JSON and stored as a sidecar
  `g3-metagroups-<sha256>.json` next to G's pickle.

## Numerical notes

Kernels use the same IEEE operations as the scalar formulas, in the same order (`**` → C `pow`, `floor`, `sin`), so
results match Pyfa well within the 1e-4 relative tolerance. `unerr` reproduces G's `float_unerr` exactly, branch-free.

## Time-axis evaluation (vectorised)

The schedule of each dealer is still built once by a scalar walk over the cycles, since reload and spool are
sequential. That walk memoises identical (volleys, duration) segments and their comparison keys, and skips spool
work for modules that don't spool. Everything after it is array code, cached per time cache in `Ctx.memo`:

- `_Entries` holds one dealer column: the change times (pre-`unerr`ed), the entry 4-vectors (dps, volley, or
  running damage sums) and the resisted vectors per resist profile.
  - Running sums use the same left-to-right float additions as chaining `Dmg.plus`.
  - They are built in one pass instead of materialising one `Dmg` per entry.
- A query is one `searchsorted` per dealer on the shared `unerr(t)`, a row gather of the resisted table, and
  `sum(axis=1) * application`.
- Breacher ticks become [entries × ticks × L] tensors (absolute, relative, present).
  - For running sums they are filled incrementally: each increment sets its slot for all later entries.
  - Per request, each dealer's ticks are evaluated as one [points × ticks] block. Python `min`/`max` semantics are
    kept with `where(y < x, y, x)`, which matters for NaN when hp is infinite.
  - Ticks merge into a global [points × ticks] matrix with a running max.
  - Tick columns are summed in first-visit order via `add.accumulate`, the same sequential order as the scalar loop.
- Application-profile bisections for all charge changes run as one batch per bisection step. This is valid because
  the current charge at grid point j is always the grid best at j−1.

Output identity was the rule for this work: every change is checked byte-for-byte against f7aa4cb on 3285 stress
requests, plus the corpus and the 1.8.0 stats gate.

## Empty `x.values`

CONTRACT-GRAPHS 0.1 doesn't list empty input as an error. It says each series has "the same length as x", and
Pyfa evaluates per point (`getPoint`), so no points means nothing to evaluate. Eve ruled (2026-10-03) that an empty
`x.values` succeeds with empty series:
- `{"graph", "x_axis", "x": [], "series": {<y>: [] ...}}`. Application profile also returns empty
  `<y>_charge_type_id` arrays.
- Exit 0 for `graph`, and a `result` over RPC.

f7aa4cb..3357471 returned `INTERNAL` (an AxisError) on the time-axis and time-parameter damage paths. That is fixed.
`tests/test_empty_x.py` covers every graph and axis of the corpus with all valid y series, in-process (cached and
`--no-cache`), through `graph-batch`, `graph` and `serve-stdio`. Every non-empty request is unchanged: the identity
check still passes all 3285 stress requests.

## Contract 0.2

- Validation follows the 0.2 precedence: structural `BAD_REQUEST` (graph / fit / x / x.values / y, empty y), then
  `UNKNOWN_GRAPH`, `BAD_AXIS` (axis, y, and (x, y) pairs such as `ecm_burst` `tgt_dps` × `tgt_lock_time_s`), the enums
  (`target.resist_mode`, `settings.mobile_drone_mode`, `params.ammo_quality` → `BAD_REQUEST`), and last
  `UNKNOWN_TYPE` (source fit first, then `target.fit` — only for damage, application_profile, ewar and remote_reps;
  other graphs ignore `target`, bench 0397d95).
- `target.fit` is built once per request through the fit cache under its own key (the canonical target
  FitRequest), so source and target contexts never alias. ewar: per-y resistance attribute
  (`clamp(1 − attr, 0, 1)`, 0 / missing → 1), explicit `params.resist` wins, `disallowOffensiveModifiers` zeroes all
  but `neut_gj_s`. remote_reps: × `remoteRepairImpedance`, 0 with `disallowAssistance`.
- Clamps: ewar `resist` 0..1 and the `time_s` param 0..2500 (damage, remote_reps). These were 0.1 bugs too.
- Damage `tgt_speed_pct` / `tgt_sig_pct` are normalised to m/s and m and then follow the absolute-axis paths.
  An infinite-signature target gives `null` for `tgt_sig_pct`.
- `ecm_burst` (`simple.ecm_burst`): damp multiplier `calculateMultiplier` over modules/drones (range ignored), and
  stats-panel EHP / weapon dps / drone + fighter dps from variant G's `FitStats`.
- Identity: the 3285 stress requests (all 0.1-valid) are still byte-identical to f7aa4cb.

## Known limits / next steps

- The cold path is dominated by variant G's fit calculation, which is not modified on this branch.
- The mobile-drone web speed model (drones chasing the target) still loops over points in Python, but costs well
  under 1 ms at 500 points.
- The extra 2-D `x2` heat-map axis from the plan is not implemented. The scorer ignores it.
