# Variant C — Go dogma engine (pull-based modifier registry)

Part of the EX-CT engine bake-off (`EX-CT/eve-dogma-lab`). Same contract as `eve-dogma-rs`
(stateless `FitRequest` JSON → `FitStats` JSON, same dataset `exct-eve-dataset` v1), different architecture.

## Goals (in order)
1. **Correctness** — identical numbers to the Pyfa oracle (`testdata/oracle/pyfa_expected.json`, 207 cases).
2. **Speed** — beat eve-dogma-rs per calculation, not just Pyfa.
3. **Maintainability / embeddability** — idiomatic Go, zero dependencies (stdlib only), library + CLI + HTTP.

## Architecture

```
dataset.json.gz ──load once──► Dataset (immutable, shared by goroutines)
                                 • types: sorted attr slices (no per-item copies), req skills, slot, effects
                                 • effects: pre-decoded modifier tuples
                                 • indices: name→id, group→types, published skills
FitRequest ──► Fit (one per calculation, cheap)
                 items[]  (ship, char, skills, modules, charges, drones, fighters, implants, boosters, mode, beacons, projected)
                 base overlay per item (only for mutated/overridden/skill-level attrs; everything else read from Dataset)
                 Registry: modifiers keyed by *selector*, NOT expanded to targets
                     (item,attr) · (shipLoc,attr) · (shipLoc,group,attr) · (shipLoc,skill,attr)
                     (owner,skill,attr) · (charLoc,attr) · (charLoc,group,attr) · (charSkill,skill,attr)
                 Cache: (item,attr) → value, memoised on first read, generation-stamped
```

### Pull, not push
eve-dogma-rs (variant A) resolves every modifier to its concrete target items at registration
(`O(modifiers × items)`), creating attribute nodes for many attributes that are never read
(≈500 skills × their modifiers × every ship item). Variant C stores each modifier once, under the selector
it was declared with (location / group / required skill / owner). When `(item, attr)` is first read, the
engine looks up the handful of buckets that can apply to that item (its own, its location, its group, each
required skill) and folds them. Work is proportional to *attributes actually read by the stats layer*.

### Evaluation
CCP operator order (PreAssign, PreMul, PreDiv, ModAdd, ModSub, PostMul, PostDiv, PostPercent, PostAssign),
per-operator stacking-penalty buckets (exempt source categories Ship/Charge/Skill/Implant/Subsystem/Structure,
`e^-(i/2.67)^2`), assign = max/min by highIsGood, min/max attribute caps, cpu/power rounding, cycle guard.

### Dirty tracking
Each cached value carries the fit's generation. `Fit` exposes mutators for incremental use (what-if loops,
optimisers, MCP sessions): `SetBase(item, attr, v)` invalidates precisely the values that depended on it
(reverse-dependency edges recorded while evaluating, opt-in via `TrackDeps`), structural changes
(`SetState`) re-register the item's modifiers and bump the generation (O(1) invalidate of everything).
The stateless CLI never mutates, so it pays nothing for this.

### Specials (no modifierInfo in the SDE)
Same list as eve-dogma-rs, implemented as explicit registry entries with computed sources:
AB/MWD (mass add + speed boost from thrust/mass), MWD/MJD signature bloom, slot/hardpoint modifiers (T3C),
bastion hull resists unpenalised, structure skill rules, booster side effects, fighter abilities, projected
webs/TPs/damps/sebos with range factor and resistance, warfare buffs (explicit + local bursts), RAH adaptation.

### Layers
| package | file | role |
|---|---|---|
| `dogma` | `data.go` | dataset loader (gzip JSON → indexed structs) |
| | `request.go` | FitRequest v1 with serde-compatible defaults |
| | `engine.go` | item graph, registry, evaluation, specials, RAH |
| | `stats.go` | resources/offense/defense/capacitor/navigation/targeting/validation |
| | `capsim.go` | Pyfa-compatible event-driven capacitor simulation |
| | `eft.go` | EFT import/export (incl. mutations) |
| `cmd/eve-dogma-go` | `main.go` | CLI: calc, batch, serve-stdio, serve-http, eft, search, type, meta, bench |

### Concurrency
`Dataset` is immutable after load → any number of goroutines can `Calc` in parallel (HTTP server, batch
`-j N`). A `Fit` is single-goroutine.

## Contract
CLI flags, JSON shapes and error codes mirror eve-dogma-rs (`calc`, `batch`, `serve-stdio`, `eft`, `search`,
`type`, `meta`, `bench`). Output keys are sorted, floats rounded to 1e-6, non-finite → null.

## Licence
LGPL-3.0-or-later (algorithms for RAH / capsim follow Pyfa eos, LGPL). EVE data © CCP hf.
