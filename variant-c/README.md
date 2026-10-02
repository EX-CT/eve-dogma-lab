# Variant C: eve-dogma-go

Go (stdlib only, Go ≥ 1.24) implementation of the EX-CT dogma engine contract
(`eve-dogma-rs/docs/contract.md`): one `FitRequest` JSON in, one `FitStats` JSON out, stateless,
loading the shared `exct-eve-dataset` v1 (`dataset-3569502.json.gz`). Design notes: [DESIGN.md](DESIGN.md).
Bench results: [BENCH.md](BENCH.md).

## Build

```sh
cd variant-c
go build -trimpath -o bin/eve-dogma-go ./cmd/eve-dogma-go
```

## Run

```sh
D=/workspace/exct-eve/data/dataset-3569502.json.gz      # or $EVE_DOGMA_DATASET, or ./dataset.json.gz
bin/eve-dogma-go --dataset $D calc < request.json > response.json
bin/eve-dogma-go --dataset $D batch [-j N] < requests.jsonl > responses.jsonl   # order preserved, N = NumCPU by default
bin/eve-dogma-go --dataset $D serve-http -addr :8080     # POST /v1/calc, /v1/rpc, /v1/eft/parse, /v1/eft/export; GET /v1/search?q=, /v1/type/{id}, /v1/meta
bin/eve-dogma-go --dataset $D serve-stdio                # JSONL RPC {"id","method","params"}
bin/eve-dogma-go --dataset $D eft fit.txt --calc         # EFT -> request (-> stats)
bin/eve-dogma-go --dataset $D search rifter            # also: type 587, meta, bench request.json -n 1000
```

## Library

```go
ds, _ := dogma.LoadPath("dataset-3569502.json.gz") // immutable, safe for concurrent use
stats := dogma.Calc(ds, req)                        // map[string]any, contract shape
out := dogma.CalcJSON(ds, requestBytes)             // bytes, contract JSON (errors included)

f, _ := dogma.Build(ds, req)                        // incremental use
f.EnableDepTracking()
v := f.Get(itemIdx, attrID)
f.SetBase(itemIdx, attrID, 42)                      // invalidates only dependents
```

## Test

```sh
go test ./dogma                    # Pyfa oracle parity: 249 fits / 13 812 values, determinism, EFT round-trip
go test -run x -bench . ./dogma    # micro benchmarks (per-fit latency, dataset load)
python3 tools/diff_vs_rs.py '/workspace/exct-eve/eve-dogma-bench/cases/*.json'   # full-output diff vs eve-dogma-rs
```

Bench harness entry: [bench.yaml](bench.yaml).
