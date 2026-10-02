package dogma

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

// TestGenRequests writes the FitRequest of every oracle case to testdata/requests/<name>.json
// (run with GEN_REQUESTS=1). Used for cross-engine diffs and benchmarks.
func TestGenRequests(t *testing.T) {
	if os.Getenv("GEN_REQUESTS") == "" {
		t.Skip("set GEN_REQUESTS=1")
	}
	ds := testDataset(t)
	b, _ := os.ReadFile("../testdata/oracle/pyfa_expected.json")
	var exp struct {
		Fits map[string]oracleFit `json:"fits"`
	}
	_ = json.Unmarshal(b, &exp)
	dir := filepath.Join("..", "testdata", "requests")
	_ = os.MkdirAll(dir, 0o755)
	for name, f := range exp.Fits {
		req, err := loadOracleCase(ds, f)
		if err != nil {
			t.Fatal(err)
		}
		_ = os.WriteFile(filepath.Join(dir, name+".json"), Marshal(req), 0o644)
	}
}
