# Variant K — C# / .NET 8, strongly typed rule engine (Native AOT)

An EVE Online dogma/fitting engine that implements the eve-dogma-bench contract (`FitRequest` JSON in, `FitStats` JSON out).
Variant K focuses on **clear, strongly typed rule expression and maintainability**: every special case is a small named
class registered in one ordered `RuleBook`. See [DESIGN.md](DESIGN.md).

## Build

```sh
./build.sh          # installs .NET 8 SDK into ~/.dotnet if missing, then Native AOT publish -> ./bin/eve-dogma-k
```

This produces one self-contained native executable, `bin/eve-dogma-k` (about 4.7 MB). You need the .NET SDK to build it, but not to run it.

## Run

```sh
DS=/workspace/exct-eve/data/dataset-3569502.json.gz
./bin/eve-dogma-k --dataset $DS calc < request.json > response.json     # one FitRequest -> FitStats
./bin/eve-dogma-k --dataset $DS batch < requests.jsonl > stats.jsonl    # JSONL in, JSONL out (one line per request)
./bin/eve-dogma-k --dataset $DS serve-stdio                             # JSONL RPC: calc | eft_parse | eft_export | search | type | meta
./bin/eve-dogma-k --dataset $DS eft fit.eft [--calc] [--skills 5]         # EFT text -> FitRequest JSON (or FitStats)
./bin/eve-dogma-k --dataset $DS search Rifter     # also: type 587, meta, bench request.json -n 1000
```

You can also set the dataset with `$EVE_DOGMA_DATASET`, or put `./dataset.json.gz` in the working directory.

**Dataset cache:** the first run parses the gzipped JSON dataset (~0.4 s). It then writes a binary cache, keyed by the
dataset file's SHA-256, to `$EVE_DOGMA_K_CACHE`, `$XDG_CACHE_HOME/eve-dogma-k` or `~/.cache/eve-dogma-k`. Later runs load
the cache in about 50 ms. Set `EVE_DOGMA_K_CACHE=off` to disable it. If the cache is corrupt or stale, the engine falls back to the JSON.

Bench integration is in `bench.yaml` (`build`, `cmd`, `batch_cmd`, `rpc_cmd` with `{dataset}`). The latest scorecard is in `bench/`.

## Status

- eve-dogma-bench 1.8.0 (contract 1.4.3): **326/326 cases, 21,051/21,051 values (100 %)**, EFT export 326/326; 1.464 ms/fit, 673 fits/s batch (quick run, shared box). Scorecard in `bench/scorecard.md`.
- 1.6.0/1.7.0 additions ported from reference A (c53d333, aa46025): projected remote tracking computers (assistance-gated) and TD drones, local specials without modifierInfo (superweapon/lance speed + warp status, Emergency Hull Energizer, Entosis Link, MJFG, uncharged WDFG), doomsday subcycle DPS, Python `round(v, 2)` for cpu/pg.
- 1.8.0 additions ported from reference A (9f8579c): burst projectors (web/paint/damp/track at full strength, a neut drain, an ECM jam) and the Standup weapon disruptor, Sansha/Drifter incursion system effects (unpenalised), abyssal weather and AoE cloud beacon buffs (drones included; the weather buffs Pyfa leaves unpenalised), Breach Control. All of these stay engine-side, so the output is identical with dataset r3 and r4 (r4 ships the promoted pipeline patches 0101-0103).
- Full-output diff against reference A (eve-dogma-rs 8b7053e) over all 295 bench cases and the reference's 162 test cases: identical
  within 1e-9 relative tolerance. Run `tools/compare_ref.py`.
- EFT: `eft_export` matches Pyfa's exporter on all 295 bench fits (`tools/check_eft_export.py`). `eft_parse` gives the same FitRequest as the reference on its 131 EFT test fits. `search` follows the interim contract 1.4.1 spec.

## License

This is an LGPL-3.0-or-later derivative (see `LICENSE`). The engine logic is ported from the reference engine eve-dogma-rs (LGPL).
The Reactive Armor Hardener, capacitor simulator, missile range, remote-repair diminishing returns and sustainable tank formulas
re-implement Pyfa/eos (LGPL) through that reference. No GPL code was copied.
