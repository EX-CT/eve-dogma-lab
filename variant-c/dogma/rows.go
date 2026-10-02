package dogma

import "strconv"

// modRow is a typed output row for offense-free module data (stats.modules[]). It avoids a map and
// ~9 interface boxes per module. The fast encoder writes it directly (keys in sorted order, same
// bytes as the equivalent obj); Tidy converts it back to an obj for the map-returning Calc API.
type modRow struct {
	idx        int // -1 = null
	typeID     uint32
	name       string
	slot       Slot
	state      State
	cpu, power float64
	cycleMs    float64
	capUse     float64
	hasCycle   bool
	hasCapUse  bool
}

func (r *modRow) toObj() obj {
	o := obj{"module_index": optIdx(r.idx), "type_id": r.typeID, "name": r.name, "slot": r.slot,
		"state": r.state, "cpu": r.cpu, "power": r.power}
	if r.hasCycle {
		o["cycle_time_ms"] = r.cycleMs
	}
	if r.hasCapUse {
		o["cap_use_gj_s"] = r.capUse
	}
	return o
}

func (e *jsonEnc) modRow(r *modRow) {
	b := append(e.b, '{')
	if r.hasCapUse {
		e.b = append(b, `"cap_use_gj_s":`...)
		e.treeFloat(r.capUse)
		b = append(e.b, ',')
	}
	e.b = append(b, `"cpu":`...)
	e.treeFloat(r.cpu)
	if r.hasCycle {
		e.b = append(e.b, `,"cycle_time_ms":`...)
		e.treeFloat(r.cycleMs)
	}
	e.b = append(e.b, `,"module_index":`...)
	if r.idx < 0 {
		e.b = append(e.b, "null"...)
	} else {
		e.b = strconv.AppendInt(e.b, int64(r.idx), 10)
	}
	e.b = append(e.b, `,"name":`...)
	e.b = appendJSONString(e.b, r.name)
	e.b = append(e.b, `,"power":`...)
	e.treeFloat(r.power)
	e.b = append(e.b, `,"slot":`...)
	e.value(r.slot, true)
	e.b = append(e.b, `,"state":`...)
	e.value(r.state, true)
	e.b = append(e.b, `,"type_id":`...)
	e.b = strconv.AppendUint(e.b, uint64(r.typeID), 10)
	e.b = append(e.b, '}')
}

// fobj is a small all-float object with a fixed, pre-sorted key set: one allocation instead of a
// map plus one interface box per value. keys must be sorted (the encoder writes them in order).
type fobj struct {
	k []string
	v [5]float64
}

var (
	keysDmg   = []string{"em", "explosive", "kinetic", "thermal", "total"}
	keysRes   = []string{"em", "explosive", "kinetic", "thermal"}
	keysUsage = []string{"total", "used"}
	keysLayer = []string{"armor", "hull", "shield", "total"}
)

func (o *fobj) toObj() obj {
	m := make(obj, len(o.k))
	for i, k := range o.k {
		m[k] = o.v[i]
	}
	return m
}

func (e *jsonEnc) fobj(o *fobj) {
	e.b = append(e.b, '{')
	for i, k := range o.k {
		if i > 0 {
			e.b = append(e.b, ',')
		}
		e.b = appendJSONString(e.b, k)
		e.b = append(e.b, ':')
		e.treeFloat(o.v[i])
	}
	e.b = append(e.b, '}')
}
