package dogma

import (
	"fmt"
	"math"
	"sort"
	"strconv"
	"strings"
)

func mutRef(line string) (string, int, bool) {
	l := strings.TrimRight(line, " \t")
	if strings.HasSuffix(l, "]") {
		if p := strings.LastIndex(l, " ["); p >= 0 {
			if n, err := strconv.Atoi(l[p+2 : len(l)-1]); err == nil && n >= 0 {
				return strings.TrimRight(l[:p], " \t"), n, true
			}
		}
	}
	return l, 0, false
}

func isMutHead(l string) bool {
	t := strings.TrimSpace(l)
	if !strings.HasPrefix(t, "[") {
		return false
	}
	e := strings.Index(t, "]")
	if e < 0 {
		return false
	}
	_, err := strconv.ParseUint(t[1:e], 10, 32)
	return err == nil
}

func parseMutations(ds *Dataset, lines []string) (map[int]*Mutation, int, error) {
	out := map[int]*Mutation{}
	first := len(lines)
	for i, l := range lines {
		if isMutHead(l) {
			first = i
			break
		}
	}
	i := first
	for i < len(lines) {
		t := strings.TrimSpace(lines[i])
		if !isMutHead(t) {
			i++
			continue
		}
		e := strings.Index(t, "]")
		n, _ := strconv.Atoi(t[1:e])
		baseName := strings.TrimSpace(t[e+1:])
		base, ok := ds.TypeByName(baseName)
		if !ok {
			return nil, 0, fmt.Errorf("unknown mutated base '%s'", baseName)
		}
		m := &Mutation{BaseTypeID: base, Attributes: map[string]float64{}}
		i++
		for i < len(lines) && !isMutHead(lines[i]) {
			l := strings.TrimSpace(lines[i])
			i++
			if l == "" {
				continue
			}
			if m.MutaplasmidTypeID == nil {
				id, ok := ds.TypeByName(l)
				if !ok {
					return nil, 0, fmt.Errorf("unknown mutaplasmid '%s'", l)
				}
				m.MutaplasmidTypeID = &id
				continue
			}
			for _, kv := range strings.Split(l, ",") {
				kv = strings.TrimSpace(kv)
				if p := strings.LastIndex(kv, " "); p >= 0 {
					aid := ds.AttrID(strings.TrimSpace(kv[:p]))
					if aid != 0 {
						if v, err := strconv.ParseFloat(strings.TrimSpace(kv[p+1:]), 64); err == nil {
							m.Attributes[strconv.FormatUint(uint64(aid), 10)] = v
						}
					}
				}
			}
		}
		out[n] = m
	}
	return out, first, nil
}

func mutatedType(ds *Dataset, m *Mutation) uint32 {
	if m.MutaplasmidTypeID != nil {
		if mu := ds.Mutaplasmids[*m.MutaplasmidTypeID]; mu != nil {
			for _, x := range mu.Mapping {
				if containsU32(x.Inputs, m.BaseTypeID) {
					return x.Output
				}
			}
		}
	}
	return m.BaseTypeID
}

func copyMut(m *Mutation) *Mutation {
	if m == nil {
		return nil
	}
	c := *m
	c.Attributes = map[string]float64{}
	for k, v := range m.Attributes {
		c.Attributes[k] = v
	}
	return &c
}

