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
	return Tidy(calcRaw(ds, req)).(obj)
}

// calcRaw computes the stats tree without rounding (CalcJSON rounds while encoding).
func calcRaw(ds *Dataset, req *FitRequest) obj {
	f, err := Build(ds, req)
	if err != nil {
		if e, ok := err.(*EngineError); ok {
			return obj{"error": obj{"code": e.Code, "message": e.Message, "path": e.Path}}
		}
		return obj{"error": obj{"code": "INTERNAL", "message": err.Error(), "path": ""}}
	}
	out := f.ComputeStats(req, EngineName)
	f.Release()
	return out
}

// CalcJSON: JSON request in, JSON stats out.
func CalcJSON(ds *Dataset, request []byte) []byte {
	var req FitRequest
	var v any
	if err := DecodeRequest(request, &req); err != nil {
		v = obj{"error": obj{"code": "BAD_REQUEST", "message": err.Error(), "path": ""}}
	} else {
		v = calcRaw(ds, &req)
	}
	return appendJSON(make([]byte, 0, 8<<10), v, true) // typical response is ~5 KB
}

// DecodeRequest decodes a FitRequest (same result and errors as json.Unmarshal, but skips
// encoding/json's up-front validation scan when the fast decoder accepts the input).
func DecodeRequest(b []byte, r *FitRequest) error { return r.UnmarshalJSON(b) }

// Marshal encodes without HTML escaping (keys sorted by encoding/json).
func Marshal(v any) []byte { return appendJSON(nil, v, false) }

// marshalStd is the reference encoding/json path (tests compare the fast encoder against it).
func marshalStd(v any) []byte {
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
	case *modRow:
		return Tidy(x.toObj())
	case *fobj:
		return Tidy(x.toObj())
	case *kobj:
		return Tidy(x.toObj())
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
