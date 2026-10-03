# Pyfa features that variant F does not have (engine, CLI, WASM interface)

Compared: Pyfa `1d9f72b71` (2026-10-01, `/workspace/exct-eve/ref/pyfa`, read for behaviour only, GPL: no code taken)
against F at `af1c04b` / `variant-f-perf` (same feature set).

F's surface today:
- **CLI:** `calc`, `batch`, `serve-stdio`, `eft`, `search`, `type`, `meta`, `bench`.
- **RPC:** `calc`, `eft_parse`, `eft_export`, `format_export`, `format_import`, `search`, `type`, `meta`.
- **WASM:** `src/wasm.rs` exports only `alloc`/`dealloc`/`calc`/`rpc` (C ABI). wasip1 runs the same CLI.

Evidence key:
- "F: rg 0" means `rg -i <term> src/` finds nothing relevant in F.
- "F: error …" is the actual response of `serve-stdio`.

## A. Fit statistics Pyfa shows that F doesn't compute

| # | feature | Pyfa evidence | F evidence | size |
|---|---|---|---|---|
| A1 | **Mining yield** (module + drone m³/s, the "Mining yield" stats panel) | `eos/saveddata/fit.py:374 minerYield`, `droneYield` (l.142); `gui/builtinStatsViews/miningyieldViewFull.py` | F: `rg -i mining src/` 0 hits; the calc output has no mining key (top-level keys: capacitor, defense, drones, meta, modules, navigation, offense, resources, ship, targeting, violations, warnings) | S |
| A2 | **Outgoing remote repair / cap transfer** (the "Remote reps" panel: shield/armor/hull/cap outgoing per s, with spool) | `eos/saveddata/fit.py:1560 getRemoteReps(spoolOptions)`, `gui/builtinStatsViews/outgoingViewFull.py` | F only models *incoming* RR (`stats.rs:581`). There's no outgoing key in the output. | S |
| A3 | **Bombing panel** (bombs needed to kill a target per bomb type / damage pattern) | `gui/builtinStatsViews/bombingViewFull.py:33` | F: `rg -i bomb src/stats.rs` only matches smartbomb DPS (l.283, 421) | S |
| A4 | **Drone EHP / drone regen columns** | `gui/builtinViewColumns/droneEhp.py`, `droneRegen.py` | F's drone rows have dps/volley/range/speed/sig only (no hp/ehp) | S |
| A5 | **Price** (ship + fit + per-module market price, Jita/ESI/evemarketer sources) | `service/price.py:70 fetchPrices`, `service/marketSources/`, `gui/builtinStatsViews/priceViewFull.py` | F: `{"method":"price"}` → `UNKNOWN_METHOD`. F is offline by design (needs a network source or a price input). | M |
| A6 | **"Affected by" / modifier sources per attribute** (item-stats Affected-by tab, skill affectors menu) | `gui/builtinContextMenus/skillAffectors.py`, `gui/builtinItemStatsViews/` | `options.sources` is parsed (`request.rs:224`) but never read: `rg "\.sources" src/` gives 0 hits. `include_attributes` returns values only. | M |
| A7 | **Heat / overheat damage** (module heat column, burnout estimate) | `gui/builtinViewColumns/heat.py` | F applies overheated *bonuses* (state `overheated`) but has no heat-damage model | M |

## B. Graphs

| # | feature | Pyfa evidence | F evidence | size |
|---|---|---|---|---|
| B1 | **All 10 Pyfa graphs**: damage stats (DPS/volley over distance and over time), application profile, capacitor over time, ECM/burst/scanres/damps, ewar, lock time, mobility, remote reps, shield regen, warp time | `graphs/data/fit*` (10 dirs) | All 10 exist on the **graphs-g4** branch (contract 0.2: 178/178 native + wasm), but they are **not in F's mainline**: `eve-dogma-f graph` → usage text. `variant-f` has no `src/graphs/`. | M (merge/port of graphs-g4) |

## C. Profiles, presets and libraries (Pyfa keeps them; F takes everything inline)

| # | feature | Pyfa evidence | F evidence | size |
|---|---|---|---|---|
| C1 | **Character / skill profiles** (All 0, All 5, saved characters, skill import/export, ESI character skills) | `service/character.py:217 importCharacter`, `:223 all0`, `:230 all5`; `eos/saveddata/character.py`, `ssocharacter.py` | F: per-request `character.skills {default_level, levels}` only; no named profiles | S (presets) / L (ESI) |
| C2 | **Implant sets** (saved sets, apply to fit; the precalculated faction sets) | `eos/saveddata/implantSet.py:25`, `service/implantSet.py`, `service/precalcImplantSet.py` | F: `implants: [type_id]` only; `rg -i implant_set src/` 0 hits | S |
| C3 | **Built-in damage-pattern presets** (NPC factions, ammo patterns, "ammo → damage pattern") | `eos/saveddata/damagePattern.py:37 BUILTINS`, `gui/builtinContextMenus/ammoToDmgPattern.py` | F: `damage_pattern {em,…}` numbers only | S |
| C4 | **Built-in target-profile presets** (~195 lines of NPC/ship profiles) | `eos/saveddata/targetProfile.py:192 getBuiltinList` | F: `target_profile {…}` numbers only | S |
| C5 | **Fit storage / management** (saved fits DB, fit browser, backup/export all, fit notes, tags) | `service/fit.py`, `eos/saveddata/fit.py` (DB-mapped), `gui/builtinAdditionPanes/notesView.py` | F is stateless by design (request → stats) | L (UI/app layer, not engine) |

