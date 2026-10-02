package dogma

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

// checkFastDecode: whenever the fast decoder accepts input, encoding/json must accept it and
// produce a deeply equal wireFit.
func checkFastDecode(t *testing.T, src []byte) bool {
	var fast wireFit
	if !fastDecodeWire(src, &fast) {
		return false
	}
	var ref wireFit
	if err := json.Unmarshal(src, &ref); err != nil {
		t.Fatalf("fast decoder accepted input rejected by encoding/json (%v): %.200s", err, src)
	}
	if !reflect.DeepEqual(fast, ref) {
		t.Fatalf("fast decode differs for %.200s\nfast %+v\nref  %+v", src, fast, ref)
	}
	return true
}

func fastDecodeSeeds(t testing.TB) [][]byte {
	files, _ := filepath.Glob("../testdata/requests/*.json")
	var out [][]byte
	for _, f := range files {
		b, _ := os.ReadFile(f)
		out = append(out, b)
	}
	for _, s := range []string{
		`{"ship":{"type_id":1},"drones":[{"type_id":2}],"fighters":[{"type_id":3,"active":null,"abilities":[]}],"cargo":[{"type_id":4}]}`,
		`{"ship":{"type_id":1},"options":null,"character":null,"modules":[null,{"type_id":5,"slot":null,"state":"overheated"}]}`,
		`{"ship":{"type_id":1},"unknown":{"a":[1,2,{"b":null}],"c":"x"},"implants":[]}`,
		`{"ship":{"type_id":1},"Ship":{"type_id":2}}`,
		`{"ship":{"type_id":1},"ship":{"type_id":2}}`,
		`{"ship":{"type_id":1.5}}`, `{"ship":{"type_id":-1}}`, `{"ship":{"type_id":4294967296}}`,
		`{"ship":{"type_id":1}} x`, `{"ship":{"type_id":1},}`, `{"ship":{"type_id":01}}`,
		`{"ship":{"type_id":1},"character":{"skills":{"default_level":256}}}`,
		`{"ship":{"type_id":1},"character":{"skills":{"levels":{"Gunnery":5,"Gunnery":4}}}}`,
		`{"ship":{"type_id":1},"modules":[{"type_id":2,"slot":"HIGH"}]}`,
		`{"ship":{"type_id":1},"projected":[{"kind":"fit","fit":{"ship":{"type_id":2}},"amount":null,"distance_m":1e3}]}`,
		`{"ship":{"type_id":1},"modules":[{"type_id":2,"mutation":{"base_type_id":3,"attributes":{"cpu":1.5}}}]}`,
		`{"ship":{"type_id":1},"options":{"validate":false,"cap_sim":{"reload":true,"max_time_s":null},"rah":"x\u0041"}}`,
		`null`, ``, `{}`, `[]`, `{"ship":null}`,
	} {
		out = append(out, []byte(s))
	}
	return out
}

func TestFastDecoderMatchesEncodingJSON(t *testing.T) {
	accepted := 0
	for _, src := range fastDecodeSeeds(t) {
		if checkFastDecode(t, src) {
			accepted++
		}
	}
	files, _ := filepath.Glob("../testdata/requests/*.json")
	if accepted < len(files) {
		t.Fatalf("fast decoder accepted only %d inputs (want all %d request files)", accepted, len(files))
	}
}

func FuzzFastDecoder(f *testing.F) {
	for _, s := range fastDecodeSeeds(f) {
		f.Add(s)
	}
	f.Fuzz(func(t *testing.T, src []byte) { checkFastDecode(t, src) })
}
