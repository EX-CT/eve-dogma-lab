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

## Trade-offs
* Per-query overhead (interning `(item, attr, layer)`, memo lookup, dependency recording) makes a cold, from-scratch
  fit ~3x slower than variant A's plain `Cell` cache (≈3.2 ms vs ≈1.1 ms).
* In a session, an edit costs ≈0.8–1.6 ms and an unchanged re-request ≈1.0 ms — currently dominated by
  re-building specs for ~500 skills and re-running the (non-memoised) stats layer. Next steps: memoise spec
  building per request section, make stats sections salsa queries, cheaper keys (avoid interning per `get`).
* Memory grows with distinct keys; the session resets itself after 20 000 slots.

## Measurements (box, 1 thread)
| | ms |
|---|---|
| bench latency (same fit repeated, session) | ~1.0–1.6 |
| fresh database per fit (`--fresh`) | ~3.2 |
| incremental edit, toggle one module (rifter / hyperion / RAH hyperion) | 0.82 / 1.60 / 1.08 |
