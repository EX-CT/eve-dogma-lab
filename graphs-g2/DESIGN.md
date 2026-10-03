# graphs-g2: graph primitives + portable evaluator (round 2, approach G2)

Status: **111/111 cases, 1843/1843 sample values** on `eve-dogma-bench@graphs-round2` (CONTRACT-GRAPHS.md rev 0.1),
the same with the engine compiled to WebAssembly. The engine still scores 326/326 on bench 1.8.0 (stats).

## Architecture

```
GraphRequest ──► engine (Go, variant-c)  ──► primitives JSON ──► evaluator (TypeScript, no deps) ──► GraphResult
                 eve-dogma-go graph-primitives                    graphs-g2/src/evaluator
                 (native, or the same Go compiled to WASM)        (Node or browser)
```

1. **Engine stage** (`variant-c/dogma/graphprim.go`, `graphcharges.go`, CLI `graph-primitives`, JSONL in/out):
   one `Build` of the source fit (plus the target fit, plus derived fits where the graph needs a re-evaluated fit)
   and an export of everything the formulas need. Nothing graph-specific is evaluated here except fits that Pyfa
   itself re-computes (subwarp speed, scrammed target, charge variants).
2. **Evaluator stage** (`src/evaluator/*.ts`): pure functions `evaluate(request, primitives) → result`. No I/O, no
   Node APIs, so the same code runs in a browser: fetch the primitives once per fit, then re-evaluate any axis,
   target, speed, angle or setting instantly. `src/cli/graph-batch.ts` is the bench shim (spawns the engine once
   for the whole batch, then evaluates).

## Primitives schema `eve-dogma-graph-primitives/1`

```jsonc
{ "schema": "eve-dogma-graph-primitives/1", "engine": "…",
  "source": FitPrim,
  "target": {"normal": FitPrim, "scrammed": FitPrim?},   // target fit only; scrammed = MWD/MJD forced online
  "subwarp_speed": 123.4,                                // warp_time only: prop/cloak/siege/… online, no projected
  "charges": {"group": 74, "modules": [11, 12], "variants": [ChargeVariant…]} }   // application_profile only
FitPrim = { ship: {type_id, name, attrs (all modified attrs), effects, character, stack: {maxVelocity, signatureRadius}},
            items: [ItemPrim…], cap_drains: [CapDrain…], stats: <round-1 stats object> }
ItemPrim = { kind: module|drone|fighter, index, type_id, name, group_id, state, attrs, effects,
             effect_ranges: {<effect>: {range, falloff, tracking, resistance_attr, category}},
             weapon_kind, volley[em,th,ki,ex], cycle{raw_ms, reactivation_ms, reload_ms, shots, charges, avg_ms, avg_reload_ms},
             charge{attrs, effects, group}, spool, quantity, active, abilities }
StackInputs = { base, mods: [{op, value, penalized}], value, high_is_good, min?, max? }   // re-fold with extra multipliers
ChargeVariant = { type_id, name, group, meta_group, meta_level, source: {items: <dominant modules only>, offense} }
```

