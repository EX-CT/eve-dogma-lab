# Variant I — design: dogma as salsa queries

## Idea
A fitting tool recomputes stats after every small edit (toggle a module, swap a charge, change one skill).
Variant I models dogma as a **demand-driven, memoised query graph** (salsa 0.28) so a persistent session only
re-executes queries whose inputs actually changed, while the CLI contract stays stateless (byte-identical
output for identical requests, whatever was computed before — checked by running the corpus forward+reverse
in one session vs. a fresh database per request: identical bytes).

## Inputs
* `ItemIn {slot, spec}` — one per item *identity* (`ItemKey`: Ship, Char, Skill(type), Mode, Module(i), Charge(i),
  Drone(i), Fighter(i), Implant(i), Booster(i), Beacon(i), Projected(i,k), ProjCharge(i,k)). A slot is allocated
  the first time a key is seen and reused afterwards; a spec is only re-set when it differs (manual diff, so no
  needless revision bumps).
* `FitIn {slots, order, ctx}` — canonical registration order of present items + fit context (fleet buffs,
  booster-fit buff offers, damage pattern, RAH option).

`spec.rs` turns a request into canonical `ItemSpec`s (pure); `session.rs` diffs them into the inputs.

## Derived queries (`engine.rs`)
| query | depends on | notes |
|---|---|---|
| `index(fit)` | every spec | structural info for target filters; backdates when only states change |
| `links(item)` | spec | charge/parent slots |
| `outgoing(fit, item)` | spec, index, links, parent state | modifiers an item emits (effects x targets) |
| `incoming(fit)` | all `outgoing` | per-target modifier maps (layer 0) |
| `item_mods(fit, item)` | `incoming` | one target's map — **backdates**, so attribute memos of untouched items stay green |
| `burst_mods(fit)` | layer-0 values | local command bursts + booster-fit offers + explicit buffs (strongest per id) |
| `plan(fit)` | specs, `burst_mods` | which modifier layers exist |
| `layer_mods(fit, L)` | values at L-1 | L=1 buffs (if any), then one layer per Reactive Armor Hardener |
| `attr_value(fit, (item, attr, L))` | `item_mods`, `layer_mods(1..L)`, source values | stacking penalties, caps, rounding |

Layers replace the reference engine's "evaluate, clear cache, add modifiers" sequence with pure queries: values a
later modifier depends on (warfare buff ids, RAH input resonances) are evaluated at an earlier layer, so there is
no salsa cycle. Fits without buffs/RAH evaluate only layer 0. Real attribute cycles (none in the corpus) fall back
to the base value via salsa `cycle_result`.

Projected fits and fleet booster fits run on **sub-sessions** (one persistent salsa DB per projected/booster slot),
so they are incremental too; their evaluated values are frozen into the parent's item specs.

Stats (`stats.rs`), capacitor simulation and EFT are shared semantics with variant A (eve-dogma-rs, LGPL) and run
on a read view (`session::Fit`) whose `get()` is a salsa query.

## Attribute evaluation: inline vs. per-attribute memo (since 84cd44a)
Most of the query graph stays as above. After `84cd44a`, an attribute value is not always its own salsa query:
* **Inline mode:** values are computed from the memoised modifier graph (`item_mods`, `layer_mods`) by a plain
  per-request cache (`engine::VCache`). It holds the value map, a cycle stack with salsa-style fallback to the base
  value, and caches for item mods and layers. This avoids interning, memo lookup and dependency recording for every
  `(item, attr, layer)`.
* **Memo mode:** this is the per-attribute `attr_value` salsa query. It pays off only when the previous request kept
  the same hull, which is what an edit looks like (only 59 of the 326 consecutive corpus fits share a hull).
* **Auto (default):** uses memo mode when the request keeps the previous request's hull and inline mode otherwise.
  `EVE_I_ATTR_MEMO=1` forces memo mode and `EVE_I_ATTR_MEMO=0` forces inline mode. Output is byte-identical in
  all three modes.
* The session's inline cache is reused while the salsa revision is unchanged
  (`salsa::plumbing::current_revision`). That makes an identical re-request almost free.

Other changes from the speed rounds:
* `item_mods(fit, slot)` returns the spec together with the mods, so an attribute records one dependency.
* A `roles` query means bursts and RAH never visit skill specs.
* Resolved skill specs are memoised together with the skill level list.
* `load` diffs specs against a shadow Vec and does no salsa reads.
* `validate` reads canFit and required skills in one pass, using bitmaps.
* `raw_cycle_ms` is memoised per item per view.
* Floats are rounded in a serde Formatter at serialisation, and batch output goes through a BufWriter.

## Trade-offs
* For a fit computed from scratch, I is still about 1.6x variant A in instructions (4.61G vs ~2.9G Ir on the
  corpus x5, A measured ~07:24). The salsa input diff, the modifier-graph queries and verifying ~400 skill memos per
  request cost more than A's plain cache.
* In a session, an edit (toggle one module on exct_rifter, `bench-edit`) recomputes in **~83–90 µs** at best
  (~140–190 µs on a loaded box). Before the speed rounds it was about 0.8 ms, and about 211 µs earlier this round.
* Memory grows with distinct keys. The session resets itself after 20,000 slots.

## Measurements (box, 1 thread, noisy shared load)
| | value |
|---|---|
| Callgrind Ir, corpus x5 batch | 6.54G (a3f4b8e era) → **4.61G** (9a4844a) |
| batch, corpus x5 (official, 08:18 CST) | 1799 fits/s |
| latency one fit, official | 0.017 ms (artefact, see PROGRESS) |
| latency one fit, dev reruns | 0.146 / 0.194 ms |
| cold start, one process per case | 19.8 ms (binary dataset cache) |
| incremental edit, rifter module toggle | ~83–90 µs (best of several runs) |
