// eve-dogma-go — stateless CLI: JSON FitRequest in, JSON FitStats out (EX-CT variant C).
package main

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"runtime"
	"slices"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/EX-CT/eve-dogma-lab/variant-c/dogma"
)

const usage = `eve-dogma-go <command> [--dataset PATH] [args]

Commands:
  calc [FILE]            FitRequest JSON (file or stdin) -> FitStats JSON
  batch [-j N]           JSONL FitRequests on stdin -> JSONL FitStats on stdout (order preserved; streams,
                         so it also works as a long-running calc process)
  serve-stdio [-j N]     long-running JSONL RPC (ordered, flushed per reply): {"id":..,"method":"calc|eft_parse|eft_export|search|type|meta","params":..}
  serve-http [-addr :8080]  HTTP: POST /v1/calc, /v1/batch (JSONL), /v1/eft/parse, /v1/eft/export, /v1/rpc;
                         GET /v1/search?q=, /v1/type/{id}, /v1/meta, /healthz
  eft [FILE]             EFT text (file or stdin) -> FitRequest JSON (add --calc to compute, --skills N)
  search QUERY           search types by name (en/zh)
  type ID|NAME           show type with base attributes
  meta                   dataset info
  bench [FILE] [-n N]    time N calculations of a request

Dataset: --dataset PATH, or $EVE_DOGMA_DATASET, or ./dataset.json.gz`

func load(path string) *dogma.Dataset {
	if path == "" {
		path = os.Getenv("EVE_DOGMA_DATASET")
	}
	if path == "" {
		path = "dataset.json.gz"
	}
	ds, err := dogma.LoadPathCached(path) // derived binary cache (EVE_DOGMA_CACHE=off to disable)
	if err != nil {
		fmt.Fprintln(os.Stderr, "error:", err)
		os.Exit(3)
	}
	return ds
}

func readInput(args []string) []byte {
	if len(args) > 0 && args[0] != "-" {
		b, err := os.ReadFile(args[0])
		if err != nil {
			fmt.Fprintf(os.Stderr, "error: %s: %v\n", args[0], err)
			os.Exit(2)
		}
		return b
	}
	b, _ := io.ReadAll(os.Stdin)
	return b
}

// Interim search spec (contract v1.4.1): published types of the scored categories; case-insensitive match on the
// English or Chinese name; rank exact > prefix > substring, ties by type id ascending; default limit 20.
var searchCategories = []struct {
	kind string
	cat  uint32
}{{"ship", 6}, {"module", 7}, {"charge", 8}, {"drone", 18}, {"fighter", 87}, {"implant", 20}, {"subsystem", 32}, {"skill", 16}}

func searchKind(ds *dogma.Dataset, t *dogma.TypeInfo) string {
	if t.Category == 20 {
		if g := ds.Groups[t.Group]; g != nil && strings.Contains(g.Name, "Booster") {
			return "booster"
		}
		return "implant"
	}
	for _, c := range searchCategories {
		if c.cat == t.Category {
			return c.kind
		}
	}
	return ""
}

var matchNames = [...]string{"exact", "prefix", "substring"}

