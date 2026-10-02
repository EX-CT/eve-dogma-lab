package dogma

import (
	"bytes"
	"math/rand"
	"sync"
	"testing"
)

func TestNodeCacheMatchesMap(t *testing.T) {
	r := rand.New(rand.NewSource(1))
	var c nodeCache
	ref := map[uint64]float64{}
	for step := 0; step < 200000; step++ {
		k := nodeKey(r.Intn(300), uint32(r.Intn(400)+1))
		switch r.Intn(4) {
		case 0, 1:
			v := r.Float64()
			c.put(k, v)
			ref[k] = v
		case 2:
			c.del(k)
			delete(ref, k)
		default:
			v, ok := c.get(k)
			if w, wok := ref[k]; ok != wok || v != w {
				t.Fatalf("step %d key %x: got %v,%v want %v,%v", step, k, v, ok, w, wok)
			}
		}
		if step%50000 == 0 {
			c.reset()
			clear(ref)
		}
	}
	if c.n != len(ref) {
		t.Fatalf("count %d want %d", c.n, len(ref))
	}
}

// TestPooledReuseConcurrent: recycled Fits (any order, many goroutines) give byte-identical output.
func TestPooledReuseConcurrent(t *testing.T) {
	ds := testDataset(t)
	_, reqs := loadRequests(t)
	want := make([][]byte, len(reqs))
	for i, r := range reqs {
		f, err := Build(ds, r)
		if err != nil {
			want[i] = appendJSON(nil, calcRaw(ds, r), true)
			continue
		}
		f.noPool = true // fresh, never recycled
		want[i] = appendJSON(nil, f.ComputeStats(r, EngineName), true)
	}
	var wg sync.WaitGroup
	for g := 0; g < 8; g++ {
		wg.Add(1)
		go func(g int) {
			defer wg.Done()
			for n := 0; n < 3; n++ {
				for k := range reqs {
					i := (k*7 + g*31 + n) % len(reqs)
					if got := appendJSON(nil, calcRaw(ds, reqs[i]), true); !bytes.Equal(got, want[i]) {
						t.Errorf("request %d differs after reuse", i)
						return
					}
				}
			}
		}(g)
	}
	wg.Wait()
}
