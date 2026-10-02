package dogma

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"testing"
)

var (
	dsOnce sync.Once
	dsVal  *Dataset
	dsErr  error
)

func testDataset(t testing.TB) *Dataset {
	dsOnce.Do(func() {
		p := os.Getenv("EVE_DOGMA_DATASET")
		if p == "" {
			for _, c := range []string{"../dataset.json.gz", "../../../data/dataset-3569502.json.gz", "/workspace/exct-eve/data/dataset-3569502.json.gz"} {
				if _, err := os.Stat(c); err == nil {
					p = c
					break
				}
			}
		}
		if p == "" {
			dsErr = fmt.Errorf("dataset not found (set EVE_DOGMA_DATASET)")
			return
		}
		dsVal, dsErr = LoadPath(p)
	})
	if dsErr != nil {
		t.Skip(dsErr)
	}
	return dsVal
}

func pointer(v any, ptr string) any {
	// JSON pointer with optional array selector segments `name[key=value]`
	for _, p := range strings.Split(strings.TrimPrefix(ptr, "/"), "/") {
		m, ok := v.(map[string]any)
		if !ok {
			return nil
		}
		b := strings.IndexByte(p, '[')
		if b < 0 || !strings.HasSuffix(p, "]") {
			v = m[p]
			continue
		}
		k, want, ok := strings.Cut(p[b+1:len(p)-1], "=")
		arr, _ := m[p[:b]].([]any)
		if !ok {
			return nil
		}
		v = nil
		for _, e := range arr {
			if em, ok := e.(map[string]any); ok && fmt.Sprint(em[k]) == want {
				v = em
				break
			}
		}
		if v == nil {
			return nil
		}
	}
	return v
}

func close(got, want any) bool {
	switch w := want.(type) {
	case bool:
		switch g := got.(type) {
		case bool:
			return g == w
		case float64:
			return (g != 0) == w
		}
		return false
	case float64:
		switch g := got.(type) {
		case float64:
			return math.Abs(g-w) <= math.Max(1e-3, 1e-4*math.Abs(w))
		case bool:
			return g == (w != 0)
		}
		return false
	}
	return fmt.Sprint(got) == fmt.Sprint(want)
}

type oracleFit struct {
	Eft          string                     `json:"eft"`
	RequestPatch map[string]json.RawMessage `json:"request_patch"`
	Values       map[string]any             `json:"values"`
	CapState     *float64                   `json:"cap_state_percent"`
}

// LoadOracleCase builds the FitRequest for a case of testdata/oracle/pyfa_expected.json.
func loadOracleCase(ds *Dataset, f oracleFit) (*FitRequest, error) {
	text, err := os.ReadFile(filepath.Join("..", "testdata", strings.TrimPrefix(f.Eft, "tests/")))
	if err != nil {
		return nil, err
	}
	req, err := ParseEFT(ds, string(text))
	if err != nil {
		return nil, err
	}
	five := uint8(5)
	req.Character.Skills.DefaultLevel = &five
	if len(f.RequestPatch) > 0 {
		b, _ := json.Marshal(req)
		var m map[string]json.RawMessage
		_ = json.Unmarshal(b, &m)
		for k, v := range f.RequestPatch {
			m[k] = v
		}
		b, _ = json.Marshal(m)
		var r2 FitRequest
		if err := json.Unmarshal(b, &r2); err != nil {
			return nil, err
		}
		req = &r2
	}
	return req, nil
}

func normalize(v any) any {
	var out any
	_ = json.Unmarshal(Marshal(v), &out)
	return out
}