func search(ds *dogma.Dataset, q string, limit int, kinds []string) []any {
	ql := strings.ToLower(strings.TrimSpace(q))
	type hit struct {
		rank int
		id   uint32
		kind string
		t    *dogma.TypeInfo
	}
	var hits []hit
	for id, t := range ds.Types {
		if !t.Published {
			continue
		}
		k := searchKind(ds, t)
		if k == "" || (kinds != nil && !slices.Contains(kinds, k)) {
			continue
		}
		en, zh := strings.ToLower(t.Name), strings.ToLower(ds.NamesZh[id])
		r := -1
		switch {
		case en == ql || (zh != "" && zh == ql):
			r = 0
		case strings.HasPrefix(en, ql) || (zh != "" && strings.HasPrefix(zh, ql)):
			r = 1
		case strings.Contains(en, ql) || (zh != "" && strings.Contains(zh, ql)):
			r = 2
		}
		if r >= 0 {
			hits = append(hits, hit{r, id, k, t})
		}
	}
	sort.Slice(hits, func(i, j int) bool {
		if hits[i].rank != hits[j].rank {
			return hits[i].rank < hits[j].rank
		}
		return hits[i].id < hits[j].id
	})
	out := []any{}
	for k, h := range hits {
		if k >= limit {
			break
		}
		out = append(out, map[string]any{"type_id": h.id, "name": h.t.Name, "name_zh": nilStr(ds.NamesZh[h.id]), "kind": h.kind,
			"match": matchNames[h.rank], "group": groupName(ds, h.t.Group), "category_id": h.t.Category, "meta_level": h.t.MetaLevel, "slot": h.t.Slot})
	}
	return out
}

func nilStr(s string) any {
	if s == "" {
		return nil
	}
	return s
}

func groupName(ds *dogma.Dataset, g uint32) any {
	if gi := ds.Groups[g]; gi != nil {
		return gi.Name
	}
	return nil
}

func typeInfo(ds *dogma.Dataset, key string) any {
	key = strings.Trim(key, "\" ")
	var t *dogma.TypeInfo
	if id, err := strconv.ParseUint(key, 10, 32); err == nil {
		t = ds.Types[uint32(id)]
	} else if id, ok := ds.TypeByName(key); ok {
		t = ds.Types[id]
	}
	if t == nil {
		return map[string]any{"error": map[string]any{"code": "UNKNOWN_TYPE", "message": key}}
	}
	attrs := map[string]any{}
	for _, a := range t.AttrIDs() {
		v, _ := t.Attr(a)
		name := strconv.FormatUint(uint64(a), 10)
		if ai := ds.Attrs[a]; ai != nil {
			name = ai.Name
		}
		attrs[name] = v
	}
	effects := []any{}
	for _, e := range t.Effects {
		var n any
		if ei := ds.Effects[e.ID]; ei != nil {
			n = ei.Name
		}
		effects = append(effects, map[string]any{"id": e.ID, "name": n, "default": e.Default})
	}
	return map[string]any{"type_id": t.ID, "name": t.Name, "name_zh": nilStr(ds.NamesZh[t.ID]), "group": groupName(ds, t.Group),
		"group_id": t.Group, "category_id": t.Category, "published": t.Published, "mass": t.Mass, "volume": t.Volume,
		"capacity": t.Capacity, "slot": t.Slot, "attributes": attrs, "effects": effects}
}

func meta(ds *dogma.Dataset) map[string]any {
	return map[string]any{"engine": dogma.EngineName, "schema_version": 1, "sde_build": ds.Build, "sde_release_date": ds.ReleaseDate,
		"dataset_sha256": ds.SHA256, "types": len(ds.Types), "attributes": len(ds.Attrs), "effects": len(ds.Effects)}
}

func errObj(code, msg string) map[string]any {
	return map[string]any{"error": map[string]any{"code": code, "message": msg}}
}

