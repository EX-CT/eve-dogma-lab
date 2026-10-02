package dogma

import (
	"encoding/json"
	"math"
	"strconv"
	"unicode/utf8"
)

// Fast encoder for the generic stats tree (obj / []any / scalars). It produces exactly the bytes
// encoding/json would (sorted keys, same float formatting, SetEscapeHTML(false) string escaping), but
// without reflection. With round=true, float64 values inside obj/[]any are rounded like Tidy, so
// CalcJSON skips the separate Tidy pass. Unknown types fall back to encoding/json.

type jsonEnc struct {
	b     []byte
	round bool
	keys  [][]kvPair // per-depth entry scratch (reused)
	depth int
}

func appendJSON(b []byte, v any, round bool) []byte {
	e := jsonEnc{b: b, round: round}
	e.value(v, false)
	return e.b
}

func (e *jsonEnc) value(v any, inTree bool) {
	switch x := v.(type) {
	case nil:
		e.b = append(e.b, "null"...)
	case float64:
		if inTree {
			e.treeFloat(x)
		} else {
			e.float(x)
		}
	case *modRow:
		e.modRow(x)
	case *fobj:
		e.fobj(x)
	case *kobj:
		e.kobj(x)
	case string:
		e.b = appendJSONString(e.b, x)
	case bool:
		e.b = strconv.AppendBool(e.b, x)
	case int:
		e.b = strconv.AppendInt(e.b, int64(x), 10)
	case int32:
		e.b = strconv.AppendInt(e.b, int64(x), 10)
	case int64:
		e.b = strconv.AppendInt(e.b, x, 10)
	case uint8:
		e.b = strconv.AppendUint(e.b, uint64(x), 10)
	case uint32:
		e.b = strconv.AppendUint(e.b, uint64(x), 10)
	case uint64:
		e.b = strconv.AppendUint(e.b, x, 10)
	case obj:
		if x == nil {
			e.b = append(e.b, "null"...)
			return
		}
		e.obj(x)
	case []any:
		if x == nil {
			e.b = append(e.b, "null"...)
			return
		}
		e.b = append(e.b, '[')
		for i, el := range x {
			if i > 0 {
				e.b = append(e.b, ',')
			}
			e.value(el, true)
		}
		e.b = append(e.b, ']')
	case Slot:
		if x == SlotNone || int(x) >= len(slotNames) {
			e.b = append(e.b, "null"...)
		} else {
			e.b = appendJSONString(e.b, slotNames[x])
		}
	case State:
		e.b = appendJSONString(e.b, stateNames[x])
	case json.RawMessage:
		if len(x) == 0 {
			e.b = append(e.b, "null"...)
			return
		}
		e.b = append(e.b, x...) // pre-encoded (e.g. CalcJSON output embedded in an RPC reply)
	case []string:
		if x == nil {
			e.b = append(e.b, "null"...)
			return
		}
		e.b = append(e.b, '[')
		for i, s := range x {
			if i > 0 {
				e.b = append(e.b, ',')
			}
			e.b = appendJSONString(e.b, s)
		}
		e.b = append(e.b, ']')
	default:
		raw, err := json.Marshal(v)
		if err != nil {
			raw = []byte("null")
		}
		// encoding/json escapes HTML in Marshal; re-encode through a compact copy only if needed
		e.b = append(e.b, unescapeHTML(raw)...)
	}
}

type kvPair struct {
	k string
	v any
}

func (e *jsonEnc) obj(m obj) {
	if e.depth >= len(e.keys) {
		e.keys = append(e.keys, nil)
	}
	ks := e.keys[e.depth][:0]
	for k, v := range m {
		ks = append(ks, kvPair{k, v})
	}
	// insertion sort: objects are small (typically < 20 keys) and this avoids sort's indirections
	for i := 1; i < len(ks); i++ {
		for j := i; j > 0 && ks[j].k < ks[j-1].k; j-- {
			ks[j], ks[j-1] = ks[j-1], ks[j]
		}
	}
	e.keys[e.depth] = ks
	e.depth++
	e.b = append(e.b, '{')
	for i := range ks {
		if i > 0 {
			e.b = append(e.b, ',')
		}
		e.b = appendJSONString(e.b, ks[i].k)
		e.b = append(e.b, ':')
		e.value(ks[i].v, true)
	}
	e.b = append(e.b, '}')
	clear(ks) // drop references for the GC
	e.depth--
}

