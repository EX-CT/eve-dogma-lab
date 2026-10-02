# Variant D — TypeScript pull-based attribute graph with a typed modifier pipeline

> 中文摘要：方案 D 用 TypeScript 实现同一个无状态契约（`FitRequest` → `FitStats`），零运行时依赖，
> Node 与浏览器都能直接跑（前端/MCP 不需要 WASM 或原生二进制）。核心是“按需拉取的属性图 + 类型化修饰器流水线 +
> 特殊效果注册表 + 预建目标索引”，目标是**最易维护、最适合 Web**，同时尽量逼近 Rust 的速度。

## Goals (in priority order)

1. **Correctness**: identical numbers to Pyfa on the shared oracle corpus (same `pyfa_expected.json` that
   eve-dogma-rs uses: 207 fits / 9 827 values, tolerance `max(1e-3, 1e-4·|x|)`).
2. **Same contract** as eve-dogma-rs `docs/contract.md`: CLI `calc | batch | serve-stdio | eft | search | type | meta | bench`,
   same JSON in/out, same error codes, same dataset file (`dataset-<build>.json.gz`).
3. **Maintainability**: every dogma rule lives in one small, named, typed place; adding a special effect is
   one registry entry; no 1000-line functions.
4. **Web-friendliness**: pure ESM, no `fs`/`zlib` in the core (only the CLI adapter touches Node APIs);
   the browser gets the same `calc()` with `DecompressionStream` for the gz dataset.
5. **Speed**: beat Pyfa by a wide margin; get within ~2–3× of Rust.

## Architecture

```
src/
  core/
    dataset.ts      Dataset: raw JSON -> indexed, typed tables (lazy per-type attribute maps)
    operators.ts    Operator table (data, not code): order, kind (assign/add/mul), transform, penalisable
    graph.ts        AttrGraph: items + lazily materialised attribute cells, pull evaluation, epoch cache
    domains.ts      Target resolution through prebuilt indexes (location / group / required skill / owner)
    specials.ts     Registry of special effects (effects CCP ships without modifierInfo), keyed by effect name
    build.ts        FitRequest -> item graph (ship, char, skills, modules, charges, drones, fighters, ...)
    rah.ts          Reactive armor hardener adaptation
  stats/
    resources.ts offense.ts defense.ts capacitor.ts navigation.ts targeting.ts validation.ts
    capsim.ts       event-driven capacitor simulator (Pyfa-compatible heap ordering)
    index.ts        compose sections -> FitStats (fixed key order, 1e-6 rounding like eve-dogma-rs)
  formats/eft.ts    EFT import/export incl. mutation blocks
  index.ts          public API: loadDataset(json), calc(ds, req), calcJson(ds, str), eftParse, eftExport, search, typeInfo, meta
  node.ts           Node helpers: read gz file
  cli.ts            stateless CLI (same commands as eve-dogma-rs)
  test/parity.ts    oracle parity runner (reads pyfa_expected.json + EFT fixtures)
  bench/bench.ts    per-calc timing on the corpus
```

### 1. Attribute graph (pull-based, lazily materialised)

* An `Item` does **not** copy its type's attributes. Base values are read through a three-level lookup:
  `item.baseOverrides` (mutations, overrides, security modifier, skill level) → the type's attribute map
  (built once per type and memoised on the immutable Dataset) → attribute default.
* An `AttrCell` (base + modifier list + cached value) is created only when a modifier targets the attribute.
  For a typical fit ~90 % of attribute reads hit no cell at all and return the base value directly.
* Evaluation is a **pull**: `get(item, attr)` evaluates the cell's modifiers, each of which pulls its source
  value recursively; results are memoised. A busy flag breaks dogma cycles (returns base, like Pyfa/eos).
* Cache invalidation is an **epoch counter**: `graph.invalidate()` bumps an integer; a cell's value is valid iff
  `cell.epoch === graph.epoch`. RAH needs several invalidations; this makes each O(1).

### 2. Typed modifier pipeline

Modifiers are plain records `{op, penalised, source}` where `source` is a discriminated union
(`attr` | `const` | `prop` | `projected`). The operator semantics are a **data table** (`operators.ts`):