func rpc(ds *dogma.Dataset, line []byte) any {
	var v struct {
		ID     any             `json:"id"`
		Method *string         `json:"method"`
		Params json.RawMessage `json:"params"`
	}
	if err := json.Unmarshal(line, &v); err != nil {
		return map[string]any{"id": nil, "error": map[string]any{"code": "BAD_JSON", "message": err.Error()}}
	}
	method := "calc"
	if v.Method != nil {
		method = *v.Method
	}
	var p map[string]json.RawMessage
	if method != "calc" { // calc passes params through untouched
		_ = json.Unmarshal(v.Params, &p)
	}
	str := func(k string) string {
		var s string
		_ = json.Unmarshal(p[k], &s)
		return s
	}
	var result any
	switch method {
	case "calc":
		// CalcJSON output is embedded verbatim (no second encode pass)
		result = json.RawMessage(calcJSON(ds, v.Params))
	case "eft_parse":
		r, err := dogma.ParseEFT(ds, str("text"))
		if err != nil {
			result = errObj("EFT_PARSE", err.Error())
		} else {
			result = r
		}
	case "eft_export":
		var r dogma.FitRequest
		if err := dogma.DecodeRequest(p["fit"], &r); err != nil {
			result = errObj("BAD_REQUEST", err.Error())
		} else {
			name := str("name")
			if name == "" {
				name = "EXCT fit"
			}
			result = map[string]any{"text": dogma.ExportEFT(ds, &r, name)}
		}
	case "search":
		limit := 20
		_ = json.Unmarshal(p["limit"], &limit)
		var kinds []string
		if k, ok := p["kinds"]; ok && string(k) != "null" {
			var raw []any
			_ = json.Unmarshal(k, &raw)
			kinds = []string{}
			for _, x := range raw {
				if s, ok := x.(string); ok {
					kinds = append(kinds, s)
				}
			}
		}
		result = search(ds, str("query"), limit, kinds)
	case "type":
		result = typeInfo(ds, string(p["id"]))
	case "meta":
		result = meta(ds)
	default:
		result = errObj("UNKNOWN_METHOD", method)
	}
	return map[string]any{"id": v.ID, "result": result}
}

func takeFlag(args *[]string, f string) (string, bool) {
	for i, a := range *args {
		if a == f {
			v := ""
			if i+1 < len(*args) {
				v = (*args)[i+1]
			}
			*args = append((*args)[:i], (*args)[min(i+2, len(*args)):]...)
			return v, true
		}
	}
	return "", false
}

func takeBool(args *[]string, f string) bool {
	for i, a := range *args {
		if a == f {
			*args = append((*args)[:i], (*args)[i+1:]...)
			return true
		}
	}
	return false
}

