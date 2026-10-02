# variant-e design: Pyfa, transpiled

**Goal:** match Pyfa bit-for-bit by *not re-deriving* dogma. The approach is to translate Pyfa's own source,
keep its evaluation order, and copy its quirks. That includes the double-applied skill multipliers in some handlers,
the stacking-penalty sort order, `round(x, 2)` on cpu/power, `floatUnerr`, the cap simulator's heap order, and the
injector hacks.

License: Pyfa is GPL-3.0-or-later, so this variant is too. The reference engine A (LGPL-3.0) was used only to
copy the output JSON shape; LGPL code can be combined into a GPL work.

## Pipeline

```
Pyfa eos/effects.py ──(tools/pyfa2rs.py, Python ast)──▶ src/generated/effects.rs   (2,359 of 2,402 handlers)
dataset-3569502.json.gz ──(name → id resolution at transpile time)──┘
                                         hand ports: src/eos/custom.rs (RAH, remote reps/neuts/nos/cap transfer, …)
FitRequest ─▶ eos::fit::build ─▶ calculate (early/normal/late) ─▶ projected / command fits ─▶ eos::stats ─▶ FitStats
```

### Transpiler (`tools/pyfa2rs.py`)
- Parses every `class EffectNNNN(BaseEffect)`, reads `runTime`, `type`, `grouped`, `activeByDefault`, `prefix`, and
  `hasCharges`, and translates `handler()` statement by statement in its original order.
- Attribute, skill, group and type **names are resolved to ids at transpile time** from the dataset. Pyfa-only
  attribute names (fit "extra attributes" such as `shieldRepair` and `maxTargetsLockedFromSkills`) get synthetic ids
  of 1,000,000 and up. At runtime there are no string lookups for attributes inside handlers.
- Compile-time strings and tuples are folded, and `for x in (literal tuple)` loops are unrolled. Runtime numbers and
  booleans become `let mut` locals. String values chosen at runtime (`fit.scanType`) become `strsel` switch arms.
- `fit.modules.filteredItemBoost(lambda …, attr, value, **kw)` becomes
  `cx.filtered(L::Modules, charge, &|cx, m| <filter>, Op::Boost, attr, value, O{skill, stack, group, post, kw})`.
- `Fit.__runCommandBoosts` (the warfare-buff table) is transpiled into `command_buff(id, value)`.
- 26 handlers cannot be expressed this way: Python imports, `fit._armorRr` lists, `fit.addDrain`, and the RAH
  simulation. 13 of these are ported by hand in `custom.rs`. ECM strength is gathered there too and used for `targeting.jam_chance_percent` (Pyfa `jamChance`).

### Calculation core (`src/eos`)
| Pyfa | here |
|---|---|
| `ModifiedAttributeDict` (preAssign / increase / multiply / penalized groups / postIncrease / force, placeholders, intermediary, caps) | `mad.rs` `Mad`/`Entry`, lazy calculation with a `Cell` cache, same penalty math and sort |
| `fit.modules.filtered*`, `boostItemAttr(..., skill=, stackingPenalties=, penaltyGroup=, **kwargs resistance)` | `cx.rs` `Fit::op`, `Fit::filtered`, `Fit::resistance` |
| `Fit.calculateModifiedAttributes` item order: (char, ship), drones, fighters, boosters, implants, modules, then mode, projected drones and projected modules; command boosts after each runtime | `fit.rs` `calc_rt` |
| `Module/Drone/Fighter.calculateModifiedAttributes` (charge effects first, overheat, state rules, `grouped`, amountActive repetitions) | `fit.rs` `calc_module` / `calc_drone` / `calc_fighter` |
| command fits (`CalcType.COMMAND`, gang effects add bonuses to the target, strongest |value| per buff wins) | `calculate_command` + `api.rs` |
| projected fits (`__runProjectionEffects` after each runtime of the source fit's own calculation) | `api.rs` + `Fit::project_from` (source items are mirrored with their *current* values each runtime, the same as Pyfa's live reads) |
| module/drone/fighter DPS, `getCycleParameters` (reload, forceReload, CycleSequence average), `numShots`, spool-up, fighter refuel and optimisation | `stats.rs` |
| `capSim.py` (including the CPython `heapq` sift order, stagger, injectors, optimisation by period repetition) | `capsim.rs` |
| `__getAppliedRr` diminishing returns, `addDrain` signature-resolution scaling | `stats.rs`, `custom.rs` |

### Trade-offs
- **Faithful over clean.** Handlers run in Pyfa's order with Pyfa's side effects, including handlers that read
  half-calculated values. That is what makes 100 % agreement possible, and it is also why there is no dependency
  graph or incremental recomputation.
- **Generated code size:** about 16 k lines of Rust, compiled with fat LTO. A release build takes about 80 s.
- **Per-fit state** uses a `Vec<Item>` arena with an `FxHashMap` of modifier entries per item. Every published skill
  is materialised as an item, which is Pyfa's model. That is the main per-fit cost (about 0.5 ms).
- **Also ported:** sustained tank (`calculateSustainableTank`), projected fighters with abilities, and drone/fighter range, velocity and signature fields.
- **EFT export** (`eft_export` RPC, `src/eft.rs`): port of Pyfa `exportEft` with all options on, after `Fit.fill()`
  (empty-slot lines from the modified slot counts), DRONE_ORDER by market group, mutation blocks with `floatUnerr`
  values in Python float repr. 295/295 byte-identical to Pyfa on the bench corpus.
- **Fleet buffs** (contract 1.4.2): explicit `fleet.buffs` override booster fits and the fit's own bursts per buff id
  (several entries with one id aggregate by min/max per the buff's aggregate mode); other ids keep Pyfa's
  strongest-|value| rule (`addCommandBonus`).
- **Not ported (not scored):** mining yield, EFT import, and the `search`/`type` RPC methods.

### Performance notes
- JSON output goes through a flat output tree (`src/jv.rs`: objects are vectors, keys sorted at serialisation), so
  the output is byte-identical to `serde_json::Value` without per-key BTreeMap nodes.
- Python `round(x, n)` (used by `floatUnerr`, cpu/power rounding, capSim) has an exact fast path: `round(x·10ⁿ)/10ⁿ`
  when x·10ⁿ is not within its rounding error of a .5 tie (Clinger fast path), else the decimal formatter. A test
  checks 32 M random cases bit-for-bit against format+parse.
- The parsed dataset is cached as bincode. The cache layout is `[len][Dataset][len][type index][type records]`: the
  type index (sorted ids, groups, offsets) is decoded eagerly, each `TypeInfo` lazily (`OnceLock` per type) straight
  from the cache buffer (no copy), and the name index is built only when EFT/T3D-mode lookups need it. One-shot
  `calc` also skips freeing the dataset. Cold start + calc ≈7–8 ms (was ~160 ms without a cache, ~14 ms eager).
- mimalloc is the global allocator. Skill type records are resolved once per dataset (`skill_pos`) and the per-fit
  item/skill tables are presized; each weapon's volley parameters and cycle time are evaluated once in the offense
  section; the capSim heap compare short-circuits on the event time (same ordering as Python tuple comparison).
- Hot paths use dense tables instead of hash maps: effect metadata (`MetaTable`, a `Vec<u16>` index into
  `effects::META`), attribute metadata for ids < 8192 (`AttrLite` in `mad.rs`), and static attribute-id arrays for
  resonances/cycle-time lookups. capSim uses an index-arena binary heap with the same `heapq` ordering, so event
  order (and therefore every float) is unchanged.
- Every perf change is checked by byte-comparing batch output for the whole corpus against a saved reference.