// ParseEFT converts EFT text into a FitRequest.
func ParseEFT(ds *Dataset, text string) (*FitRequest, error) {
	all := strings.Split(strings.ReplaceAll(text, "\r\n", "\n"), "\n")
	if len(all) > 0 && all[len(all)-1] == "" {
		all = all[:len(all)-1]
	}
	muts, first, err := parseMutations(ds, all)
	if err != nil {
		return nil, err
	}
	var lines []string
	for _, l := range all[:first] {
		if t := strings.TrimSpace(l); t != "" {
			lines = append(lines, t)
		}
	}
	if len(lines) == 0 {
		return nil, fmt.Errorf("empty EFT")
	}
	h := strings.TrimSuffix(strings.TrimPrefix(lines[0], "["), "]")
	shipName := strings.TrimSpace(strings.SplitN(h, ",", 2)[0])
	ship, ok := ds.TypeByName(shipName)
	if !ok {
		return nil, fmt.Errorf("unknown ship '%s'", shipName)
	}
	one := uint32(1)
	req := &FitRequest{SchemaVersion: &one, Ship: ShipReq{TypeID: ship}, Options: Options{Validate: true}}
	for _, line := range lines[1:] {
		if strings.HasPrefix(line, "[Empty") {
			continue
		}
		offline := false
		if l, ok := strings.CutSuffix(line, "/OFFLINE"); ok {
			line, offline = strings.TrimSpace(l), true
		} else if l, ok := strings.CutSuffix(line, "/offline"); ok {
			line, offline = strings.TrimSpace(l), true
		}
		line, n, hasRef := mutRef(line)
		var mutation *Mutation
		if hasRef {
			m, ok := muts[n]
			if !ok {
				return nil, fmt.Errorf("mutation [%d] not defined", n)
			}
			mutation = m
		}
		if pos := strings.LastIndex(line, " x"); pos >= 0 {
			if q, err := strconv.ParseUint(strings.TrimSpace(line[pos+2:]), 10, 32); err == nil {
				name := strings.TrimSpace(line[:pos])
				tid, ok := ds.TypeByName(name)
				if !ok {
					return nil, fmt.Errorf("unknown item '%s'", name)
				}
				if mutation != nil {
					tid = mutatedType(ds, mutation)
				}
				qq := uint32(q)
				switch ds.Types[tid].Category {
				case 18:
					a := qq
					req.Drones = append(req.Drones, DroneReq{TypeID: tid, Quantity: qq, Active: &a, Mutation: copyMut(mutation)})
				case 87:
					req.Fighters = append(req.Fighters, FighterReq{TypeID: tid, Quantity: &qq, Active: true})
				default:
					req.Cargo = append(req.Cargo, CargoReq{TypeID: tid, Quantity: qq})
				}
				continue
			}
		}
		parts := strings.SplitN(line, ",", 2)
		name := strings.TrimSpace(parts[0])
		tid, ok := ds.TypeByName(name)
		if !ok {
			return nil, fmt.Errorf("unknown item '%s'", name)
		}
		if mutation != nil {
			tid = mutatedType(ds, mutation)
		}
		t := ds.Types[tid]
		switch t.Category {
		case 20:
			if _, ok := t.Attr(1087); ok {
				req.Boosters = append(req.Boosters, BoosterReq{TypeID: tid, SideEffects: []uint32{}})
			} else {
				req.Implants = append(req.Implants, tid)
			}
		case 18:
			a := uint32(1)
			req.Drones = append(req.Drones, DroneReq{TypeID: tid, Quantity: 1, Active: &a, Mutation: copyMut(mutation)})
		case 8:
			req.Cargo = append(req.Cargo, CargoReq{TypeID: tid, Quantity: 1})
		default:
			if t.Group == 1306 {
				id := tid
				req.Ship.ModeTypeID = &id
				continue
			}
			slot := t.Slot
			var charge *uint32
			if len(parts) > 1 {
				c := strings.TrimSpace(parts[1])
				cid, ok := ds.TypeByName(c)
				if !ok {
					return nil, fmt.Errorf("unknown charge '%s'", c)
				}
				charge = &cid
			}
			activeCapable := false
			for _, e := range t.Effects {
				if ei := ds.Effects[e.ID]; ei != nil && ei.Category == 1 {
					activeCapable = true
					break
				}
			}
			if v, ok := t.Attr(6); ok && v != 0 {
				activeCapable = true
			}
			st := Online
			if offline {
				st = Offline
			} else if activeCapable && slot != SlotRig && slot != SlotSubsystem {
				st = Active
			}
			var sp *Slot
			if slot != SlotNone {
				s := slot
				sp = &s
			}
			req.Modules = append(req.Modules, ModuleReq{TypeID: tid, Slot: sp, State: &st, ChargeTypeID: charge, Mutation: copyMut(mutation)})
		}
	}
	return req, nil
}

// pyFloat is Python repr() of Pyfa's floatUnerr(v) (7 significant digits), as Pyfa prints mutated values.
func pyFloat(v float64) string {
	if v != 0 && !math.IsInf(v, 0) && !math.IsNaN(v) {
		rf := 7 - int(math.Ceil(math.Log10(math.Abs(v))))
		if rf >= 0 {
			v, _ = strconv.ParseFloat(strconv.FormatFloat(v, 'f', rf, 64), 64)
		} else {
			p := math.Pow(10, float64(-rf))
			v = math.Round(v/p) * p
		}
	}
	if math.IsInf(v, 1) {
		return "inf"
	} else if math.IsInf(v, -1) {
		return "-inf"
	}
	a := math.Abs(v)
	if a != 0 && (a < 1e-4 || a >= 1e16) {
		return strconv.FormatFloat(v, 'e', -1, 64) // Go and Python agree: 1e-05, 1.5e+16
	}
	if v == math.Trunc(v) {
		return strconv.FormatFloat(v, 'f', 1, 64)
	}
	return strconv.FormatFloat(v, 'f', -1, 64)
}

