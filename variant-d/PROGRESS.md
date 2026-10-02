# PROGRESS — Variant D (TypeScript)

Updated: 2026-10-03 07:10 CST

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
- Browser: zero-dependency bundle (`npm run build:web`, ~160 KB), demo page `web/index.html` verified in headless
  Chrome; `npm run check:web` = bundle output byte-identical to Node on all bench cases.
- Perf: compiled per-type plans, memoised skill reach tests, lazy skill materialisation, dense attribute defaults,
  V8 field-representation fix (NaN-initialised double fields), shared AttrPost table, capacitor sim with one-sift
  re-arm, chunked JSONL I/O, per-type validation memo. Cold start: VDC4 lazy cache (load ≈ 33 ms), single-file CLI,
  V8 code cache, and a **startup snapshot with the dataset preloaded** (`snapshot` command, used by `bench.yaml`).
  The design is in DESIGN.md §7.
- Bench 1.8.0 official rerun (07:00 CST, box load ~8 on 8 cores, snapshot build): cold median **115 ms** (was
  203–221), batch 518 fits/s, Rifter 1.24 ms/calc. These two are noisy on this box. Local interleaved runs at load
  ~7 give 880–1100 fits/s and 0.7–0.8 ms/calc. In-process warm corpus ≈ 0.55 ms/fit.
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

## Next / gaps
- Throughput is JIT/GC bound and about 2× behind the Rust/C++ variants in batch, mostly because of JIT warm-up in
  short runs.
- Cold start ≈ 105–115 ms with the snapshot (bare node ≈ 90 ms). Next step: smaller snapshot heap (typed-array
  columns).