func main() {
	args := os.Args[1:]
	dsPath, _ := takeFlag(&args, "--dataset")
	cmd := ""
	if len(args) > 0 {
		cmd = args[0]
	}
	out := bufio.NewWriterSize(os.Stdout, 1<<16)
	defer out.Flush()
	switch cmd {
	case "calc":
		ds := load(dsPath)
		res := dogma.CalcJSON(ds, readInput(args[1:]))
		out.Write(res)
		out.WriteByte('\n')
		if strings.HasPrefix(string(res), `{"error"`) {
			out.Flush()
			os.Exit(2)
		}
	case "batch":
		jv, _ := takeFlag(&args, "-j")
		j, _ := strconv.Atoi(jv)
		if j <= 0 {
			j = runtime.NumCPU()
		}
		ds := load(dsPath)
		batch(ds, os.Stdin, out, j)
	case "serve-stdio":
		ds := load(dsPath)
		enableMemo()
		fmt.Fprintf(os.Stderr, "eve-dogma-go serve-stdio ready (sde %d)\n", ds.Build)
		jv, _ := takeFlag(&args, "-j")
		j, _ := strconv.Atoi(jv)
		if j <= 0 {
			j = runtime.NumCPU()
		}
		pipeline(os.Stdin, out, j, func(line []byte) []byte { return dogma.Marshal(rpc(ds, line)) })
	case "serve-http":
		addr, ok := takeFlag(&args, "-addr")
		if !ok {
			addr = ":8080"
		}
		ds := load(dsPath)
		out.Flush()
		enableMemo()
		serveHTTP(ds, addr)
	case "eft":
		skills, hasSkills := takeFlag(&args, "--skills")
		doCalc := takeBool(&args, "--calc")
		ds := load(dsPath)
		r, err := dogma.ParseEFT(ds, string(readInput(args[1:])))
		if err != nil {
			fmt.Fprintln(os.Stderr, "error:", err)
			out.Flush()
			os.Exit(2)
		}
		if hasSkills {
			if l, err := strconv.Atoi(skills); err == nil {
				lv := uint8(l)
				r.Character.Skills.DefaultLevel = &lv
			}
		}
		var v any = r
		if doCalc {
			v = dogma.Calc(ds, r)
		}
		b, _ := json.MarshalIndent(v, "", "  ")
		out.Write(b)
		out.WriteByte('\n')
	case "search":
		ds := load(dsPath)
		b, _ := json.MarshalIndent(search(ds, strings.Join(args[1:], " "), 20, nil), "", "  ")
		out.Write(b)
		out.WriteByte('\n')
	case "type":
		ds := load(dsPath)
		b, _ := json.MarshalIndent(typeInfo(ds, strings.Join(args[1:], " ")), "", "  ")
		out.Write(b)
		out.WriteByte('\n')
	case "meta":
		t0 := time.Now()
		ds := load(dsPath)
		loadMs := float64(time.Since(t0).Microseconds()) / 1000
		m := meta(ds)
		m["load_ms"] = loadMs
		b, _ := json.MarshalIndent(m, "", "  ")
		out.Write(b)
		out.WriteByte('\n')
	case "bench":
		nv, _ := takeFlag(&args, "-n")
		n, err := strconv.Atoi(nv)
		if err != nil || n <= 0 {
			n = 1000
		}
		t0 := time.Now()
		ds := load(dsPath)
		loadMs := float64(time.Since(t0).Microseconds()) / 1000
		raw := readInput(args[1:])
		var req dogma.FitRequest
		if err := dogma.DecodeRequest(raw, &req); err != nil {
			fmt.Fprintln(os.Stderr, "bad request:", err)
			os.Exit(2)
		}
		_ = dogma.CalcJSON(ds, raw)
		t1 := time.Now()
		for k := 0; k < n; k++ {
			_ = dogma.CalcJSON(ds, raw) // production path: decode + calc + encode
		}
		el := time.Since(t1).Seconds()
		out.Write(dogma.Marshal(map[string]any{"dataset_load_ms": loadMs, "iterations": n, "total_s": el, "per_calc_us": el / float64(n) * 1e6}))
		out.WriteByte('\n')
	default:
		fmt.Fprintln(os.Stderr, usage)
		out.Flush()
		if cmd == "" || cmd == "help" || cmd == "--help" {
			os.Exit(0)
		}
		os.Exit(2)
	}
}

// batch computes JSONL requests with j workers, preserving input order.
func batch(ds *dogma.Dataset, in io.Reader, out *bufio.Writer, j int) {
	pipeline(in, out, j, func(line []byte) []byte { return dogma.CalcJSON(ds, line) })
}

// pipeline is the long-running JSONL engine behind batch and serve-stdio: lines are processed by j
// workers, replies are written in input order, and output is flushed whenever no further reply is
// pending, so an interactive client (MCP server, editor plugin, web backend) gets each answer as
// soon as it is ready while a bulk stream is still written in large chunks.
func pipeline(in io.Reader, out *bufio.Writer, j int, fn func([]byte) []byte) {
	sc := bufio.NewScanner(in)
	sc.Buffer(make([]byte, 1<<20), 64<<20)
	type job struct {
		line []byte
		res  chan []byte
	}
	jobs := make(chan job, j*4)
	order := make(chan chan []byte, j*16)
	var wg sync.WaitGroup
	for w := 0; w < j; w++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for jb := range jobs {
				jb.res <- fn(jb.line)
			}
		}()
	}
	done := make(chan struct{})
	go func() {
		for ch := range order {
			out.Write(<-ch)
			out.WriteByte('\n')
			if len(order) == 0 {
				out.Flush()
			}
		}
		out.Flush()
		close(done)
	}()
	for sc.Scan() {
		line := sc.Bytes()
		if len(bytes.TrimSpace(line)) == 0 {
			continue
		}
		jb := job{append([]byte(nil), line...), make(chan []byte, 1)}
		order <- jb.res
		jobs <- jb
	}
	close(jobs)
	close(order)
	wg.Wait()
	<-done
}

