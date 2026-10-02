package dogma

import (
	"bytes"
	"os"
	"path/filepath"
	"testing"
)

// TestCacheRoundTrip: a cache-loaded Dataset re-encodes to identical bytes and computes identical stats.
func TestCacheRoundTrip(t *testing.T) {
	ds := testDataset(t)
	var key [32]byte
	key[0] = 42
	enc := encodeCache(ds, key)
	ds2, err := decodeCache(enc, key)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(encodeCache(ds2, key), enc) {
		t.Fatal("re-encoded cache differs")
	}
	if ds2.SHA256 != ds.SHA256 || ds2.Build != ds.Build || len(ds2.PublishedSkills) != len(ds.PublishedSkills) {
		t.Fatal("metadata differs")
	}
	_, reqs := loadRequests(t)
	for i, r := range reqs {
		if a, b := appendJSON(nil, calcRaw(ds, r), true), appendJSON(nil, calcRaw(ds2, r), true); !bytes.Equal(a, b) {
			t.Fatalf("request %d differs with cached dataset", i)
		}
	}
	// corruption and stale keys are rejected
	bad := append([]byte(nil), enc...)
	bad[len(bad)/2] ^= 1
	if _, err := decodeCache(bad, key); err == nil {
		t.Fatal("corrupt cache accepted")
	}
	key[0] = 43
	if _, err := decodeCache(enc, key); err == nil {
		t.Fatal("stale cache accepted")
	}
}

func TestLoadPathCached(t *testing.T) {
	src := "/workspace/exct-eve/data/dataset-3569502.json.gz"
	if _, err := os.Stat(src); err != nil {
		t.Skip(err)
	}
	dir := t.TempDir()
	t.Setenv("EVE_DOGMA_CACHE_DIR", dir)
	a, err := LoadPathCached(src) // miss: parses JSON, writes cache
	if err != nil {
		t.Fatal(err)
	}
	files, _ := filepath.Glob(filepath.Join(dir, "*.bin"))
	if len(files) != 1 {
		t.Fatalf("cache files: %v", files)
	}
	b, err := LoadPathCached(src) // hit
	if err != nil || a.SHA256 != b.SHA256 || len(a.Types) != len(b.Types) {
		t.Fatal("cached load differs", err)
	}
	t.Setenv("EVE_DOGMA_CACHE", "off")
	if cacheDir() != "" {
		t.Fatal("cache not disabled")
	}
}
