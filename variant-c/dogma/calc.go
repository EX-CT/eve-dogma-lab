package dogma

import (
	"bytes"
	"encoding/json"
	"math"
)

// EngineName identifies this implementation in FitStats.meta.engine.
const EngineName = "eve-dogma-go 0.1.0 (variant-c)"

// Calc computes full fit statistics for one request (pure function).
func Calc(ds *Dataset, req *FitRequest) map[string]any {
	f, err := Build(ds, req)
	if err != nil {
		if e, ok := err.(*EngineError); ok {
			return obj{"error": obj{"code": e.Code, "message": e.Message, "path": e.Path}}
		}
		return obj{"error": obj{"code": "INTERNAL", "message": err.Error(), "path": ""}}
	}
	return Tidy(f.ComputeStats(req, EngineName)).(obj)
}

// CalcJSON: JSON request in, JSON stats out.
func CalcJSON(ds *Dataset, request []byte) []byte {
	var req FitRequest
	var v any
	if err := json.Unmarshal(request, &req); err != nil {
		v = obj{"error": obj{"code": "BAD_REQUEST", "message": err.Error(), "path": ""}}
	} else {
		v = Calc(ds, &req)
	}
	return Marshal(v)
}

// Marshal encodes without HTML escaping (keys sorted by encoding/json).
func Marshal(v any) []byte {
	var b bytes.Buffer
	enc := json.NewEncoder(&b)
	enc.SetEscapeHTML(false)
	_ = enc.Encode(v)
	return bytes.TrimRight(b.Bytes(), "\n")
}

func round6(v float64) any {
	if math.IsNaN(v) || math.IsInf(v, 0) {
		return nil
	}
	r := math.Round(v*1e6) / 1e6
	if math.IsInf(r, 0) || math.IsNaN(r) {
		return v
	}
	return r
}

// Tidy rounds every float to 1e-6 and maps non-finite values to null (stable, readable output).
func Tidy(v any) any {
	switch x := v.(type) {
	case float64:
		return round6(x)
	case obj:
		for k, e := range x {
			x[k] = Tidy(e)
		}
		return x
	case []any:
		for i, e := range x {
			x[i] = Tidy(e)
		}
		return x
	}
	return v
}
