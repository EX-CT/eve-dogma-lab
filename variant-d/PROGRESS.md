# PROGRESS — Variant D (TypeScript)

Updated: 2026-10-03 (Asia/Shanghai)

## Done
- Full port of the eve-dogma-rs contract in TypeScript, zero runtime dependencies (Node 20+, browsers):
  dataset loader, pull-based attribute graph with epoch cache, operator table, indexed domain resolution,
  special-effect registry (prop mods, MJD, T3C slots/hardpoints, projected webs/TPs/damps/sebos, remote reps,
  neuts/nos/cap transfers), RAH adaptation, fleet buffs + booster fits, projected modules/drones/whole fits,
  stats sections (resources, offense incl. Pyfa missile range, defense/tank incl. incoming RR, capacitor sim,
  navigation, targeting, drones, validation), EFT import/export, CLI (`calc|batch|serve-stdio|eft|search|type|meta|bench`).
- Parity: **249/249 cases, 13 812/13 812 values** match Pyfa (EX-CT/eve-dogma-bench and local `npm test`).
- Perf: compiled per-type plans, lazy skill materialisation, memoised lookups. See `bench-results/`.

## Run
```bash
npm ci && npm run build
node dist/cli.js calc --dataset /workspace/exct-eve/data/dataset-3569502.json.gz < req.json
node dist/test/parity.js --dataset ...      # oracle parity on fixtures/
node dist/bench/bench.js --dataset ... --cases ../../eve-dogma-bench/cases -n 20
```

## Next
- Cold start: dataset load is ~200 ms (JSON.parse dominates); options: Node startup snapshot, compact binary cache.
- Browser bundle + demo page (`src/browser.ts` already provides `loadDatasetUrl`).
