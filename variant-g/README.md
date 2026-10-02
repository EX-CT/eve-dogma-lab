# Variant G — Python + NumPy batch-vectorised dogma engine

Part of the EX-CT dogma architecture lab (branch `variant-g`). Implements the eve-dogma contract v1
(`eve-dogma-rs/docs/contract.md`): one JSON `FitRequest` on stdin → one JSON `FitStats` on stdout, stateless,
same fields and units as the reference engine (variant A, eve-dogma-rs).

The idea of this variant: **many fits are computed together as array operations.** Items, modifiers and
attribute values of every fit in a batch live in flat NumPy columns; the dogma graph is levelised and every
level is evaluated with vector ops for all fits at once. See [DESIGN.md](DESIGN.md).

## Requirements / build

* Python ≥ 3.10 with NumPy (tested: Python 3.13.5, NumPy 2.5.3). No other dependencies, nothing to compile.
* Dataset: `dataset-3569502.json.gz` (EX-CT/eve-sde-pipeline release `sde-3569502`; on the EXCT box:
  `/workspace/exct-eve/data/dataset-3569502.json.gz`). On first use a derived cache (pickled NumPy columns,
  keyed by the sha256 of the .gz) is written to `$EVE_DOGMA_G_CACHE` or `~/.cache/eve-dogma-g/` (≈0.6 s once;
  afterwards loading takes ≈25 ms).

```bash
# "build" = check NumPy and warm the dataset cache
./bin/eve-dogma-g --dataset /workspace/exct-eve/data/dataset-3569502.json.gz meta
```

## Run

```bash
export EVE_DOGMA_DATASET=/workspace/exct-eve/data/dataset-3569502.json.gz   # or pass --dataset PATH

./bin/eve-dogma-g calc < request.json > response.json          # single request (contract "calc")
./bin/eve-dogma-g calc request.json > response.json
./bin/eve-dogma-g batch < requests.jsonl > responses.jsonl     # JSONL, same order; evaluated 256 at a time
./bin/eve-dogma-g batch --chunk 1024 < requests.jsonl > out.jsonl
./bin/eve-dogma-g serve-stdio                                   # JSONL RPC: calc | meta | type | search
./bin/eve-dogma-g meta | type 587 | search "Hammerhead"
./bin/eve-dogma-g bench request.json -n 500                     # per-calc time, single vs batched
```

`bin/eve-dogma-g` is a 3-line shell wrapper for `python3 -m evedogma_g` (put `variant-g/` on `PYTHONPATH`
to use it as a library: `from evedogma_g.calc import calc, calc_many`).

Errors are JSON (`{"error":{"code","message","path"}}` with `BAD_JSON`, `BAD_REQUEST`, `UNKNOWN_TYPE`,
`UNKNOWN_METHOD`); in batch mode a bad request only affects its own line.

## Tests

```bash
python3 tests/run_tests.py        # Pyfa parity on the bench corpus, batch==single, determinism, errors, unit checks
python3 tests/compare_ref.py      # every response leaf vs the reference engine (variant A) binary
```

Bench manifest: [bench.yaml](bench.yaml) (used by `eve-dogma-bench/bench.py`).

## Results

See [results/RESULTS.md](results/RESULTS.md) (bench scorecard, reference comparison, timings).

## License

LGPL-3.0-or-later, like the reference engine whose semantics it reproduces (see DESIGN.md "Provenance").
The Pyfa oracle (GPL) was only used as a black-box test tool via the bench; no Pyfa code is included.
EVE Online data © CCP hf.
