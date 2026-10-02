package dogma

import (
	"strconv"
	"strings"
)

// Fast, reflection-free decoder for the FitRequest wire format. It is deliberately conservative:
// it handles the common, well-formed shape exactly like encoding/json and gives up (ok=false) on
// anything unusual (escaped or non-ASCII strings, duplicate keys, keys differing from a field
// only by case, type mismatches, malformed JSON, out-of-range numbers). The caller then re-decodes
// with encoding/json, so errors and edge-case semantics are always the reference ones.

type fdec struct {
	b    []byte
	i    int
	fail bool
}

func fastDecodeWire(b []byte, w *wireFit) bool {
	d := fdec{b: b}
	d.ws()
	d.wireFit(w)
	d.ws()
	return !d.fail && d.i == len(d.b)
}

func (d *fdec) bad() { d.fail = true; d.i = len(d.b) }

func (d *fdec) ws() {
	for d.i < len(d.b) {
		switch d.b[d.i] {
		case ' ', '\t', '\n', '\r':
			d.i++
		default:
			return
		}
	}
}

func (d *fdec) peek() byte {
	d.ws()
	if d.i >= len(d.b) {
		d.bad()
		return 0
	}
	return d.b[d.i]
}

// null consumes a null literal if present.
func (d *fdec) null() bool {
	if d.peek() == 'n' {
		if d.i+4 <= len(d.b) && string(d.b[d.i:d.i+4]) == "null" {
			d.i += 4
			return true
		}
		d.bad()
	}
	return false
}

// rawStr returns the bytes of a plain (escape-free, ASCII, no control chars) string.
func (d *fdec) rawStr() []byte {
	if d.peek() != '"' {
		d.bad()
		return nil
	}
	s := d.i + 1
	for j := s; j < len(d.b); j++ {
		c := d.b[j]
		if c == '"' {
			d.i = j + 1
			return d.b[s:j]
		}
		if c < 0x20 || c >= 0x80 || c == '\\' {
			break
		}
	}
	d.bad()
	return nil
}

func (d *fdec) str() string { return string(d.rawStr()) }

// num returns a JSON number token (strict grammar).
func (d *fdec) num() []byte {
	d.ws()
	s, j, n := d.i, d.i, len(d.b)
	if j < n && d.b[j] == '-' {
		j++
	}
	switch {
	case j < n && d.b[j] == '0':
		j++
	case j < n && d.b[j] >= '1' && d.b[j] <= '9':
		for j < n && d.b[j] >= '0' && d.b[j] <= '9' {
			j++
		}
	default:
		d.bad()
		return nil
	}
	if j < n && d.b[j] == '.' {
		j++
		k := j
		for j < n && d.b[j] >= '0' && d.b[j] <= '9' {
			j++
		}
		if j == k {
			d.bad()
			return nil
		}
	}
	if j < n && (d.b[j] == 'e' || d.b[j] == 'E') {
		j++
		if j < n && (d.b[j] == '+' || d.b[j] == '-') {
			j++
		}
		k := j
		for j < n && d.b[j] >= '0' && d.b[j] <= '9' {
			j++
		}
		if j == k {
			d.bad()
			return nil
		}
	}
	d.i = j
	return d.b[s:j]
}

func (d *fdec) u64(bits int) uint64 {
	t := d.num()
	if d.fail {
		return 0
	}
	// plain digits only (encoding/json rejects fractions/exponents/negatives for unsigned fields)
	var v uint64
	if len(t) > 19 {
		d.bad()
		return 0
	}
	for _, c := range t {
		if c < '0' || c > '9' {
			d.bad()
			return 0
		}
		v = v*10 + uint64(c-'0')
	}
	if bits < 64 && v>>uint(bits) != 0 {
		d.bad()
		return 0
	}
	return v
}

func (d *fdec) u32() uint32 { return uint32(d.u64(32)) }

func (d *fdec) f64() float64 {
	t := d.num()
	if d.fail {
		return 0
	}
	v, err := strconv.ParseFloat(string(t), 64)
	if err != nil {
		d.bad()
	}
	return v
}

func (d *fdec) boolean() bool {
	d.ws()
	if d.i+4 <= len(d.b) && string(d.b[d.i:d.i+4]) == "true" {
		d.i += 4
		return true
	}
	if d.i+5 <= len(d.b) && string(d.b[d.i:d.i+5]) == "false" {
		d.i += 5
		return false
	}
	d.bad()
	return false
}