## D. Import / export and integrations

| # | feature | Pyfa evidence | F evidence | size |
|---|---|---|---|---|
| D1 | **EFS export** (the Eve Fitting Stats JSON) | `service/port/efs.py` | `format_export {"format":"efs"}` → `{"error":{"code":"UNSUPPORTED_FORMAT","message":"efs"}}` | M |
| D2 | **Mutated-module text** (`muta.py`, the "copy mutated module" export) | `service/port/muta.py`, `gui/builtinContextMenus/moduleMutatedExport.py` | F reads mutations inside EFT/ESI, but has no standalone muta export or import | S |
| D3 | **ESI fittings** (SSO login, fetch / upload / delete in-game fittings) | `gui/esiFittings.py`, `service/esi.py`, `service/esiAccess.py` | none (no network) | L |
| D4 | **Killmail import**: *not a Pyfa feature either* (Pyfa has no zKill/killmail importer in `service/port/`) | `service/port/port.py` import list: EFT, EFT cfg, DNA, DNA alt, ESI, XML, multibuy (export) | n/a; F already covers all Pyfa import paths except EFS (export-only in Pyfa anyway) | – |
| D5 | **Clipboard multi-format auto-detect for files and folders** (`importFitsFromFile(s)`, threaded) | `service/port/port.py:100–193` | F `format_import` auto-detects one text. There's no file or folder import (CLI reads one file or stdin). | S |

## E. Data access / search

| # | feature | Pyfa evidence | F evidence | size |
|---|---|---|---|---|
| E1 | **Search jargon / aliases** ("mwd", "lse", "ab", …) | `service/jargon/` (defaults.yaml, jargon.py) | F `search` is name exact > prefix > substring only | S |
| E2 | **Market browser tree and meta variations** (market groups, "show variations", meta-level switch) | `gui/builtinMarketBrowser/`, `gui/builtinContextMenus/itemVariationChange.py`, `service/market.py` | F: `search` + `type` only; no market-group tree or variation lookup over RPC/WASM | M |
| E3 | **Localisation** (Pyfa's UI + item names in many languages) | `locale/` | F: English + Chinese names in `search` only | S–M |

## F. Interface gaps (CLI / WASM)

| # | gap | evidence | size |
|---|---|---|---|
| F1 | WASM C-ABI exports only `calc` and `rpc`. There's no typed JS binding (wasm-bindgen / TS types) and no streaming or batch call. | `src/wasm.rs` (36 lines, 4 exports) | S |
| F2 | No multi-fit context: compare fits, fleet of fits, "command fit" chosen by *saved fit name*. Booster/projected fits must be inlined as full requests. | `request.rs` `booster_fits: Vec<FitRequest>` | S–M |
| F3 | No incremental API: every change re-computes from scratch. Pyfa keeps a live fit and recalculates. Fine for F's latency (~0.08 ms/fit single-thread), but a UI would want `set_module`/`undo`-style editing. | design | M |

## Already covered by F (not missing, for the avoidance of doubt)

The following are covered and verified by the bench (round 1: 326/326 cases, 21051/21051 values; formats 100 %):
- Capacitor sim: stable %, depletes time, reload/stagger options, injectors, incoming neuts/nos/transfers.
- Tank: raw / effective / sustained, RAH sim, damage pattern EHP.
- Offence: weapons / drones / fighters, spool, reload factor, vs target profile.
- Fleet boosts and booster fits; projected modules / drones / fits.
- Environment effects: wormhole, abyssal, incursion.
- Structures, T3D modes, mutaplasmids, booster side effects, fighter abilities, pilot/system security.
- Validation, including missing skills.
- Formats: EFT, DNA, ESI, XML, multibuy, shipstats, EFT cfg import.

Graphs are covered on graphs-g4 only (B1).

Suggested order (smallest first; each S is roughly a day or less with bench cases from the Pyfa oracle):
A1 mining, A2 outgoing RR, C1–C4 presets, A4 drone EHP, A3 bombing, D1 EFS, E1 jargon, then B1 (graphs merge) and A6 (sources).
