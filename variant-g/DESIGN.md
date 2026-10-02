# Variant G — design

**Goal:** the readability + batch-throughput baseline of the lab: a Python implementation where the dogma
pipeline is expressed as NumPy array operations over *many fits at once*, with results identical to the
reference engine (variant A) and therefore to Pyfa. Correctness first, then speed.

## Pipeline

```
requests ──parse──► Batch.add_fit (per fit, Python: items + base-value overrides)
                         │
                         ▼
             register_generic  (vectorised, whole batch)          dataset cache (NumPy CSR tables)
             ─ gather modifier-template rows of every item  ◄──── tm_*: one row per (type, effect, modifier)
             ─ filter: state ⨯ effect category, structures,       ta_key/t_attr_*: type attributes
               booster side effects, fighter abilities             attr_*: default/stackable/high-is-good/caps
             ─ resolve targets: direct (self/ship/char/other)
               + equi-join on (fit, class, group|skill) table
             register_python  (per fit, small): propulsion, MJD, slot/hardpoint modifiers,
               projected ewar (webs, paints, damps, ECM, TD/GD/remote tracking computers onto the
               target's gunnery modules / missile charges) / reps / neuts, fighters, fleet + burst buffs, RAH
                         │
                         ▼
             evaluate  (vectorised, whole batch)
             ─ nodes = unique (item<<14 | attr) keys; base = override > skill level > type attr > default
             ─ levelise: longest path in the dependency graph (sources, caps) via np.maximum.at relaxation
             ─ per level: source values → per-stage aggregation (assign / add / sub / multiply) with
               stacking penalties ranked by lexsort, then min/max caps and cpu/power rounding
                         │
                         ▼
             stats (per fit, Python): resources, offense, defense/tank, capsim, navigation, targeting, validation
```

### Data layout

* **Dataset cache** (`dataset.py`): the gz JSON is flattened once into column arrays and pickled
  (`~/.cache/eve-dogma-g/<sha256(gz)[:16]>-v8.pkl`). Per-type Python tables that only some requests
  need (effects lists, required skills, effect infos, names, search/EFT facts) are stored as marshal blobs and
  decoded per entry on first access (`LazySeq` / `LazyMap` / lazy whole), so a cold `calc` decodes only what it touches. Per type: attributes as CSR (`t_attr_ptr/ids/vals`,
  type-level mass/capacity/volume/radius merged in like the reference), effects, required skills, inferred slot,
  and a **modifier-template table** `tm` (CSR by type): one row per `(effect, modifier)` with the effect
  category/default flag, `func, domain, modified, modifying, op, extra`, plus marker rows for the hand-written
  "special" effects. `ta_key = type<<14 | attr` (sorted) gives vectorised base lookups.
* **Batch** (`engine.py`): item columns over all fits (`fit, type, kind, location, owned, state, parent, charge`).
  All published skills of every fit are created as a NumPy block (512 per fit) rather than Python objects.
  Skills, implants, boosters, modes and beacons are *pruned* items: only attributes that are referenced
  (modifier sources/targets, overrides) become nodes — a skill's level is a node only where a modifier reads it.
  For ships, modules, charges, drones, fighters, projected items and the character only attributes that the
  evaluation changes (modifier targets, min/max-capped, cpu/power rounding) are nodes; `Values` serves every
  other attribute from the shared type table (`Evaluated.full`).
* **Modifiers**: columns `tgt, attr, op, pen, kind, a, b, c, const, factor, mul, o1, o2`. `kind` is the
  source expression: `ATTR` (value of node `a`), `CONST`, `PROP` (AB/MWD speed factor from nodes a,b and ship
  mass c), `PROJ` (projected value scaled by range factor and target resistance node c). `pen` = the
  attribute is non-stackable and the source category is not exempt (ship, charge, skill, implant, subsystem,
  structure). `o1, o2` = registration order (source item, row in that item's effect order; later passes by
  insertion).

### Target resolution as a join

Location-type modifiers become an equi-join: every item is entered into a sorted key table under
`(fit, class, x)` with classes *ship-location*, *char-location*, *group@ship*, *group@char*,
*required-skill@ship-location*, *required-skill@owned*, *required-skill@char-or-owned (not skills)*. A modifier
row computes its key from its (func, domain) and `extra` (group or skill; the dataset's patch convention
"skill 0 = the effect's owner" is resolved first), and `searchsorted` left/right + `repeat` expand it into
(row, target item) pairs for the whole batch in one go.