// object iteration: for d.objNext(&first) { k := d.key(); ... }
func (d *fdec) objStart() bool {
	if d.peek() != '{' {
		d.bad()
		return false
	}
	d.i++
	return true
}

func (d *fdec) objNext(first *bool) bool {
	if d.fail {
		return false
	}
	c := d.peek()
	if c == '}' {
		d.i++
		return false
	}
	if !*first {
		if c != ',' {
			d.bad()
			return false
		}
		d.i++
	}
	*first = false
	return true
}

func (d *fdec) key() []byte {
	k := d.rawStr()
	if d.peek() != ':' {
		d.bad()
		return nil
	}
	d.i++
	return k
}

func (d *fdec) arrStart() bool {
	if d.peek() != '[' {
		d.bad()
		return false
	}
	d.i++
	return true
}

func (d *fdec) arrNext(first *bool) bool {
	if d.fail {
		return false
	}
	c := d.peek()
	if c == ']' {
		d.i++
		return false
	}
	if !*first {
		if c != ',' {
			d.bad()
			return false
		}
		d.i++
	}
	*first = false
	return true
}

// seen marks field bit; duplicates make the decoder give up (encoding/json merges objects).
func (d *fdec) seen(mask *uint32, bit uint) {
	if *mask&(1<<bit) != 0 {
		d.bad()
	}
	*mask |= 1 << bit
}

// unknown skips the value of a key that matched no field, unless it matches one case-insensitively.
func (d *fdec) unknown(k []byte, fields string) {
	ks := string(k)
	for f := range strings.SplitSeq(fields, ",") {
		if strings.EqualFold(f, ks) {
			d.bad()
			return
		}
	}
	d.skip(0)
}

func (d *fdec) skip(depth int) {
	if depth > 200 {
		d.bad()
		return
	}
	switch d.peek() {
	case '{':
		d.i++
		first := true
		for d.objNext(&first) {
			d.key()
			d.skip(depth + 1)
		}
	case '[':
		d.i++
		first := true
		for d.arrNext(&first) {
			d.skip(depth + 1)
		}
	case '"':
		d.rawStr()
	case 't', 'f':
		d.boolean()
	case 'n':
		d.null()
	default:
		d.num()
	}
}

func (d *fdec) u32ptr() *uint32 {
	if d.null() {
		return nil
	}
	v := d.u32()
	return &v
}

func (d *fdec) f64ptr() *float64 {
	if d.null() {
		return nil
	}
	v := d.f64()
	return &v
}

func (d *fdec) strptr() *string {
	if d.null() {
		return nil
	}
	v := d.str()
	return &v
}

func (d *fdec) u32s() []uint32 {
	if d.null() {
		return nil
	}
	out := make([]uint32, 0, 4)
	if !d.arrStart() {
		return nil
	}
	first := true
	for d.arrNext(&first) {
		out = append(out, d.u32())
	}
	return out
}

// arr decodes an array into a fresh slice (nil for null), calling el for each element.
func fdecArr[T any](d *fdec, el func(*T)) []T {
	if d.null() {
		return nil
	}
	if !d.arrStart() {
		return nil
	}
	out := make([]T, 0, 4)
	first := true
	for d.arrNext(&first) {
		var x T
		el(&x)
		out = append(out, x)
	}
	return out
}

func (d *fdec) ship(s *ShipReq) {
	if d.null() || !d.objStart() {
		return
	}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "type_id":
			d.seen(&m, 0)
			s.TypeID = d.u32()
		case "mode_type_id":
			d.seen(&m, 1)
			s.ModeTypeID = d.u32ptr()
		default:
			d.unknown(k, "type_id,mode_type_id")
		}
	}
}

func (d *fdec) character(c *Character) {
	if d.null() || !d.objStart() {
		return
	}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "skills":
			d.seen(&m, 0)
			d.skills(&c.Skills)
		case "security_status":
			d.seen(&m, 1)
			c.SecurityStatus = d.f64ptr()
		default:
			d.unknown(k, "skills,security_status")
		}
	}
}

func (d *fdec) skills(s *Skills) {
	if d.null() || !d.objStart() {
		return
	}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "default_level":
			d.seen(&m, 0)
			if d.null() {
				s.DefaultLevel = nil
			} else {
				v := uint8(d.u64(8))
				s.DefaultLevel = &v
			}
		case "levels":
			d.seen(&m, 1)
			if d.null() {
				s.Levels = nil
				continue
			}
			if !d.objStart() {
				return
			}
			s.Levels = make(map[string]uint8, 8)
			f2 := true
			for d.objNext(&f2) {
				name := string(d.key())
				if _, dup := s.Levels[name]; dup {
					d.bad()
				}
				s.Levels[name] = uint8(d.u64(8))
			}
		default:
			d.unknown(k, "default_level,levels")
		}
	}
}