| code | name | kind | transform | penalisable |
|---|---|---|---|---|
| -1 | PreAssign | assign | – | – |
| 0 | PreMul | mul | v | yes |
| 1 | PreDiv | mul | 1/v | yes |
| 2 | ModAdd | add | v | – |
| 3 | ModSub | add | −v | – |
| 4 | PostMul | mul | v | yes |
| 5 | PostDiv | mul | 1/v | yes |
| 6 | PostPercent | mul | 1+v/100 | yes |
| 7 | PostAssign | assign | – | – |

`evaluate()` buckets the cell's modifiers by operator in one pass, then folds buckets in order: assign
(high-is-good picks max, else min), add, multiply (unpenalised directly; penalised split into >1 and <1,
sorted by |m−1| desc, weight `exp(−i²/7.1289)`), then min/max attribute clamps and the cpu/power 2-dp rounding.
Stacking-penalty exemption is decided once at registration time from the source item's category
(ship, charge, skill, implant, subsystem, structure).

### 2b. Compiled registration plans and lazy skills (performance)

* Per type (keyed by the immutable effects array, memoised on a WeakMap), effects are compiled once into a
  **plan**: resolved special handler, filtered modifier list with the skill-self filter (`extra = 0`) and the
  bastion exemption already resolved, and the list of *outgoing* (non-self) modifiers.
* **Lazy skills**: a skill's attributes are only ever read through its own outgoing modifiers. Skills are
  therefore materialised *after* the rest of the fit and only if at least one outgoing modifier resolves to an
  item of this fit (or the skill has a special). A Rifter goes from ~500 skill items to the few dozen that matter;
  validation reads trained levels from the request, not from items.

### 3. Domain resolution through indexes

Rust variant A scans all items for each location modifier (O(items × modifiers)). Variant D builds, after
the item graph is complete, maps `skillId → items requiring it` and `groupId → items`, and the location
lists (`ship-located`, `char-located`, `owned`). Each modifier then resolves its targets by a hash lookup +
small filter. Function × domain rules are in one table-like `switch` in `domains.ts`.

### 4. Special effects registry

```ts
special('moduleBonusAfterburner', propulsion);       // AB/MWD speed + mass (+ MWD sig bloom)
special('microJumpDrive', mjdSignature);             // unpenalised sig bloom
special('slotModifier', slotModifier);               // T3C subsystems
special('hardPointModifierEffect', hardpoints);
special('adaptiveArmorHardener', /* handled by rah.ts after registration */);
projectedSpecial(/^remoteWebifier/, web); ...
```

Each handler receives a small `RegCtx` (graph, item, effect, helpers `push(target, attr, op, source)`)
and returns nothing. This is the only place hand-written dogma lives; everything else is the SDE's
`modifierInfo` (+ eve-sde-pipeline data patches).

### 5. Stats as independent sections

Each stats section is a pure function `(graph, req) → object` in its own file; `stats/index.ts` composes
them. Formulas mirror Pyfa / eve-dogma-rs (spool-up, reload factoring, missile pilot multiplier,
RAH, cap sim with Pyfa heap ordering, nos income, passive shield regen peak `10/τ·0.25·HP`, lock time
`40000/scanRes/asinh(sig)²`).

### 6. Statelessness & determinism