// treeFloat encodes a float inside the stats tree (rounded like Tidy when e.round).
func (e *jsonEnc) treeFloat(x float64) {
	if e.round {
		if math.IsNaN(x) || math.IsInf(x, 0) {
			e.b = append(e.b, "null"...)
			return
		}
		if r := math.Round(x*1e6) / 1e6; !math.IsInf(r, 0) && !math.IsNaN(r) {
			x = r
		}
	}
	e.float(x)
}

// float formats like encoding/json's float64 encoder.
func (e *jsonEnc) float(f float64) {
	if math.IsNaN(f) || math.IsInf(f, 0) {
		e.b = append(e.b, "null"...) // encoding/json would fail; never emitted by the engine
		return
	}
	abs := math.Abs(f)
	fm := byte('f')
	if abs != 0 && (abs < 1e-6 || abs >= 1e21) {
		fm = 'e'
	}
	e.b = strconv.AppendFloat(e.b, f, fm, -1, 64)
	if fm == 'e' {
		n := len(e.b)
		if n >= 4 && e.b[n-4] == 'e' && e.b[n-3] == '-' && e.b[n-2] == '0' {
			e.b[n-2] = e.b[n-1]
			e.b = e.b[:n-1]
		}
	}
}

const hexDigits = "0123456789abcdef"

// appendJSONString escapes like encoding/json with SetEscapeHTML(false).
func appendJSONString(b []byte, s string) []byte {
	plain := true
	for i := 0; i < len(s); i++ {
		if c := s[i]; c < 0x20 || c == '"' || c == '\\' || c >= utf8.RuneSelf {
			plain = false
			break
		}
	}
	if plain { // fast path: printable ASCII (all keys, most values)
		b = append(b, '"')
		b = append(b, s...)
		return append(b, '"')
	}
	b = append(b, '"')
	start := 0
	for i := 0; i < len(s); {
		c := s[i]
		if c < utf8.RuneSelf {
			if c >= 0x20 && c != '"' && c != '\\' {
				i++
				continue
			}
			b = append(b, s[start:i]...)
			switch c {
			case '\\', '"':
				b = append(b, '\\', c)
			case '\b':
				b = append(b, '\\', 'b')
			case '\f':
				b = append(b, '\\', 'f')
			case '\n':
				b = append(b, '\\', 'n')
			case '\r':
				b = append(b, '\\', 'r')
			case '\t':
				b = append(b, '\\', 't')
			default:
				b = append(b, '\\', 'u', '0', '0', hexDigits[c>>4], hexDigits[c&0xf])
			}
			i++
			start = i
			continue
		}
		r, size := utf8.DecodeRuneInString(s[i:])
		if r == utf8.RuneError && size == 1 {
			b = append(b, s[start:i]...)
			b = append(b, `\ufffd`...)
			i += size
			start = i
			continue
		}
		if r == '\u2028' || r == '\u2029' {
			b = append(b, s[start:i]...)
			b = append(b, '\\', 'u', '2', '0', '2', hexDigits[r&0xf])
			i += size
			start = i
			continue
		}
		i += size
	}
	b = append(b, s[start:]...)
	return append(b, '"')
}

// unescapeHTML undoes json.Marshal's \u003c \u003e \u0026 escaping (fallback path only).
func unescapeHTML(b []byte) []byte {
	out := b[:0:0]
	for i := 0; i < len(b); i++ {
		if b[i] == '\\' && i+5 < len(b) && b[i+1] == 'u' && b[i+2] == '0' && b[i+3] == '0' {
			switch string(b[i+4 : i+6]) {
			case "3c":
				out = append(out, '<')
				i += 5
				continue
			case "3e":
				out = append(out, '>')
				i += 5
				continue
			case "26":
				out = append(out, '&')
				i += 5
				continue
			}
		}
		if b[i] == '\\' && i+1 < len(b) {
			out = append(out, b[i], b[i+1])
			i++
			continue
		}
		out = append(out, b[i])
	}
	return out
}