func (d *fdec) mutation() *Mutation {
	if d.null() || !d.objStart() {
		return nil
	}
	mu := &Mutation{}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "base_type_id":
			d.seen(&m, 0)
			mu.BaseTypeID = d.u32()
		case "mutaplasmid_type_id":
			d.seen(&m, 1)
			mu.MutaplasmidTypeID = d.u32ptr()
		case "attributes":
			d.seen(&m, 2)
			if d.null() {
				mu.Attributes = nil
				continue
			}
			if !d.objStart() {
				return nil
			}
			mu.Attributes = make(map[string]float64, 8)
			f2 := true
			for d.objNext(&f2) {
				name := string(d.key())
				if _, dup := mu.Attributes[name]; dup {
					d.bad()
				}
				mu.Attributes[name] = d.f64()
			}
		default:
			d.unknown(k, "base_type_id,mutaplasmid_type_id,attributes")
		}
	}
	return mu
}

func (d *fdec) spool() *Spool {
	if d.null() || !d.objStart() {
		return nil
	}
	s := &Spool{}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "type":
			d.seen(&m, 0)
			if !d.null() {
				s.Type = d.str()
			}
		case "amount":
			d.seen(&m, 1)
			if !d.null() {
				s.Amount = d.f64()
			}
		default:
			d.unknown(k, "type,amount")
		}
	}
	return s
}

func (d *fdec) enumIdx(names []string, from int) int {
	k := d.rawStr()
	for i := from; i < len(names); i++ {
		if string(k) == names[i] {
			return i
		}
	}
	d.bad()
	return 0
}

func (d *fdec) module(mr *ModuleReq) {
	if d.null() || !d.objStart() {
		return
	}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "type_id":
			d.seen(&m, 0)
			mr.TypeID = d.u32()
		case "slot":
			d.seen(&m, 1)
			if d.null() {
				mr.Slot = nil
			} else {
				s := Slot(d.enumIdx(slotNames[:], 1))
				mr.Slot = &s
			}
		case "state":
			d.seen(&m, 2)
			if d.null() {
				mr.State = nil
			} else {
				s := State(d.enumIdx(stateNames[:], 0))
				mr.State = &s
			}
		case "charge_type_id":
			d.seen(&m, 3)
			mr.ChargeTypeID = d.u32ptr()
		case "mutation":
			d.seen(&m, 4)
			mr.Mutation = d.mutation()
		case "spool":
			d.seen(&m, 5)
			mr.Spool = d.spool()
		default:
			d.unknown(k, "type_id,slot,state,charge_type_id,mutation,spool")
		}
	}
}

func (d *fdec) drone(x *wireDrone) {
	if d.null() || !d.objStart() {
		return
	}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "type_id":
			d.seen(&m, 0)
			x.TypeID = d.u32()
		case "quantity":
			d.seen(&m, 1)
			x.Quantity = d.u32ptr()
		case "active":
			d.seen(&m, 2)
			x.Active = d.u32ptr()
		case "mutation":
			d.seen(&m, 3)
			x.Mutation = d.mutation()
		default:
			d.unknown(k, "type_id,quantity,active,mutation")
		}
	}
}

func (d *fdec) fighter(x *wireFighter) {
	if d.null() || !d.objStart() {
		return
	}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "type_id":
			d.seen(&m, 0)
			x.TypeID = d.u32()
		case "quantity":
			d.seen(&m, 1)
			x.Quantity = d.u32ptr()
		case "active":
			d.seen(&m, 2)
			if d.null() {
				x.Active = nil
			} else {
				v := d.boolean()
				x.Active = &v
			}
		case "abilities":
			d.seen(&m, 3)
			if d.null() {
				x.Abilities = nil
			} else {
				v := d.u32s()
				x.Abilities = &v
			}
		default:
			d.unknown(k, "type_id,quantity,active,abilities")
		}
	}
}

func (d *fdec) booster(x *BoosterReq) {
	if d.null() || !d.objStart() {
		return
	}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "type_id":
			d.seen(&m, 0)
			x.TypeID = d.u32()
		case "side_effects":
			d.seen(&m, 1)
			x.SideEffects = d.u32s()
		default:
			d.unknown(k, "type_id,side_effects")
		}
	}
}

