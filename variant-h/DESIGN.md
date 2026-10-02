# Variant H — design

## Goal

Same contract as Variant A (one `FitRequest` → one `FitStats`, stateless, deterministic), built as an
**Entity-Component-System**: dogma items are entities, attributes/modifiers are components, effect application
and attribute calculation are systems run in a fixed order.

## Why `hecs`

| option | verdict |
|---|---|
| **hecs 0.10** (chosen) | Small, no scheduler/global registries, `World::new()` is cheap, so a *fresh world per request* costs almost nothing. Random-access `View`s give fast `entity -> component` lookups, which is what dogma needs (modifier sources and targets point at arbitrary entities). Builds on Rust 1.85 (hecs 0.11 needs a newer compiler). |
| bevy_ecs | Strong scheduler, but heavy compile, `Resource`/`Schedule` setup per request, and its parallel executor buys nothing for a ~1 ms single-fit job. |
| hand-rolled | Possible, but then "ECS" is just `Vec<Item>` (= Variant A/B). Using a real archetype store keeps the design honest. |

The schedule is explicit Rust code (`Fit::run`) instead of a scheduler: every system is a method with a
documented read/write set, so the order is visible and deterministic.

## Entities and components (`src/components.rs`)

One entity per ship, character, skill (every published skill, untrained = level 0), module, charge, drone stack,
fighter squadron, implant, booster, T3D mode, environment beacon, projected source.

| component | on | content |
|---|---|---|
| `Item` | all | type id, group, category, `Kind`, `Loc` (ship/char/space/nowhere), `owned` |
| `Power` | all | effective state (offline/online/active/overheated); charges inherit their module's |
| `Attrs` | all | **sparse overlay**: only attributes that have an own base (override, mutation, skill level, security) or modifiers get an `AttrSlot {base, mods}`; everything else is read straight from the static type (no per-request copy of ~50-300 attributes per item) |
| `Fitted` | modules | slot, request index, loaded charge entity, spool option |
| `LoadedIn` | charges | parent module entity |
| `Squad` | drones, fighters | quantity, active count, request index |
| `FighterAbilities`, `SideEffects`, `Distance`, `Mutated` | as needed | fighter ability set, booster side effects, projection range, effect/required-skill lists of mutated items |

Components are plain data (`Send + Sync`); memoised results are **not** stored in components but in the
calculation system's memo table, so the world can be re-evaluated or inspected at any point.

## Systems (`src/fit.rs`, run by `Fit::run`)

1. **spawn** — request → entities; mutations (base type attributes + rolled values clamped to the mutaplasmid range),
   T3D default mode, system security → `securityModifier`, attribute overrides.
2. **build_index** — per-request lookup tables (ship-located items, by group, by required skill, char-located items).
   Location-filtered modifiers (`LocationGroupModifier`, `OwnerRequiredSkillModifier`, …) hit these indices
   instead of scanning every entity.
