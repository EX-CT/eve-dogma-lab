# PROGRESS — Variant D (TypeScript)

Updated: 2026-10-03 08:15 CST

## Done
- Full port of the eve-dogma-rs contract in TypeScript, zero runtime dependencies (Node 20+, browsers):
  dataset loader, pull-based attribute graph with epoch cache, operator table, indexed domain resolution,
  special-effect registry (prop mods, MJD, T3C slots/hardpoints, projected webs/TPs/damps/sebos/TDs/GDs/RTCs, remote
  reps, neuts/nos/cap transfers, ECM), RAH adaptation, fleet buffs + booster fits, projected modules/drones/fighters/
  whole fits, stats sections (resources, offense incl. Pyfa missile range + doomsday subcycles, defense/tank incl.
  incoming RR and sustained tank, capacitor sim, navigation, targeting incl. jam chance, drones, validation),
  Pyfa-exact EFT import/export, CLI (`calc|batch|serve-stdio|eft|search|type|meta|bench|cache`).
- Contract 1.4.1 rulings: calc error exits 2 with the `{"error":…}` JSON on stdout (BAD_JSON for invalid JSON);
  `options` missing → `validate=true`; interim `search` spec; byte-exact `eft_export`.
- Contract 1.4.2 / bench 1.5–1.7: projected amount semantics, projected TD/GD (Effect6424/6423), RTC (Effect6428,
  disallowAssistance), TD drones (Effect6694), local "active" handlers without modifierInfo (superweapon/lance speed and
  warp status, EHE, entosis, MJFG, local WDFG), Python `round(v, 2)` for cpu/power (exact ties-to-even).
- Parity: **official bench 1.8.0: 326/326 cases, 21 051/21 051 values, EFT export 326/326, deterministic**
  (`bench-results/`). Fast inner loop: `python3 score_bench.py` (batch mode; `VD_CLI=dist-cli/eve-dogma-ts.cjs`
  for the bundle).
- Browser: zero-dependency bundle (`npm run build:web`, ~185 KB), demo page `web/index.html` verified in headless
  Chrome; `npm run check:web` = bundle output byte-identical to Node on all bench cases.
- Perf: compiled per-type plans, memoised skill reach tests, lazy skill materialisation, dense attribute defaults,
  V8 field-representation fix (NaN-initialised double fields), shared AttrPost table, capacitor sim with one-sift
  re-arm, chunked JSONL I/O, per-type validation memo. Cold start: VDC4 lazy cache (load ≈ 33 ms), single-file CLI,
  V8 code cache, and a **startup snapshot with the dataset preloaded** (`snapshot` command, used by `bench.yaml`).
  The design is in DESIGN.md §7.
- Bench 1.8.0, official run in the shared checkout (08:13 CST, bench 33db85a, D df7dcae, box load ≈ 11): **326/326,
  21 051 values, EFT 326/326, deterministic; Rifter 0.758 ms/calc, batch 947 fits/s, cold median 122 ms** (07:22 run
  at load ≈ 7: 0.757 ms, 1040 fits/s, 114 ms).
  In-process warm corpus ≈ 0.5 ms/fit. Build-side: deferred ship bonuses (Fit.build ~20% faster), validation memo.
  Small hot-path cleanups since: static attribute-name tables (defense, jam chance, warfare buffs), single-name
  effect test without an array per call, capsim wrap-key fast path, default skill level set directly.
- `batch --threads N`: ordered worker-thread pool, output identical to serial. Off by default (no gain on the loaded
  box).

## Run
```bash
npm ci && npm run build      # dist/ (ESM), dist-web/ (browser bundle), dist-cli/ (CJS CLI bundle)
node dist-cli/eve-dogma-ts.cjs cache --dataset /workspace/exct-eve/data/dataset-3569502.json.gz
node dist-cli/eve-dogma-ts.cjs snapshot --dataset /workspace/exct-eve/data/dataset-3569502.json.gz   # optional
node --snapshot-blob dist-cli/eve-dogma-ts.blob calc --dataset /workspace/exct-eve/data/dataset-3569502.json.gz < req.json
node dist-cli/eve-dogma-ts.cjs calc --dataset /workspace/exct-eve/data/dataset-3569502.json.gz < req.json   # without snapshot
python3 score_bench.py                     # all bench cases vs Pyfa expected values
node dist/test/parity.js --dataset ...      # eve-dogma-rs fixtures
```

## Tried, not kept (no gain that stood out from the noise on the loaded box)
- Deferring *all* attribute-sourced modifiers per item until first read: Fit.build got about 25% faster, but stats
  got slower by the same amount (extra checks on every read, plus materialisation).
- A direct-mapped exp() memo in the capacitor sim. Capsim runs ~25 ns per event and is iteration-bound;
  `sim_iterations` is an output, so the event loop cannot be shortened.
- Indexed loops instead of `for…of` in `registerAll` (no measurable difference).
- V8 flags; deeper snapshot warm-up; dropping decoded type names from the snapshot; `batch --threads` as a default.

## Next / gaps
- Throughput is JIT/GC bound and about 2× behind the Rust/C++ variants in batch, mostly because of JIT warm-up in
  short runs.
- Cold start ≈ 105–115 ms with the snapshot (bare node ≈ 90 ms). The snapshot already stores numeric type columns as
  typed arrays (blob ≈ 7.75 MB); heap deserialisation (≈ 37 ms) is most of what is left.
- Warm Rifter calc in-process ≈ 0.35 ms, of which JSON parse + stringify ≈ 0.06 ms.