`calc(ds, req)` has no I/O, no clock, no randomness, no module-level mutable state except memoised
*pure* derivations of the immutable Dataset (per-type attribute maps, plans, name sets). Every stats section
builds its objects in a fixed key order and floats are rounded to 1e-6 (same as eve-dogma-rs), so output is
byte-stable for a given request (verified by the bench's determinism check).

### 7. Cold start: VDC4 cache, CLI bundle, V8 code cache and startup snapshot

One process per request is dominated by loading the 5 MB dataset (gunzip + `JSON.parse` of ~25 MB + building maps)
and by compiling/warming the engine. The bench `build` step prepares four pure, rebuildable artefacts:

1. **VDC4 dataset cache.** `node dist-cli/eve-dogma-ts.cjs cache --dataset X` writes
   `.cache/<fnv64(path)>-<size>-<mtime>.vdc4`, a pure re-layout of the same dataset: a header JSON (sde info, sha256,
   attributes, groups, categories, dbuffs, an effect index), a columnar type table (id/group/category/mass/... arrays
   + byte offsets) and a body of per-type `[attrs, effects]` JSON slices, per-effect slices, mutaplasmids, the
   type-name array and the zh table. `ds.types` is a `TypeStore`: a Map on the JSON path, a `TypeTable` of lazily
   created `LazyType` objects on the cache path. Type bodies, effects, mutaplasmids, names and the name index are
   decoded on first access and memoised. zlib and node:crypto are loaded only when really needed. Load is about 33 ms.
2. **Single-file CLI.** `dist-cli/eve-dogma-ts.engine.js` holds the tsc AMD outFile plus a 20-line loader that maps
   `node:*` to `require`, so there is no ESM resolution or translation (~25 ms). `dist-cli/eve-dogma-ts.cjs` is a
   small launcher. stdout goes through `fs.writeSync`, so no stream machinery is loaded.
3. **V8 code cache.** `cache` also runs a sample calc and stores V8's code cache for the engine
   (`.cache/engine-<size>-<mtime>-<node version>.v8cc`). The launcher compiles the engine with it through
   `vm.Script` `cachedData`. A stale or foreign cache is rejected by V8 and simply ignored. It saves about 10 ms.
4. **Startup snapshot.** `node dist-cli/eve-dogma-ts.cjs snapshot --dataset X` runs
   `node --build-snapshot dist-cli/eve-dogma-ts.snapshot.cjs X`. That file evaluates the engine, loads the dataset
   from its VDC4 cache, warms it with a sample calc, detaches the cache *body bytes* (a smaller heap deserialises
   faster) and registers the CLI as the snapshot main function. Run it as
   `node --snapshot-blob dist-cli/eve-dogma-ts.blob calc|batch|serve-stdio --dataset X`. The preloaded dataset is
   used only when the requested dataset file and its cache file are the very files it was built from (path, size,
   mtime, cache head bytes). Otherwise, or with `EVE_DOGMA_TS_NO_CACHE`, it loads normally. The body bytes are
   re-read from the cache file (~3 ms). The blob is specific to the node binary (and V8 flags) that built it.
   Calcs remain the same stateless computation: the snapshot only holds what a load and pure memo tables would
   produce anyway.

Cold `calc` on the shared box (min of 12, load ~7): snapshot ≈ 101–110 ms, launcher + code cache ≈ 150–160 ms,
bare `node -e 0` ≈ 90–95 ms, and a trivial snapshot ≈ 60 ms. Most of the remaining gap is the deserialisation of
about 4 MB of dataset heap.

`batch --threads N` (or `$EVE_DOGMA_TS_THREADS`, where 0 means one per core, up to 8) answers JSONL with an ordered
pool of worker threads. Each thread does its own dataset load. Small inputs stay on the main thread, and the output
is byte-identical to a serial run. The default is serial (1): on the loaded 8-core box the pool did not beat one
thread, because every worker pays its own JIT warm-up.

### 8. V8 notes (what mattered for speed)
- Objects keep one hidden class: double fields (`Item.ovV`, `Mod.v`, `Cell.val`) start as `NaN`, never as a small
  integer, otherwise V8 migrates maps on the first fractional write (Fit.build went 1.0 → 0.4 ms).
- Per-dataset memo tables (plans, skill reach tests, attribute post-processing, per-type validation inputs) instead
  of per-fit state.
- In one process, the first few hundred calcs run mostly in the interpreter or baseline tier. Rifter: about 2.3 ms
  each over the first 100, about 0.5 ms at steady state. This warm-up, not the steady state, dominates the bench's
  500-calc latency figure. `--maglev` and semi-space/lazy-feedback flags made no measurable difference.
- The capacitor simulator keeps the processed event at the heap top and re-sifts once (same order as Pyfa's
  heappop/heappush because the event order is a strict total order).

## Trade-offs vs the other variants

| | D (TS) | A (Rust, eve-dogma-rs) |
|---|---|---|
| Runs in browser | natively, ~160 KB JS bundle, zero deps | needs wasm build |
| Runs in MCP (TS) | in-process import | subprocess / wasm |
| Raw speed | JIT, GC; expect 1.5–4× slower than Rust | fastest |
| Contributor friendliness | highest (TS) | medium |
| Type safety | structural TS types for request/stats | strong |

## Status

See `PROGRESS.md`.
