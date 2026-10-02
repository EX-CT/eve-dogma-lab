# Variant K — design

**Goal:** match reference A exactly, with **rules that read like a rulebook** and a type system that catches mistakes at
compile time. Speed comes second, but Native AOT plus a binary dataset cache keeps it competitive.

## Why C# / .NET 8

- Strong nominal types with cheap value-type wrappers (`readonly record struct`). Exhaustive `switch` expressions.
  Pattern matching. Nullable reference analysis.
- Native AOT publishing gives one native executable with ~3 ms runtime startup and no JIT warm-up. That suits the
  `cmd < request.json` process-per-fit model.
- `System.Text.Json` (reader/`JsonElement`) is used without reflection, so it is AOT-safe and trimming-safe.

## Layers

```
Data/      dataset model + loader + binary cache      (immutable after load, shared across fits)
Requests/  typed FitRequest records + parser           (all input validation -> BAD_REQUEST)
Engine/    attribute graph (Fit), FitBuilder, TargetIndex, Calculator
Rules/     RuleBook: gates, effect rules, projected rules, fit passes  <- all the dogma special cases
Stats/     derived stats (offense/defense/capacitor/...), CapSim, Validation
Json/      deterministic JSON writer (sorted keys, 6-decimal rounding, contract float formatting)
Cli/       calc / batch / serve-stdio / search / type / meta / bench
```

## Typed identifiers

`AttrId` and `EffectId` are distinct `record struct`s, so you cannot pass an attribute id where an effect id is expected.
`Modifier` and `EffectRef` are typed records as well.
`Op`, `ModFunc`, `ModDomain` and `EffectCategory` are enums, not magic ints.
`KnownIds` resolves every attribute or effect the engine names **by name, once per dataset**. If a dataset lacks one, it fails at
load time rather than silently mid-calc. Rules refer to `k.MaxVelocity` and similar names, never to `37`.

## The RuleBook

`Rules/RuleBook.cs` is the single place that decides how an effect turns into modifiers. It is an ordered list of:

1. **Gates** (`IEffectGate`) decide *whether* an effect applies: state (offline/online/active/overload), structure-only
   skills, booster side effects, fighter abilities.
2. **Effect rules** (`IEffectRule`) decide *what* an effect does. The first rule whose `Matches` returns true wins.
   Named special cases (`Propulsion`, `MicroJumpDrive`, `SlotModifier`, `HardpointModifier`, ...) come first.
   `DataDrivenModifierRule`, which interprets the dataset's `modifierInfo`, is always **last**. Adding a special case means
   writing a class and placing it in the list. Nothing else changes.
3. **Projected rules** (`IProjectedRule`) handle effects applied onto another fit (webs, painters, damps, sensor boosters,
   data-driven projections). The `IncomingEffects` table describes non-modifier projections (remote reps, neuts, nos, cap transfer)
   as typed `IncomingRepair` / `IncomingCapacitor` records that the stats layer consumes.
4. **Fit passes** (`IFitPass`) are whole-fit steps that need evaluated attributes: warfare/command bursts (strongest per buff id
   across own bursts and fleet booster fits, explicit buffs override) and the Reactive Armor Hardener adaptation.

The per-effect rule choice is cached, so dispatch cost is paid once per effect id.

## Attribute graph

- `Fit` holds `Item`s (ship, modules, charges, drones, fighters, implants, boosters, skills, character, projected sources).
  Each attribute is an `AttrNode` with its base value and the `AppliedModifier`s that target it.
- `ModSource` is a tagged struct that says where a modifier's value comes from (another item's attribute, a constant, a warfare
  buff). Evaluation reads through it, so sources stay lazy.
- `Eval` uses memoised lazy evaluation. A generation counter invalidates the cache when passes add modifiers, which avoids
  clearing dictionaries. Stacking penalties follow the dogma rules: penalised groups are sorted, with exp(-(i/2.67)^2) weights,
  and the stack-exempt categories are respected.
- `TargetIndex` precomputes the location, group and required-skill indexes, so `LocationGroupModifier` and
  `LocationRequiredSkillModifier` are lookups rather than scans.

## Stats

`StatsCalculator` is a straight port of the reference's derived-stats code, split into one method per section (resources,
offense, defense, capacitor, navigation, targeting, attribute dumps). It covers:

- Pyfa missile range.
- The incoming RR diminishing-returns formula.
- The sustainable tank (cap-limited local repairers, most cap-efficient first).
- `CapSim`: the event simulator, using a `PriorityQueue`.

All of this logic re-implements Pyfa/eos via the reference.

## Trade-offs and known gaps

- **Fidelity over novelty:** the build order and float operation order mirror the reference, so outputs are bit-for-bit
  comparable. `tools/compare_ref.py` diffs the full JSON output, not just the bench metrics.
- **Cache:** the binary cache makes cold start ~50 ms instead of ~400 ms, at the cost of a writable cache directory. The cache
  is optional, and its key is the dataset file's SHA-256.
- **Performance:** per-calc time is about 0.8–1.2 ms, the same order as A. The main remaining cost is registering ~500 skill items
  per fit. A per-dataset skill template would be the next optimisation.
- **Not implemented:** EFT import/export.
- **License:** this is an LGPL-3.0-or-later derivative of the reference (see README).
