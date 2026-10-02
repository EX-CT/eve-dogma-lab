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

## License

LGPL-3.0-or-later (`LICENSE`, plus `LICENSE.GPL-3.0`, which it incorporates), following the EX-CT engine policy in
`eve-fit-docs/LICENSING.md`. Variant H is a clean-room implementation from CCP data and public formulas; Pyfa was
used only as a black-box test oracle, and no Pyfa code is included. EVE data is © CCP Games and used under the CCP
developer license; this project is not affiliated with CCP.