func (d *fdec) cargo(x *wireCargo) {
	if d.null() || !d.objStart() {
		return
	}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "type_id":
			d.seen(&m, 0)
			x.TypeID = d.u32()
		case "quantity":
			d.seen(&m, 1)
			x.Quantity = d.u32ptr()
		default:
			d.unknown(k, "type_id,quantity")
		}
	}
}

func (d *fdec) buff(x *Buff) {
	if d.null() || !d.objStart() {
		return
	}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "buff_id":
			d.seen(&m, 0)
			x.BuffID = d.u32()
		case "value":
			d.seen(&m, 1)
			if !d.null() {
				x.Value = d.f64()
			}
		default:
			d.unknown(k, "buff_id,value")
		}
	}
}

func (d *fdec) fleet(x *wireFleet) {
	if d.null() || !d.objStart() {
		return
	}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "buffs":
			d.seen(&m, 0)
			x.Buffs = fdecArr(d, d.buff)
		case "booster_fits":
			d.seen(&m, 1)
			x.BoosterFits = fdecArr(d, d.wireFit)
		default:
			d.unknown(k, "buffs,booster_fits")
		}
	}
}

func (d *fdec) projected(x *wireProjected) {
	if d.null() || !d.objStart() {
		return
	}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "kind":
			d.seen(&m, 0)
			if !d.null() {
				x.Kind = d.str()
			}
		case "module":
			d.seen(&m, 1)
			if d.null() {
				x.Module = nil
			} else {
				x.Module = &ModuleReq{}
				d.module(x.Module)
			}
		case "drone":
			d.seen(&m, 2)
			if d.null() {
				x.Drone = nil
			} else {
				x.Drone = &wireDrone{}
				d.drone(x.Drone)
			}
		case "fit":
			d.seen(&m, 3)
			if d.null() {
				x.Fit = nil
			} else {
				x.Fit = &wireFit{}
				d.wireFit(x.Fit)
			}
		case "fighter":
			d.seen(&m, 4)
			if d.null() {
				x.Fighter = nil
			} else {
				x.Fighter = &wireFighter{}
				d.fighter(x.Fighter)
			}
		case "amount":
			d.seen(&m, 5)
			x.Amount = d.u32ptr()
		case "distance_m":
			d.seen(&m, 6)
			x.DistanceM = d.f64ptr()
		default:
			d.unknown(k, "kind,module,drone,fit,fighter,amount,distance_m")
		}
	}
}

func (d *fdec) environment(x *Environment) {
	if d.null() || !d.objStart() {
		return
	}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "effect_type_ids":
			d.seen(&m, 0)
			x.EffectTypeIDs = d.u32s()
		case "system_security":
			d.seen(&m, 1)
			x.SystemSecurity = d.strptr()
		default:
			d.unknown(k, "effect_type_ids,system_security")
		}
	}
}

// fourDmg decodes em/thermal/kinetic/explosive (+ optional extras via extra).
func (d *fdec) resists() *Resists {
	if d.null() || !d.objStart() {
		return nil
	}
	r := &Resists{}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		var p *float64
		switch string(k) {
		case "em":
			d.seen(&m, 0)
			p = &r.EM
		case "thermal":
			d.seen(&m, 1)
			p = &r.Thermal
		case "kinetic":
			d.seen(&m, 2)
			p = &r.Kinetic
		case "explosive":
			d.seen(&m, 3)
			p = &r.Explosive
		default:
			d.unknown(k, "em,thermal,kinetic,explosive")
			continue
		}
		if !d.null() {
			*p = d.f64()
		}
	}
	return r
}

func (d *fdec) targetProfile() *TargetProfile {
	if d.null() || !d.objStart() {
		return nil
	}
	r := &TargetProfile{}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		var p *float64
		switch string(k) {
		case "em":
			d.seen(&m, 0)
			p = &r.EM
		case "thermal":
			d.seen(&m, 1)
			p = &r.Thermal
		case "kinetic":
			d.seen(&m, 2)
			p = &r.Kinetic
		case "explosive":
			d.seen(&m, 3)
			p = &r.Explosive
		case "signature_radius":
			d.seen(&m, 4)
			r.SignatureRadius = d.f64ptr()
			continue
		case "max_velocity":
			d.seen(&m, 5)
			r.MaxVelocity = d.f64ptr()
			continue
		case "radius":
			d.seen(&m, 6)
			r.Radius = d.f64ptr()
			continue
		default:
			d.unknown(k, "em,thermal,kinetic,explosive,signature_radius,max_velocity,radius")
			continue
		}
		if !d.null() {
			*p = d.f64()
		}
	}
	return r
}

