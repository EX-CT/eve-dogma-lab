# graphs-g2 — Pyfa graphs via engine primitives + portable evaluator

Round 2, approach G2 (see `DESIGN.md`). Score: **111/111 cases, 1843/1843 values** (`bench/scorecard.md`).

```bash
npm ci && npm run build                                   # evaluator -> dist/
(cd ../variant-c && go build -trimpath -o bin/eve-dogma-go ./cmd/eve-dogma-go)
./bin/graph-batch --dataset $D < requests.jsonl           # GraphRequest JSONL -> GraphResult JSONL
./score.sh /path/to/eve-dogma-bench@graphs-round2         # scorer
npm test                                                  # unit + golden tests
# fully portable pipeline (engine as WebAssembly):
(cd ../variant-c && GOOS=js GOARCH=wasm go build -trimpath -o bin/eve-dogma.wasm ./cmd/eve-dogma-go)
G2_ENGINE=$PWD/bin/eve-dogma-wasm ./bin/graph-batch --dataset $D < requests.jsonl
```

Browser use: `import { evaluate, graphs } from "./dist/evaluator/index.js"`; get primitives once per fit from
`eve-dogma-go graph-primitives` (or the WASM engine), then call `evaluate(request, primitives)` per slider change.

Demo: `web/index.html` (serve `graphs-g2/` over HTTP after `npm run build`, open `/web/`): sliders for target
speed/signature/angle re-evaluate a 500-point damage or application-profile curve in the browser (~1–12 ms).

License: LGPL-3.0-or-later.
