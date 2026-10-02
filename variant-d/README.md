# Variant D — eve-dogma-ts (TypeScript dogma engine)

Zero-native-dependency TypeScript engine (Node ≥ 20 and browsers) implementing the eve-dogma-rs stateless CLI
contract on the shared dataset. Design: [DESIGN.md](DESIGN.md); status: [PROGRESS.md](PROGRESS.md);
latest bench scorecard: [bench-results/scorecard.md](bench-results/scorecard.md).

```sh
npm ci && npm run build
node dist/cli.js cache --dataset ../../data/dataset-3569502.json.gz   # optional: fast cold-start cache (.cache/)
node dist-cli/eve-dogma-ts.cjs calc --dataset ...                     # same CLI as one CommonJS file (faster start)
node dist-cli/eve-dogma-ts.cjs snapshot --dataset ...                 # optional: V8 startup snapshot, dataset preloaded
node --snapshot-blob dist-cli/eve-dogma-ts.blob calc --dataset ...    # fastest cold start (what bench.yaml runs)
node dist/cli.js calc  --dataset ../../data/dataset-3569502.json.gz < request.json
node dist/cli.js batch --dataset ... < requests.ndjson                # one request per line
node dist/cli.js serve-stdio --dataset ...                            # JSON-RPC over stdio
node dist/test/parity.js --dataset ...                                # eve-dogma-rs fixtures vs pyfa oracle
python3 score_bench.py                                                # bench cases vs Pyfa (326/326 at 1.8.0)
```

Library: `import { calc, search, typeInfo } from './dist/index.js'` with a `Dataset` from
`dist/node.js` (`loadDatasetFile`) or `dist/browser.js` (`loadDatasetUrl`, uses DecompressionStream + SubtleCrypto).
The cache (`VDC4`) is a pure re-layout of the dataset (columnar type table + lazily decoded per-type bodies); set
`EVE_DOGMA_TS_NO_CACHE=1` to bypass it, `EVE_DOGMA_TS_CACHE_DIR` to move it (library default `~/.cache/eve-dogma-ts`).

## Browser

```sh
npm run build:web     # dist-web/eve-dogma-ts.js (classic script, global EveDogma) + eve-dogma-ts.mjs (ES module)
npm run check:web     # bundle vs Node build: byte-identical output on all bench cases
```
Zero-dependency bundle: `tsc` emits one AMD file (tsconfig.browser.json) and `tools/bundle.mjs` wraps it with a
20-line loader (~155 KB unminified). `web/index.html` is a demo page (serve the repo root, e.g.
`python3 -m http.server`, open `variant-d/web/index.html?dataset=<url of dataset .json.gz>`); `?selftest=1` prints a
one-line verdict (verified with headless Chrome: same values and dataset sha256 as the CLI).
