package dogma

import (
	"bufio"
	"encoding/json"
	"os"
	"strings"
	"testing"
)

// TestEFTExportMatchesPyfa: ExportEFT reproduces Pyfa's exportEft byte for byte (expected texts frozen by
// Variant A's oracle/pyfa_eft_export.py). T3 cruisers: SDE 3569502 has maxSubSystems 5 vs Pyfa's 4, so one
// extra "[Empty Subsystem slot]" line is tolerated. Every export must also parse back.
func TestEFTExportMatchesPyfa(t *testing.T) {
	ds := testDataset(t)
	fh, err := os.Open("../testdata/oracle/eft_export_expected.jsonl")
	if err != nil {
		t.Skip(err)
	}
	defer fh.Close()
	sc := bufio.NewScanner(fh)
	sc.Buffer(nil, 1<<24)
	n := 0
	for sc.Scan() {
		var e struct {
			File, Name, Text string
			Fit              json.RawMessage
		}
		if err := json.Unmarshal(sc.Bytes(), &e); err != nil {
			t.Fatal(err)
		}
		var req FitRequest
		if err := DecodeRequest(e.Fit, &req); err != nil {
			t.Fatal(e.File, err)
		}
		got := ExportEFT(ds, &req, e.Name)
		if got != e.Text && strings.Replace(got, "\n[Empty Subsystem slot]", "", 1) != e.Text {
			t.Fatalf("%s: export differs\n--- pyfa\n%s\n--- ours\n%s", e.File, e.Text, got)
		}
		back, err := ParseEFT(ds, got)
		if err != nil {
			t.Fatalf("%s: reparse: %v", e.File, err)
		}
		if len(back.Modules) != len(req.Modules) {
			t.Fatalf("%s: %d modules after reparse, want %d", e.File, len(back.Modules), len(req.Modules))
		}
		n++
	}
	if n < 250 {
		t.Fatalf("only %d cases", n)
	}
}