3. **local_effects** — for each entity in spawn order, every effect allowed by its state (passive/online/active/
   overload categories, fighter abilities, booster side effects, structure rules) turns its modifierInfo into
   `PendingMod`s on target entities. Special handlers for effects CCP ships without modifierInfo (AB/MWD speed and
   mass, MWD/MJD signature, slot and hardpoint modifiers, bastion hull resists unpenalised; Emergency Hull Energizer
   hull resonances in the postMul penalty chain; entosis link scan strengths; Micro Jump Field Generator signature;
   Warp Disruption Field Generator mass/signature/propulsion boosts; lance / disruptive lance speed and warp status;
   incursion beacons' `OffensiveDefensiveReduction` damage and resist nerfs, unpenalised).
   Read phase over the world → command buffer → write phase appends to the target `Attrs`.
4. **projected_effects** — projected modules/drones onto the ship: modifierInfo or name-based handlers (webs,
   target painters, sensor dampeners/boosters, tracking/guidance disruptors, remote tracking computers, TD drones,
   bomb launchers), with range factor (optimal/falloff) and the target's `remoteResistanceID` attribute applied
   lazily. Burst projectors (`doomsdayAOE*`) apply at full strength at any distance; the Standup Weapon Disruptor
   hits turrets and missiles with the range factor.
5. **fleet_buffs** — explicit `fleet.buffs`, this fit's active command bursts, and each `fleet.booster_fits` entry
   (each booster fit is its **own world** run through systems 1-3, then its bursts are read). Per buff id the
   strongest magnitude wins (Pyfa `commandBonuses`), then dbuff modifiers are applied. Abyssal weather and AoE
   cloud beacons in `environment.effect_type_ids` add their `warfareBuff1/2` to the same pool. Buffs 79, 90 and
   93–99 also hit drones that require Drones, and the weather resist/HP/velocity buffs are unpenalised.
6. **rah_adapt** — Reactive Armor Hardener: evaluates the ship's armor resonances, simulates the RAH cycles until
   the profile loops, applies the averaged profile.
7. **output systems** (`stats.rs`, `validate.rs`) — read-only: resources, offense (turrets/missiles/smartbombs/
   vorton/spool, drones, fighters), defense/EHP/tank, capacitor (event simulation `capsim.rs`), navigation,
   targeting, violations, optional attribute dumps.

## Attribute calculation (`src/calc.rs`)

`Calc` holds a read-only `ViewBorrow<&Attrs>` and a memo table keyed by `(entity, attribute)`. `get` evaluates
lazily and recursively (modifier sources are other entities' attributes): CCP operator order PreAssign → PreMul →
PreDiv → ModAdd → ModSub → PostMul → PostDiv → PostPercent → PostAssign; stacking penalty
`exp(-(i/2.67)^2)` for non-stackable attributes unless the source category is exempt (ship, charge, skill,
implant, subsystem, structure); then `minAttribute`/`maxAttribute` caps and Python-`round(v, 2)` rounding of
cpu/power (correctly rounded on the binary value).
A cycle guard returns the base value. Systems that need evaluated values (buff ids, RAH) create a short-lived
`Calc`, read, drop it, then write — Rust's borrow rules enforce the read/write phase split.

## Trade-offs

- **Fresh world per request** keeps the engine trivially stateless and deterministic; the price is spawning
  ~500 skill entities per calculation. The sparse `Attrs` overlay keeps that cheap (one slot per skill).
- **Lazy pull evaluation** instead of a topologically sorted push pass: simpler and only computes what output
  needs; the cost is a hash lookup per access.
- **Spawn-order lists** (`Fit::order`, `modules`, `drones`, …) are kept next to the world because archetype
  iteration order is not request order; every output is produced in request order.
- Formulas and Pyfa conventions (spool, missile multiplier, nos as income, fighter default abilities, capsim
  details) follow the shared contract and Variant A's documented behaviour; no Pyfa (GPL) code is included.
  Pyfa is only used as a black-box oracle through the bench repo.

## Known gaps

- Implemented and checked against Pyfa:
  - projected remote reps, neuts, nos and cap transfers
  - projected modules, drones, fighters and whole fits
  - booster fits, environments
  - sustained tank
  - ECM jam chance
  - per-weapon and per-drone application fields
- EFT import/export, `search` and `type` live in `src/tools.rs`; they're reachable from the CLI and from
  `serve-stdio`. Export reimplements the behaviour of Pyfa's EFT exporter, checked black-box. No Pyfa code is used.
- Known divergences from Pyfa are the bench's `known_divergences.json` entries: invalid fits and SDE/eve.db data
  drift.
- Projected bomb launchers follow Pyfa's rules. Void and focused void bombs drain the charge's
  `energyNeutralizerAmount` every `speed + moduleReactivationDelay`. Lockbreaker bombs add jam strength with no
  resist. The interdiction sphere gives an unpenalized `maxVelocity` boost of the probe's `speedFactor`
  (attribute default when the type doesn't set it).

## Performance notes

- Hot path: hecs `World::get` was replaced by `views::Views`, which takes one ViewBorrow per component
  type for each pass. This gave about 1.6x batch throughput.
- A derived bincode cache (`dataset.hcache`, keyed by xxh3 of the dataset plus the binary's identity)
  cuts cold start from about 120 ms to a few ms. The cache is memory-mapped, and its tables are zero-copy
  `LazyTable`s: ids, offsets and a dense id index are read straight from the mapping, and each record is
  bincode-decoded on first access into cells allocated per 64-record chunk. A process only pays for the types,
  attributes and effects its request touches.
- Skills whose modifiers cannot reach anything in the request are not spawned (`skill_reach`).
- Release builds use fat LTO, one codegen unit, `panic = "abort"` and stripping; `--profile profiling` keeps
  debuginfo for callgrind.

## Portability and tests

The engine library has no platform-specific code paths except the derived-cache mmap and the mimalloc allocator,
which are native-only dependencies. On `wasm32`, a stand-in `Mmap` always fails to map, so the existing
read-and-parse fallback runs. `wasm/` wraps the library in a C-ABI `cdylib`: inputs go into `h_alloc` buffers and
results are read back with `h_result_ptr` / `h_result_len`, so there is no generated JS glue. `web/eve-dogma-h.mjs`
is the loader.

`tests/pyfa_parity.rs` replays every bench 1.8.0 case against the bench's Pyfa-expected values at the bench
tolerance, one test per case family (fixtures in `tests/fixtures/`, generated from the bench repository). It also
replays the staged pending-1.9.0 cases. `tests/unit.rs` covers the contract helpers.