// droneOrder is Pyfa's DRONE_ORDER (service/port/eft.py) by market group id.
func droneOrder(mg *uint32) int {
	if mg == nil {
		return 12
	}
	switch *mg {
	case 837, 1531:
		return 0
	case 3881:
		return 1
	case 838, 1532:
		return 2
	case 3882:
		return 3
	case 839, 359:
		return 4
	case 3883:
		return 5
	case 911, 1533:
		return 6
	case 843, 1586:
		return 7
	case 841, 1029:
		return 8
	case 842, 1030:
		return 9
	case 158, 358:
		return 10
	case 1643, 1646:
		return 11
	}
	return 12
}

var fighterOrder = []string{"Light Fighter", "Structure Light Fighter", "Heavy Fighter", "Structure Heavy Fighter", "Support Fighter", "Structure Support Fighter"}

// ExportEFT renders a FitRequest byte-for-byte like Pyfa's exportEft (all options on, after the GUI's
// fill()): header, blank line, sections joined by two blank lines (racks low/med/high/rig/subsystem/
// service with [Empty X slot] fillers; drones+fighters; implants+boosters; cargo; mutation details),
// sub-sections by one blank line, no trailing newline, no T3D mode line (contract v1.4.1).
func ExportEFT(ds *Dataset, req *FitRequest, name string) string {
	n := func(id uint32) string {
		if t := ds.Types[id]; t != nil {
			return t.Name
		}
		return strconv.FormatUint(uint64(id), 10)
	}
	attr := func(id uint32, a string) float64 {
		if t := ds.Types[id]; t != nil {
			v, _ := t.Attr(ds.AttrID(a))
			return v
		}
		return 0
	}
	groupOf := func(id uint32) *GroupInfo {
		if t := ds.Types[id]; t != nil {
			return ds.Groups[t.Group]
		}
		return nil
	}
	// slot totals after modifiers (subsystems, structure rigs, ...)
	var totals [6]int64
	slotAttrs := [6]string{"lowSlots", "medSlots", "hiSlots", "rigSlots", "maxSubSystems", "serviceSlots"}
	if f, err := Build(ds, req); err == nil {
		for k, a := range slotAttrs {
			totals[k] = int64(f.Get(f.Ship, ds.AttrID(a)))
		}
		f.Release()
	}
	var muts []*Mutation
	var sections []string

	var racks []string
	for k, sl := range []struct {
		s     Slot
		label string
	}{{SlotLow, "Low"}, {SlotMid, "Med"}, {SlotHigh, "High"}, {SlotRig, "Rig"}, {SlotSubsystem, "Subsystem"}, {SlotService, "Service"}} {
		var lines []string
		for mi := range req.Modules {
			m := &req.Modules[mi]
			s := SlotNone
			if m.Slot != nil {
				s = *m.Slot
			} else if t := ds.Types[m.TypeID]; t != nil {
				s = t.Slot
			}
			if s != sl.s {
				continue
			}
			l := n(m.TypeID)
			if m.Mutation != nil {
				l = n(m.Mutation.BaseTypeID)
			}
			mtag := ""
			if m.Mutation != nil && m.Mutation.MutaplasmidTypeID != nil {
				muts = append(muts, m.Mutation)
				mtag = fmt.Sprintf(" [%d]", len(muts))
			}
			if m.ChargeTypeID != nil {
				l += ", " + n(*m.ChargeTypeID)
			}
			if m.State != nil && *m.State == Offline {
				l += " /offline"
			}
			lines = append(lines, l+mtag)
		}
		for free := totals[k] - int64(len(lines)); free > 0; free-- {
			lines = append(lines, "[Empty "+sl.label+" slot]")
		}
		if len(lines) > 0 {
			racks = append(racks, strings.Join(lines, "\n"))
		}
	}
	if len(racks) > 0 {
		sections = append(sections, strings.Join(racks, "\n\n"))
	}

	var minion []string
	type dk struct {
		d     *DroneReq
		order int
		mut   bool
		full  string
	}
	drones := make([]dk, 0, len(req.Drones))
	for i := range req.Drones {
		d := &req.Drones[i]
		base := d.TypeID
		if d.Mutation != nil {
			base = d.Mutation.BaseTypeID
		}
		var mg *uint32
		if t := ds.Types[base]; t != nil {
			mg = t.MarketGroup
		}
		mut := d.Mutation != nil && d.Mutation.MutaplasmidTypeID != nil
		full := n(d.TypeID)
		if mut {
			full = ""
			if t := ds.Types[d.TypeID]; t != nil {
				full = t.Name
			}
		}
		drones = append(drones, dk{d, droneOrder(mg), mut, full})
	}
	sort.SliceStable(drones, func(a, b int) bool {
		x, y := drones[a], drones[b]
		if x.order != y.order {
			return x.order < y.order
		}
		if x.mut != y.mut {
			return !x.mut
		}
		return x.full < y.full
	})
	var dl []string
	for _, x := range drones {
		d := x.d
		base := d.TypeID
		if d.Mutation != nil {
			base = d.Mutation.BaseTypeID
		}
		mtag := ""
		if x.mut {
			muts = append(muts, d.Mutation)
			mtag = fmt.Sprintf(" [%d]", len(muts))
		}
		dl = append(dl, fmt.Sprintf("%s x%d%s", n(base), d.Quantity, mtag))
	}
	if len(dl) > 0 {
		minion = append(minion, strings.Join(dl, "\n"))
	}
	fighters := make([]FighterReq, len(req.Fighters))
	copy(fighters, req.Fighters)
	fpos := func(f FighterReq) int {
		g := ""
		if gi := groupOf(f.TypeID); gi != nil {
			g = gi.Name
		}
		for i, x := range fighterOrder {
			if x == g {
				return i
			}
		}
		return len(fighterOrder)
	}
	sort.SliceStable(fighters, func(a, b int) bool {
		pa, pb := fpos(fighters[a]), fpos(fighters[b])
		if pa != pb {
			return pa < pb
		}
		return n(fighters[a].TypeID) < n(fighters[b].TypeID)
	})
	var fl []string
	for _, f := range fighters {
		mx := uint32(attr(f.TypeID, "fighterSquadronMaxSize"))
		q := mx
		if f.Quantity != nil && *f.Quantity < mx {
			q = *f.Quantity
		}
		fl = append(fl, fmt.Sprintf("%s x%d", n(f.TypeID), q))
	}
	if len(fl) > 0 {
		minion = append(minion, strings.Join(fl, "\n"))
	}
	if len(minion) > 0 {
		sections = append(sections, strings.Join(minion, "\n\n"))
	}

	var charsec []string
	imps := append([]uint32(nil), req.Implants...)
	sort.SliceStable(imps, func(a, b int) bool { return attr(imps[a], "implantness") < attr(imps[b], "implantness") })
	if len(imps) > 0 {
		var l []string
		for _, i := range imps {
			l = append(l, n(i))
		}
		charsec = append(charsec, strings.Join(l, "\n"))
	}
	boos := make([]uint32, 0, len(req.Boosters))
	for _, b := range req.Boosters {
		boos = append(boos, b.TypeID)
	}
	sort.SliceStable(boos, func(a, b int) bool { return attr(boos[a], "boosterness") < attr(boos[b], "boosterness") })
	if len(boos) > 0 {
		var l []string
		for _, i := range boos {
			l = append(l, n(i))
		}
		charsec = append(charsec, strings.Join(l, "\n"))
	}
	if len(charsec) > 0 {
		sections = append(sections, strings.Join(charsec, "\n\n"))
	}

	cargo := append([]CargoReq(nil), req.Cargo...)
	ckey := func(c CargoReq) [3]string {
		var k [3]string
		if g := groupOf(c.TypeID); g != nil {
			k[0], k[1] = ds.Categories[g.Category], g.Name
		}
		k[2] = n(c.TypeID)
		return k
	}
	sort.SliceStable(cargo, func(a, b int) bool {
		x, y := ckey(cargo[a]), ckey(cargo[b])
		for i := range x {
			if x[i] != y[i] {
				return x[i] < y[i]
			}
		}
		return false
	})
	if len(cargo) > 0 {
		var l []string
		for _, c := range cargo {
			l = append(l, fmt.Sprintf("%s x%d", n(c.TypeID), c.Quantity))
		}
		sections = append(sections, strings.Join(l, "\n"))
	}

	if len(muts) > 0 {
		var blocks []string
		for k, m := range muts {
			type kv struct {
				a string
				v float64
			}
			var kvs []kv
			for a, v := range m.Attributes {
				an := a
				if id, err := strconv.ParseUint(a, 10, 32); err == nil {
					if ai := ds.Attrs[uint32(id)]; ai != nil {
						an = ai.Name
					}
				}
				kvs = append(kvs, kv{an, v})
			}
			sort.SliceStable(kvs, func(i, j int) bool { return kvs[i].a < kvs[j].a })
			parts := make([]string, len(kvs))
			for i, x := range kvs {
				parts[i] = x.a + " " + pyFloat(x.v)
			}
			blocks = append(blocks, fmt.Sprintf("[%d] %s\n  %s\n  %s", k+1, n(m.BaseTypeID), n(*m.MutaplasmidTypeID), strings.Join(parts, ", ")))
		}
		sections = append(sections, strings.Join(blocks, "\n"))
	}
	return fmt.Sprintf("[%s, %s]\n\n%s", n(req.Ship.TypeID), name, strings.Join(sections, "\n\n\n"))
}