### Levelised evaluation

The value of a node depends on its modifiers' source nodes and on its min/max cap attribute nodes. A node's
*level* is the longest path from an unmodified node; it is computed by iterated `np.maximum.at` relaxation
(converges in depth+1 iterations, ~6–10 for real fits; capped at 64 to survive cycles — a node read inside a
cycle sees its base value, like the reference's cycle guard). Then levels are evaluated in order; within a
level everything is vectorised over all nodes of all fits:

1. source values gathered from the value array (`ATTR`), or computed (`CONST/PROP/PROJ`);
2. operator stages in CCP order (PreAssign, PreMul, PreDiv, ModAdd, ModSub, PostMul, PostDiv, PostPercent,
   PostAssign). Assign: max (min if `high_is_good` is false) via `np.maximum.at`. Add/Sub: `np.add.at`.
   Multiplicative: per-row factor; penalised rows are ranked inside their (node, stage, sign) group with one
   `lexsort` by |factor−1| and get `1 + (f−1)·exp(−rank²/7.1289)`;
3. min/max caps, then the 2-decimal rounding of cpu/power(Output).

**Exact float parity.** Floating-point products depend on order, and a 1-ulp difference can flip a
`trunc()` (e.g. a 1733.99999 ms vs 1734 ms cycle in the capacitor simulation). The evaluator therefore applies
rows in the reference's registration order: rows are sorted by `(o1, o2)`, unpenalised factors are applied first,
then the penalised positive list, then the negative list (each sorted, stable), with the in-order unbuffered
`ufunc.at` (`np.multiply.at(v, idx, f)`) so `v` is multiplied sequentially exactly like `val *= m`. With
this, all 326 corpus responses (bench 1.8.0) are identical to the reference on every leaf (`tests/compare_ref.py`).

### Evaluation-dependent effects

Some registrations need evaluated values; they are separate passes over *subsets* of the batch
(`evaluate(batch, fit_mask)`):

* **Local command bursts / fleet boosters**: `warfareBuffNID` (possibly PostAssigned by the charge) is read
  after a first evaluation of the fits that have active modules carrying it; per buff id the strongest |value|
  among the fit's own bursts and its `fleet.booster_fits` wins, explicit `fleet.buffs` override, all applied
  in buff-id order.
* **Booster fits and projected fits** are themselves computed first as one extra batch (recursively, with
  `booster_fits` / `projected` cleared like the reference). Booster fits yield constant buff offers; projected
  fits yield "frozen" items (active modules/drones with their evaluated attributes as base values).
* **Reactive Armor Hardener**: per RAH (sequential like the reference, one evaluation pass per RAH index for
  all fits that have that many), Pyfa's adaptation loop in Python, then constant PostAssign/PreMul rows.

### Engine-side effects (no modifierInfo in the SDE)

Pyfa implements some effects in Python handlers rather than SDE modifiers; G registers them in
`_register_python` as plain rows (same order and penalty category as the reference):
local specials (superweapon/lance speed + warp status, EHE, entosis, MJFG, WDFG, Breach Control), projected
remote reps / cap / neuts / ECM (`proj_special`), tracking/guidance disruptors, remote tracking computers, TD
drones, AoE burst projectors (full strength, no range factor; neut burst = drain, ECM burst = jam source),
the Standup weapon disruptor (range factor), incursion system effects (`OffensiveDefensiveReduction`, unpenalised)
and abyssal weather / AoE cloud beacons (their warfareBuff1/2 join the fleet-buff pool; buffs 79/90/93–99 also
hit drones requiring Drones; 90/93–96/98/99 unpenalised).

### Stats

`stats.py` is a straight per-fit Python port of the reference stats (which are Pyfa's formulas): resources,
weapons (turret/missile incl. Pyfa missile range, smartbomb, vorton, spool-up), drones, fighters (Pyfa default
abilities), EHP/resonances, local and incoming remote reps (diminishing returns), passive regen, capacitor
simulation, navigation, targeting, drones, validation, attribute dumps. Attribute reads go through per-item
dicts built lazily from the sorted node arrays.

**Capacitor simulation** (`capsim.py`): the event-heap algorithm of Pyfa's capSim (as in the reference).
Fast path for the common case (no injectors, no clips): every drain's event times are built as an exact running
sum (`t, t+d, (t+d)+d, …` like the heap, not `k·d`), each window of events is sorted with NumPy (same tie order
as the heap: time, duration, cap need), the decay factors `exp(−Δt/τ)` are precomputed with libm `math.exp`
(NumPy's SIMD exp can differ in the last ulp), and windows grow geometrically. Only the nonlinear recurrence
remains a tight Python loop (recharge ≡ new time step, `t_last`/iteration count recovered from the loop position,
`y*y` like Rust `powi(2)`). Injector / reload cases use the general heap loop; `tests/run_tests.py` fuzzes the fast
path against the heap loop on 300 random drain sets (bit-identical).

## Trade-offs

* **Batch vs latency.** Per-call overhead (NumPy call setup, ~100 small array ops per evaluation pass, Python
  stats) dominates a single fit; batching amortises the vectorised part. In-process on the 297-case (1.6.0) corpus
  (≈0.85 s, ~350 fits/s): capacitor simulation ≈ 25 % (sequential recurrence, ~410k events), vectorised dogma
  evaluation ≈ 27 %, per-fit Python stats ≈ 20 %, registration + item setup ≈ 15 %. 500 identical rifters:
  ≈ 2.0 ms/fit. Remaining targets: the capsim recurrence (inherently sequential; only a compiled kernel would
  help), and vectorising the per-fit stats (most are gathers + segment sums).
* **Cold start** ≈ 110–120 ms: Python + `import numpy` (~80–90 ms on this box; OpenBLAS/OMP threads are pinned
  to 1 before the import), cache unpickle ~10 ms, first calc ~7 ms. Worse than compiled variants by design.
* **Readability.** The engine core is ~1100 lines of plain Python/NumPy; the dogma semantics (operator order,
  penalty, caps, targeting) are each one visible block of array code instead of a recursive lazy evaluator.
* **Memory.** Eager evaluation computes every materialised node (≈6k per fit), not only requested ones. Fine
  for thousands of fits per batch (chunk size 256 by default).
* **Everything evaluated, not lazily**: differs from the reference only in the cycle guard (nodes in a cycle
  read base values); no real fit in the corpus has a cycle.

## Contract notes / ambiguities (decisions taken)

* The reference engine is the semantic spec where the contract is silent (e.g. registration order, booster/
  projected fit handling, warnings' text). Variant G reproduces its output byte-for-byte except `meta.engine`.
* EFT: `eft [FILE] [--calc] [--skills N]` and RPC `eft_parse` / `eft_export` (`evedogma_g/eft.py`). Import is
  identical to the reference on the test texts; export is Pyfa's `exportEft` byte for byte (contract 1.4.1 ruling
  4: racks with `[Empty X slot]` fillers from the evaluated slot counts, Pyfa drone/fighter/implant/booster/cargo
  order, ` /offline`, mutation block with Pyfa float formatting; no T3D mode line): 326/326 vs Pyfa (bench 1.8.0)
  (`expected_extra/eft_export.jsonl`, T3C maxSubSystems data divergence accepted like the bench), identical to A.
* `serve-stdio` methods: `calc, eft_parse, eft_export, search, type, meta`. `search` follows the interim spec
  (published ship/module/charge/drone/fighter/implant/booster/subsystem/skill, English or Chinese name,
  exact > prefix > substring, typeID ties, limit 20, optional `kinds`).
* `calc` with an error prints the `{"error":…}` JSON on stdout and exits 2 (contract 1.4.1); `batch` exits 0.
* `options.sources` is accepted and ignored (the reference also does not emit sources).
* Batch mode isolates errors per line (a bad line yields an error object, the others are unaffected).
* The dataset cache is derived only from the official dataset file and keyed by its sha256 (allowed by the brief).

## Provenance

Semantics were taken from the reference engine's source (eve-dogma-rs, LGPL-3.0-or-later) and verified against
its binary and the Pyfa oracle (black box). No code was copied; the implementation is an independent
NumPy re-expression. RAH adaptation and the capacitor simulation follow Pyfa eos's algorithms (LGPL) as the
reference does. Variant G is therefore licensed LGPL-3.0-or-later.
