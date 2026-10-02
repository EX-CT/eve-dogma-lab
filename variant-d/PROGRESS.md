# PROGRESS — Variant D (TypeScript)

Updated: 2026-10-03 05:15 CST

## Done
- Full port of the eve-dogma-rs contract in TypeScript, zero runtime dependencies (Node 20+, browsers):
  dataset loader, pull-based attribute graph with epoch cache, operator table, indexed domain resolution,
  special-effect registry (prop mods, MJD, T3C slots/hardpoints, projected webs/TPs/damps/sebos, remote reps,
  neuts/nos/cap transfers), RAH adaptation, fleet buffs + booster fits, projected modules/drones/whole fits,
  stats sections (resources, offense incl. Pyfa missile range, defense/tank incl. incoming RR, capacitor sim,
  navigation, targeting, drones, validation), EFT import/export, CLI (`calc|batch|serve-stdio|eft|search|type|meta|bench`).
- Contract v1.4/v1.5: sustainable tank, ECM jam chance (modules, bursts, EC drones, fighters), projected fighters
  (web / point / neut / ECM), fighter self abilities (MWD / AB / evasive), drone/fighter application fields,
  cap-booster forced reload + incoming drains in `capacitor.use_gj_s`.
- Parity: **bench 1.4.0: 289/289 cases, 18 591/18 591 values** match Pyfa (`python3 score_bench.py` for a fast
  batch-mode check; local `node dist/test/parity.js` = the 249 eve-dogma-rs fixtures, 13 812 values).
- Perf: compiled per-type plans, lazy skill materialisation, memoised lookups, VDC1 fast-start cache (DESIGN §7).
  Bench (05:06 CST, load ~7 on 8 cores): cold median 253 ms, batch 482 fits/s, Rifter 0.91 ms/calc, deterministic.
  See `bench-results/`.

## Run
```bash
npm ci && npm run build
node dist/cli.js calc --dataset /workspace/exct-eve/data/dataset-3569502.json.gz < req.json
node dist/test/parity.js --dataset ...      # oracle parity on fixtures/
node dist/bench/bench.js --dataset ... --cases ../../eve-dogma-bench/cases -n 20
```

## Next
- Cold start: ~95 ms is bare Node startup; remaining ~150 ms = module load + cache header parse + first-call JIT.
  Options: single-file bundle, Node 22 compile cache / startup snapshot.
- Browser bundle + demo page (`src/browser.ts` already provides `loadDatasetUrl`).