func (d *fdec) override(x *Override) {
	if d.null() || !d.objStart() {
		return
	}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "type_id":
			d.seen(&m, 0)
			x.TypeID = d.u32()
		case "attribute_id":
			d.seen(&m, 1)
			x.AttributeID = d.u32()
		case "value":
			d.seen(&m, 2)
			if !d.null() {
				x.Value = d.f64()
			}
		default:
			d.unknown(k, "type_id,attribute_id,value")
		}
	}
}

func (d *fdec) optBool(p *bool) {
	if !d.null() {
		*p = d.boolean()
	}
}

func (d *fdec) capSim(x *CapSimOpts) {
	if d.null() || !d.objStart() {
		return
	}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "reload":
			d.seen(&m, 0)
			d.optBool(&x.Reload)
		case "stagger":
			d.seen(&m, 1)
			d.optBool(&x.Stagger)
		case "max_time_s":
			d.seen(&m, 2)
			x.MaxTimeS = d.f64ptr()
		default:
			d.unknown(k, "reload,stagger,max_time_s")
		}
	}
}

func (d *fdec) options() *wireOptions {
	if d.null() || !d.objStart() {
		return nil
	}
	o := &wireOptions{}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "nos_no_target_cap":
			d.seen(&m, 0)
			d.optBool(&o.NosNoTargetCap)
		case "factor_reload":
			d.seen(&m, 1)
			d.optBool(&o.FactorReload)
		case "default_spool":
			d.seen(&m, 2)
			o.DefaultSpool = d.spool()
		case "rah":
			d.seen(&m, 3)
			o.Rah = d.strptr()
		case "include_attributes":
			d.seen(&m, 4)
			o.IncludeAttributes = d.strptr()
		case "sources":
			d.seen(&m, 5)
			d.optBool(&o.Sources)
		case "validate":
			d.seen(&m, 6)
			if d.null() {
				o.Validate = nil
			} else {
				v := d.boolean()
				o.Validate = &v
			}
		case "cap_sim":
			d.seen(&m, 7)
			d.capSim(&o.CapSim)
		default:
			d.unknown(k, "nos_no_target_cap,factor_reload,default_spool,rah,include_attributes,sources,validate,cap_sim")
		}
	}
	return o
}

const wireFitFields = "schema_version,ship,character,modules,drones,fighters,implants,boosters,cargo,fleet,projected,environment,damage_pattern,target_profile,overrides,options"

func (d *fdec) wireFit(w *wireFit) {
	if d.null() || !d.objStart() {
		return
	}
	var m uint32
	first := true
	for d.objNext(&first) {
		k := d.key()
		switch string(k) {
		case "schema_version":
			d.seen(&m, 0)
			w.SchemaVersion = d.u32ptr()
		case "ship":
			d.seen(&m, 1)
			if d.null() {
				w.Ship = nil
			} else {
				w.Ship = &ShipReq{}
				d.ship(w.Ship)
			}
		case "character":
			d.seen(&m, 2)
			d.character(&w.Character)
		case "modules":
			d.seen(&m, 3)
			w.Modules = fdecArr(d, d.module)
		case "drones":
			d.seen(&m, 4)
			w.Drones = fdecArr(d, d.drone)
		case "fighters":
			d.seen(&m, 5)
			w.Fighters = fdecArr(d, d.fighter)
		case "implants":
			d.seen(&m, 6)
			w.Implants = d.u32s()
		case "boosters":
			d.seen(&m, 7)
			w.Boosters = fdecArr(d, d.booster)
		case "cargo":
			d.seen(&m, 8)
			w.Cargo = fdecArr(d, d.cargo)
		case "fleet":
			d.seen(&m, 9)
			d.fleet(&w.Fleet)
		case "projected":
			d.seen(&m, 10)
			w.Projected = fdecArr(d, d.projected)
		case "environment":
			d.seen(&m, 11)
			d.environment(&w.Environment)
		case "damage_pattern":
			d.seen(&m, 12)
			w.DamagePattern = d.resists()
		case "target_profile":
			d.seen(&m, 13)
			w.TargetProfile = d.targetProfile()
		case "overrides":
			d.seen(&m, 14)
			w.Overrides = fdecArr(d, d.override)
		case "options":
			d.seen(&m, 15)
			w.Options = d.options()
		default:
			d.unknown(k, wireFitFields)
		}
	}
}
