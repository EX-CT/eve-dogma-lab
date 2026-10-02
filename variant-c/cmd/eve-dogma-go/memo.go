package main

import (
	"os"
	"strconv"
	"sync"

	"github.com/EX-CT/eve-dogma-lab/variant-c/dogma"
)

// calcMemo caches CalcJSON responses by exact request bytes for the long-running server modes
// (serve-stdio, serve-http): the engine is a pure, deterministic function of (dataset, request), so a
// repeated request (a UI re-rendering the same fit, an MCP client re-asking) is answered from memory.
// Bounded: when full it starts over (no per-hit bookkeeping). batch/calc never use it, so the bench's
// batch/latency numbers are real computation. EVE_DOGMA_MEMO=0 disables, EVE_DOGMA_MEMO=N sets the size.
type calcMemo struct {
	mu  sync.RWMutex
	m   map[string][]byte
	max int
}

var memo *calcMemo

func enableMemo() {
	n := 4096
	if v := os.Getenv("EVE_DOGMA_MEMO"); v != "" {
		n, _ = strconv.Atoi(v)
	}
	if n > 0 {
		memo = &calcMemo{m: make(map[string][]byte, n), max: n}
	}
}

// calcJSON is dogma.CalcJSON with the optional response memo.
func calcJSON(ds *dogma.Dataset, req []byte) []byte {
	if memo == nil || len(req) > 1<<16 {
		return dogma.CalcJSON(ds, req)
	}
	memo.mu.RLock()
	r, ok := memo.m[string(req)]
	memo.mu.RUnlock()
	if ok {
		return r
	}
	r = dogma.CalcJSON(ds, req)
	memo.mu.Lock()
	if len(memo.m) >= memo.max {
		clear(memo.m)
	}
	memo.m[string(req)] = r
	memo.mu.Unlock()
	return r
}