func TestPyfaOracle(t *testing.T) {
	ds := testDataset(t)
	b, err := os.ReadFile("../testdata/oracle/pyfa_expected.json")
	if err != nil {
		t.Fatal(err)
	}
	var exp struct {
		Fits map[string]oracleFit `json:"fits"`
	}
	if err := json.Unmarshal(b, &exp); err != nil {
		t.Fatal(err)
	}
	names := make([]string, 0, len(exp.Fits))
	for n := range exp.Fits {
		names = append(names, n)
	}
	sort.Strings(names)
	var failures []string
	checked := 0
	failedFits := map[string]bool{}
	for _, name := range names {
		f := exp.Fits[name]
		req, err := loadOracleCase(ds, f)
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		st := normalize(Calc(ds, req))
		for ptr, want := range f.Values {
			checked++
			var got any
			if strings.Contains(ptr, "+") {
				s := 0.0
				for _, p := range strings.Split(ptr, "+") {
					if x, ok := pointer(st, p).(float64); ok {
						s += x
					}
				}
				got = s
			} else {
				got = pointer(st, ptr)
			}
			if !close(got, want) {
				failures = append(failures, fmt.Sprintf("%s %s: got %v want %v", name, ptr, got, want))
				failedFits[name] = true
			}
		}
		if f.CapState != nil {
			checked++
			got := pointer(st, "/capacitor/stable_percent")
			if !close(got, *f.CapState) {
				failures = append(failures, fmt.Sprintf("%s cap: got %v want %v", name, got, *f.CapState))
				failedFits[name] = true
			}
		}
	}
	sort.Strings(failures)
	t.Logf("checked %d values over %d fits; %d mismatches in %d fits", checked, len(names), len(failures), len(failedFits))
	if len(failures) > 0 {
		if len(failures) > 80 {
			failures = failures[:80]
		}
		t.Fatalf("mismatches:\n%s", strings.Join(failures, "\n"))
	}
}

func TestDeterministic(t *testing.T) {
	ds := testDataset(t)
	text, _ := os.ReadFile("../testdata/fits/esf_vexor.eft")
	req, err := ParseEFT(ds, string(text))
	if err != nil {
		t.Fatal(err)
	}
	five := uint8(5)
	req.Character.Skills.DefaultLevel = &five
	a, b := Marshal(Calc(ds, req)), Marshal(Calc(ds, req))
	if string(a) != string(b) {
		t.Fatal("non-deterministic output")
	}
}

func TestEFTRoundtripMutations(t *testing.T) {
	ds := testDataset(t)
	text, _ := os.ReadFile("../testdata/fits/esf_mutations.eft")
	req, err := ParseEFT(ds, string(text))
	if err != nil {
		t.Fatal(err)
	}
	if req.Modules[0].Mutation == nil {
		t.Fatal("expected mutation on first module")
	}
	out := ExportEFT(ds, req, "Mutations")
	req2, err := ParseEFT(ds, out)
	if err != nil {
		t.Fatal(err)
	}
	// Pyfa's exporter sorts drones (market group, mutated last), so compare drones as multisets
	dkey := func(ds []DroneReq) string {
		v := make([]string, len(ds))
		for i := range ds {
			v[i] = string(Marshal(ds[i]))
		}
		sort.Strings(v)
		return strings.Join(v, "|")
	}
	if string(Marshal(req.Modules)) != string(Marshal(req2.Modules)) || dkey(req.Drones) != dkey(req2.Drones) {
		t.Fatalf("roundtrip mismatch\n%s", out)
	}
}

// TestFastEncoder: the reflection-free encoder must produce byte-identical output to Tidy + encoding/json.
func TestFastEncoder(t *testing.T) {
	ds := testDataset(t)
	_, reqs := loadRequests(t)
	for i, r := range reqs {
		want := marshalStd(Tidy(calcRaw(ds, r)))
		got := appendJSON(nil, calcRaw(ds, r), true)
		if !bytes.Equal(got, want) {
			t.Fatalf("request %d: fast encoder differs\n got %.300s\nwant %.300s", i, got, want)
		}
	}
	for _, s := range []string{"a<b>&c", "q\"\\\n\t\r\b\f\x01\x7f", "\u2028\u2029é漢", "bad\xffutf8"} {
		if got, want := appendJSON(nil, obj{"s": s, "f": []any{1e-7, 1e21, -0.0, 123.456, 5e-324}}, false), marshalStd(obj{"s": s, "f": []any{1e-7, 1e21, -0.0, 123.456, 5e-324}}); !bytes.Equal(got, want) {
			t.Fatalf("string %q: got %s want %s", s, got, want)
		}
	}
}
