# Variant D — eve-dogma-ts (TypeScript dogma engine)

Zero-native-dependency TypeScript engine (Node ≥ 20 and browsers) implementing the eve-dogma-rs stateless CLI
contract on the shared dataset. Design: [DESIGN.md](DESIGN.md); status: [PROGRESS.md](PROGRESS.md);
latest bench scorecard: [bench-results/scorecard.md](bench-results/scorecard.md).

```sh
npm ci && npm run build
node dist/cli.js cache --dataset ../../data/dataset-3569502.json.gz   # optional: fast cold-start cache (.cache/)
node dist/cli.js calc  --dataset ../../data/dataset-3569502.json.gz < request.json
node dist/cli.js batch --dataset ... < requests.ndjson                # one request per line
node dist/cli.js serve-stdio --dataset ...                            # JSON-RPC over stdio
node dist/test/parity.js --dataset ...                                # 249 cases / 13812 values vs pyfa oracle
```

Library: `import { calc, search, typeInfo } from './dist/index.js'` with a `Dataset` from
`dist/node.js` (`loadDatasetFile`) or `dist/browser.js` (`loadDatasetUrl`, uses DecompressionStream + SubtleCrypto).
The cache (`VDC2`) is a pure re-layout of the dataset (header JSON + lazily decoded per-type bodies); set
`EVE_DOGMA_TS_NO_CACHE=1` to bypass it.
