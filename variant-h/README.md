# Variant H — Rust ECS dogma engine (hecs)

Stateless EVE Online fitting engine implementing the EXCT contract v1 (`eve-dogma-rs/docs/contract.md`):
one JSON `FitRequest` on stdin → one JSON `FitStats` on stdout.

## Build

```bash
cd variant-h
cargo build --release          # Rust >= 1.85
```

## Run

```bash
./target/release/eve-dogma-h --dataset /workspace/exct-eve/data/dataset-3569502.json.gz calc < request.json > response.json
./target/release/eve-dogma-h --dataset /workspace/exct-eve/data/dataset-3569502.json.gz batch < requests.jsonl > responses.jsonl
```

Dataset: `--dataset PATH`, else `$EVE_DOGMA_DATASET`, else `./dataset.json.gz`, else the shared box copy.
Other commands:

- `serve-stdio`: JSONL RPC `{"id","method","params"}` → `{"id","result"}`. Methods: `calc`, `eft_parse` `{text, skills?}`,
  `eft_export` `{fit, name?}`, `search` `{query, limit?}`, `type` `{id}` (id or name), `meta`.
- `eft FILE [--calc] [--skills N]`: EFT text → FitRequest JSON. With `--calc` it computes the stats instead.
- `search QUERY [--limit N]`, `type ID|NAME`, `meta`, `bench FILE -n N`.

See [DESIGN.md](DESIGN.md) for the architecture and [RESULTS.md](RESULTS.md) for scores.

## WebAssembly (browser / Node)

`wasm/` builds the same engine crate for `wasm32-unknown-unknown` as a dependency-free C-ABI module
(`eve_dogma_h_wasm.wasm`, about 1.4 MB). `web/eve-dogma-h.mjs` is a small loader for browsers and Node: give it the
`.wasm` bytes and the dataset bytes (`.json.gz` or `.json`), then call `calc`, `search`, `eftParse` and `eftExport`.

```bash
cd wasm && cargo build --release --target wasm32-unknown-unknown && cd ..   # needs the wasm32 std (rustup target add wasm32-unknown-unknown)
node web/test-node.mjs        # all 326 bench 1.8.0 cases vs Pyfa, plus byte-identical output to the native CLI
```

In the browser there is no file mapping and no derived cache: the dataset is parsed once in memory (about 0.55 s in
Node), then each calc takes about 1.8 ms. The native build uses mimalloc and the mmapped cache; WebAssembly uses the
default allocator.

## Known differences from Variant A (unscored)

- Module state: a requested state the module cannot take (`active` without an active effect, `overheated` without an
  overload effect) is kept as requested, as in A. With every module of a bench fit set to `overheated`, 34 of the 326
  fits differ from A in capacitor-simulation details only (`capacitor.depletes_in_s`, `stable_percent`,
  `eve_stable_percent`, `sim_iterations`). The two simulators step differently once overheat changes cycle times.
  These fields are not scored (coordinator ruling, 2026-10-03); all scored values match.
- Malformed JSON is reported as `BAD_JSON` (contract code). A currently reports `BAD_REQUEST`.

## License

LGPL-3.0-or-later (`LICENSE`, plus `LICENSE.GPL-3.0`, which it incorporates; the same texts are at the branch root
as `LICENSE` and `COPYING`), following the EX-CT engine policy in
`eve-fit-docs/LICENSING.md`. Variant H is a clean-room implementation from CCP data and public formulas; Pyfa was
used only as a black-box test oracle, and no Pyfa code is included. EVE data is © CCP Games and used under the CCP
developer license; this project is not affiliated with CCP.