func serveHTTP(ds *dogma.Dataset, addr string) {
	mux := http.NewServeMux()
	writeJSON := func(w http.ResponseWriter, code int, v []byte) {
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(code)
		w.Write(v)
		w.Write([]byte("\n"))
	}
	body := func(r *http.Request) []byte {
		b, _ := io.ReadAll(io.LimitReader(r.Body, 32<<20))
		return b
	}
	mux.HandleFunc("POST /v1/calc", func(w http.ResponseWriter, r *http.Request) {
		res := calcJSON(ds, body(r))
		code := 200
		if strings.HasPrefix(string(res), `{"error"`) {
			code = 400
		}
		writeJSON(w, code, res)
	})
	mux.HandleFunc("POST /v1/rpc", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, 200, dogma.Marshal(rpc(ds, body(r))))
	})
	mux.HandleFunc("POST /v1/eft/parse", func(w http.ResponseWriter, r *http.Request) {
		req, err := dogma.ParseEFT(ds, string(body(r)))
		if err != nil {
			writeJSON(w, 400, dogma.Marshal(errObj("EFT_PARSE", err.Error())))
			return
		}
		writeJSON(w, 200, dogma.Marshal(req))
	})
	mux.HandleFunc("POST /v1/eft/export", func(w http.ResponseWriter, r *http.Request) {
		var req dogma.FitRequest
		if err := dogma.DecodeRequest(body(r), &req); err != nil {
			writeJSON(w, 400, dogma.Marshal(errObj("BAD_REQUEST", err.Error())))
			return
		}
		name := r.URL.Query().Get("name")
		if name == "" {
			name = "EXCT fit"
		}
		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
		io.WriteString(w, dogma.ExportEFT(ds, &req, name))
	})
	mux.HandleFunc("GET /v1/search", func(w http.ResponseWriter, r *http.Request) {
		limit, err := strconv.Atoi(r.URL.Query().Get("limit"))
		if err != nil || limit <= 0 {
			limit = 20
		}
		var kinds []string
		if k := r.URL.Query().Get("kinds"); k != "" {
			kinds = strings.Split(k, ",")
		}
		writeJSON(w, 200, dogma.Marshal(search(ds, r.URL.Query().Get("q"), limit, kinds)))
	})
	mux.HandleFunc("GET /v1/type/{id}", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, 200, dogma.Marshal(typeInfo(ds, r.PathValue("id"))))
	})
	mux.HandleFunc("GET /v1/meta", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, 200, dogma.Marshal(meta(ds)))
	})
	mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, 200, []byte(`{"ok":true}`))
	})
	// POST /v1/batch: JSONL requests -> JSONL responses (same order), computed on all cores
	mux.HandleFunc("POST /v1/batch", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/x-ndjson")
		// read the whole body first: net/http stops reading the request once the response is being written
		// (HTTP/1.1 is not full duplex by default), which used to truncate large batches
		in, err := io.ReadAll(io.LimitReader(r.Body, 256<<20))
		if err != nil {
			writeJSON(w, 400, dogma.Marshal(errObj("BAD_REQUEST", err.Error())))
			return
		}
		bw := bufio.NewWriterSize(w, 64<<10)
		batch(ds, bytes.NewReader(in), bw, runtime.NumCPU())
	})
	fmt.Fprintf(os.Stderr, "eve-dogma-go serve-http on %s (sde %d)\n", addr, ds.Build)
	srv := &http.Server{Addr: addr, Handler: mux, ReadHeaderTimeout: 10 * time.Second, IdleTimeout: 120 * time.Second}
	if err := srv.ListenAndServe(); err != nil {
		fmt.Fprintln(os.Stderr, "error:", err)
		os.Exit(1)
	}
}