`stack` (base value + modifier list) lets the evaluator re-apply the source's webs / target painters to a *target
fit* with the correct stacking penalty (Pyfa's "extended" attributes) without the engine.

## Graph → evaluator mapping

| graph | module | notes |
|---|---|---|
| `lock_time`, `warp_time`, `mobility`, `shield_regen` | `simple.ts` | closed forms (EVE scan-res lock formula, EVE Uni warp model, agility exponentials, shield regen curve) |
| `capacitor` | `capsim.ts`, `capacitor.ts` | TS port of the variant-c cap simulator with event history; value = last event ≤ t advanced by regen |
| `ewar` | `ewar.ts` | per-source range factor × strength; stacking-penalised products for web/damp/TD/GD/TP |
| `remote_reps` | `rr.ts` | shield reps land at cycle start, armor/hull at cycle end; ancillary reloads / paste rules |
| `damage` | `damage.ts`, `timecache.ts` | dealers from the stats weapons/drones/fighters; turret chance-to-hit with wrecking hits, missile explosion formula + flight-range chance, drones (follow / attacker centre), fighters (drf = ln RF / ln RS), bombs, breacher DoT, vorton chain, doomsday ticks; time axis = per-cycle schedule incl. reloads and spool |
| `application_profile` | `app.ts` | damage model per charge variant, max per point (below) |

### application_profile (observed behaviour)

- Dominant weapon group: the module group with the most active charge-using turrets / missile launchers; only
  those modules count (drones, smartbombs, bombs, vorton projectors and other weapons are excluded; command
  bursts and scripted EWAR modules are not weapons). Spool-up weapons count unspooled; lock range is ignored.
- Candidates: published, on-market charges of the module's charge groups with matching charge size and volume ≤
  capacity. Tiers: `t1` = Tech I meta group; `navy` = everything except the top faction tier (the +20 % damage
  pirate charges: Dread Guristas, Guardian, True Sansha, Dark Blood, Domination), so Tech II, empire navy (+15 %)
  and the lower pirate tier (+10 %, e.g. Shadow, Guristas, Arch Angel; the only faction XL hybrids besides the top
  tier) stay; `all` = everything.
- The engine rebuilds the fit with each candidate loaded into every module of the group and exports the changed
  module primitives and offense stats; the evaluator runs the damage model per variant and keeps the max.
- The target's speed and signature after the source's webs / painters are **sampled on a distance grid** and
  interpolated linearly between grid nodes: a 0-falloff web ending at 10 km fades out linearly up to the next node.
  Grid = 250 steps over the profile's reach rounded up to 25 km; reach = the longest turret optimal + 2 × falloff
  over every loadable charge, or for launchers the longest missile flight (velocity × flight time) over the
  charges of the requested tier. This was derived from black-box probes of the oracle and is checked by `tools/check_probes.py`
  (`testdata/oracle-probes.jsonl`: 319 extra oracle requests, 7493 values: web ranges, other webs, overheat,
  tiers, random application profiles, random damage requests (all axes, settings, drone modes, targets) and
  random requests of the other seven graphs on every corpus fit): 7486/7493 correct; the 7 misses are the crossover case below.
- Known deviation: very close to a charge crossover inside an interpolated stretch Pyfa sometimes keeps the
  previous charge for a few metres (e.g. Hyperion navy tier at 10 006–10 011 m it keeps Void, we switch to Caldari
  Navy Antimatter at 10 006 m, which is 1 % higher). Not in the corpus; noted for the contract discussion.
- `<y>_charge_type_id`: the first (lowest type id) charge with the best value; informational (ties).

## Licensing

LGPL-3.0-or-later (see `../LICENSE`). Written from CONTRACT-GRAPHS.md, public EVE formulas and black-box
observation of the Pyfa oracle; no Pyfa (GPL) graph code was copied or translated.

## Perf (shared box, Node 20, Go 1.24; numbers vary ±30 % with box load)

| metric | value |
|---|---|
| corpus via `graph-batch` (111 requests, 1843 points; engine + evaluator, incl. process start) | 0.94 s |
| engine `graph-primitives` for the corpus (incl. dataset load) | 0.55 s |
| evaluator only, replaying cached primitives (`--eval-only`) | 0.33 s incl. Node start |
| evaluator only, in-process (`bench/perf.mjs`) | ~90 k points/s, ~5.5 k requests/s |
| dense interactive: `damage` vs distance, 500 points, evaluator only | ~0.45 ms |
| dense `application_profile`, 500 points, 50 charges | ~14–19 ms (identical weapons merged, target geometry shared across charges) |
| cold start + one dense damage request (engine + Node) | ~190 ms |
| primitives JSON size | median 41 KB per request; application_profile up to 1.1 MB (one module set per charge) |
| WASM engine (`GOOS=js GOARCH=wasm`, 11.6 MB) + evaluator, whole corpus | 111/111, 8.0 s (WASM start + dataset load per batch) |

## Layout / commands

```
graphs-g2/src/evaluator/   portable evaluator (index.ts: registry, evaluate(), graphs())
graphs-g2/src/cli/         graph-batch shim (--eval-only primitives.jsonl, --dump-primitives file)
graphs-g2/src/test/        unit + golden tests (npm test)
graphs-g2/testdata/        golden requests + primitives + results
graphs-g2/bin/             graph-batch, eve-dogma-wasm
graphs-g2/bench/           scorecards, perf.mjs
graphs-g2/tools/           check_probes.py (extra oracle probes)
graphs-g2/web/             browser demo
graphs-g2/score.sh         build + score against a graphs-round2 checkout
```
