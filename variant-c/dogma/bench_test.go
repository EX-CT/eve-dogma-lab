package dogma

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"testing"
)

func loadRequests(b testing.TB) ([]string, []*FitRequest) {
	files, _ := filepath.Glob("../testdata/requests/*.json")
	sort.Strings(files)
	var names []string
	var reqs []*FitRequest
	for _, f := range files {
		raw, _ := os.ReadFile(f)
		var r FitRequest
		if err := json.Unmarshal(raw, &r); err != nil {
			b.Fatal(f, err)
		}
		names = append(names, filepath.Base(f))
		reqs = append(reqs, &r)
	}
	if len(reqs) == 0 {
		b.Skip("no requests (run GEN_REQUESTS=1 go test -run TestGenRequests)")
	}
	return names, reqs
}

// BenchmarkAllCases: one op = calculate every oracle case once.
func BenchmarkAllCases(b *testing.B) {
	ds := testDataset(b)
	_, reqs := loadRequests(b)
	b.ReportAllocs()
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		for _, r := range reqs {
			Calc(ds, r)
		}
	}
	b.ReportMetric(float64(b.Elapsed().Microseconds())/float64(b.N*len(reqs)), "us/fit")
}

func benchOne(b *testing.B, name string) {
	ds := testDataset(b)
	raw, err := os.ReadFile("../testdata/requests/" + name + ".json")
	if err != nil {
		b.Skip(err)
	}
	b.ReportAllocs()
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		CalcJSON(ds, raw) // the production path: JSON in, JSON out
	}
}

func BenchmarkRifter(b *testing.B)    { benchOne(b, "exct_rifter") }
func BenchmarkVexor(b *testing.B)     { benchOne(b, "esf_vexor") }
func BenchmarkTengu(b *testing.B)     { benchOne(b, "exct_tengu") }
func BenchmarkNidhoggur(b *testing.B) { benchOne(b, "exct_nidhoggur") }
func BenchmarkHyperion(b *testing.B)  { benchOne(b, "exct_hyperion") }

func BenchmarkLoadDataset(b *testing.B) {
	p := "/workspace/exct-eve/data/dataset-3569502.json.gz"
	if v := os.Getenv("EVE_DOGMA_DATASET"); v != "" {
		p = v
	}
	raw, err := os.ReadFile(p)
	if err != nil {
		b.Skip(err)
	}
	for i := 0; i < b.N; i++ {
		if _, err := LoadBytes(raw); err != nil {
			b.Fatal(err)
		}
	}
}

// BenchmarkAllCasesJSON: end-to-end like the CLI (request bytes -> response bytes) for every case.
func BenchmarkAllCasesJSON(b *testing.B) {
	ds := testDataset(b)
	files, _ := filepath.Glob("../testdata/requests/*.json")
	var raws [][]byte
	for _, f := range files {
		r, _ := os.ReadFile(f)
		raws = append(raws, r)
	}
	b.ReportAllocs()
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		for _, r := range raws {
			CalcJSON(ds, r)
		}
	}
	b.ReportMetric(float64(b.Elapsed().Microseconds())/float64(b.N*len(raws)), "us/fit")
}

// BenchmarkCase: one named request, e.g. CASE=projfit_svipul_on_vexor go test -bench Case ./dogma
func BenchmarkCase(b *testing.B) {
	name := os.Getenv("CASE")
	if name == "" {
		b.Skip("set CASE=<request name>")
	}
	benchOne(b, name)
}
