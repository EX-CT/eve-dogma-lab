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
bin/eve-dogma-go --dataset $D serve-http -addr :8080     # long-running, see below
bin/eve-dogma-go --dataset $D serve-stdio                # long-running JSONL RPC, see below
bin/eve-dogma-go --dataset $D eft fit.txt --calc         # EFT -> request (-> stats)
bin/eve-dogma-go --dataset $D search rifter            # also: type 587, meta, bench request.json -n 1000
```

## Long-running modes (no per-process startup)

Starting a process costs about 125 ms (gunzip and parse the 7 MB dataset); a warm calculation takes about 0.2–0.5 ms.
For MCP servers, web backends and editors, keep one process alive and stream requests to it:

| mode | protocol | use it for |
|---|---|---|
| `batch [-j N]` | stdin: one FitRequest JSON per line, stdout: one FitStats JSON per line, **same order** | simplest pipe; also bulk jobs |
| `serve-stdio [-j N]` | JSONL RPC `{"id":..,"method":"calc"\|"eft_parse"\|"eft_export"\|"search"\|"type"\|"meta","params":..}` → `{"id":..,"result":..}` | MCP / LSP-style child process |
| `serve-http [-addr :8080]` | `POST /v1/calc` (body = FitRequest, 400 on error), `POST /v1/batch` (JSONL), `POST /v1/rpc`, `POST /v1/eft/parse`, `POST /v1/eft/export`, `GET /v1/search?q=`, `GET /v1/type/{id}`, `GET /v1/meta`, `GET /healthz` | web apps, multiple clients |

The stdin modes compute up to N requests in parallel (default: all cores) but always answer in input
order. Output is flushed whenever no further reply is pending, so a client that sends one line and waits
gets its answer immediately, and a bulk stream is still written in large chunks. Per-calculation memory
is recycled between requests (`sync.Pool`), so a long-running process settles to a small, steady heap.

```sh
# one process, many calculations
bin/eve-dogma-go --dataset $D serve-stdio
{"id":1,"method":"calc","params":{"ship":{"type_id":587},"modules":[{"type_id":3831,"state":"active"}]}}
{"id":2,"method":"search","params":{"query":"rifter"}}

bin/eve-dogma-go --dataset $D serve-http -addr 127.0.0.1:8080 &
curl -s -X POST --data-binary @request.json localhost:8080/v1/calc
```

Measured on the shared box (load average about 10): `python3 tools/stream_latency.py` gives a warm round
trip of 0.49 ms median for `batch` and 0.78 ms for `serve-stdio`, with one request in flight; the first
reply, including the dataset load, takes about 210 ms. `python3 tools/http_load.py 127.0.0.1:8080 8 3000`
gives about 2,700 req/s, and the Python client is the bottleneck.

## Library

```go
ds, _ := dogma.LoadPath("dataset-3569502.json.gz") // immutable, safe for concurrent use
stats := dogma.Calc(ds, req)                        // map[string]any, contract shape
out := dogma.CalcJSON(ds, requestBytes)             // bytes, contract JSON (errors included)

f, _ := dogma.Build(ds, req)                        // incremental use (f.Release() when done: recycles buffers)
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
