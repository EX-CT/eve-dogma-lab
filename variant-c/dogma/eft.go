package dogma

import (
	"fmt"
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

// ExportEFT renders a FitRequest as EFT text (with mutation blocks).
func ExportEFT(ds *Dataset, req *FitRequest, name string) string {
	n := func(id uint32) string {
		if t := ds.Types[id]; t != nil {
			return t.Name
		}
		return strconv.FormatUint(uint64(id), 10)
	}
	var b strings.Builder
	fmt.Fprintf(&b, "[%s, %s]\n", n(req.Ship.TypeID), name)
	var muts []*Mutation
	tag := func(m *Mutation) string {
		if m == nil {
			return ""
		}
		muts = append(muts, m)
		return fmt.Sprintf(" [%d]", len(muts))
	}
	for _, slot := range []Slot{SlotLow, SlotMid, SlotHigh, SlotRig, SlotSubsystem, SlotService} {
		any := false
		for k := range req.Modules {
			m := &req.Modules[k]
			s := SlotNone
			if m.Slot != nil {
				s = *m.Slot
			} else if t := ds.Types[m.TypeID]; t != nil {
				s = t.Slot
			}
			if s != slot {
				continue
			}
			any = true
			if m.Mutation != nil {
				b.WriteString(n(m.Mutation.BaseTypeID))
			} else {
				b.WriteString(n(m.TypeID))
			}
			if m.ChargeTypeID != nil {
				b.WriteString(", " + n(*m.ChargeTypeID))
			}
			if m.State != nil && *m.State == Offline {
				b.WriteString(" /OFFLINE")
			}
			b.WriteString(tag(m.Mutation) + "\n")
		}
		if any {
			b.WriteString("\n")
		}
	}
	for _, d := range req.Drones {
		nm := d.TypeID
		if d.Mutation != nil {
			nm = d.Mutation.BaseTypeID
		}
		fmt.Fprintf(&b, "%s x%d%s\n", n(nm), d.Quantity, tag(d.Mutation))
	}
	for _, f := range req.Fighters {
		q := uint32(1)
		if f.Quantity != nil {
			q = *f.Quantity
		}
		fmt.Fprintf(&b, "%s x%d\n", n(f.TypeID), q)
	}
	if len(req.Implants) > 0 || len(req.Boosters) > 0 {
		b.WriteString("\n")
		for _, i := range req.Implants {
			b.WriteString(n(i) + "\n")
		}
		for _, x := range req.Boosters {
			b.WriteString(n(x.TypeID) + "\n")
		}
	}
	if len(req.Cargo) > 0 {
		b.WriteString("\n")
		for _, c := range req.Cargo {
			fmt.Fprintf(&b, "%s x%d\n", n(c.TypeID), c.Quantity)
		}
	}
	if len(muts) > 0 {
		b.WriteString("\n")
		for k, m := range muts {
			fmt.Fprintf(&b, "[%d] %s\n", k+1, n(m.BaseTypeID))
			if m.MutaplasmidTypeID != nil {
				fmt.Fprintf(&b, "  %s\n", n(*m.MutaplasmidTypeID))
			}
			keys := make([]string, 0, len(m.Attributes))
			for a := range m.Attributes {
				keys = append(keys, a)
			}
			sort.Strings(keys)
			var kv []string
			for _, a := range keys {
				an := a
				if ai := ds.Attrs[u32(a)]; ai != nil {
					an = ai.Name
				}
				kv = append(kv, fmt.Sprintf("%s %s", an, strconv.FormatFloat(m.Attributes[a], 'g', -1, 64)))
			}
			if len(kv) > 0 {
				fmt.Fprintf(&b, "  %s\n", strings.Join(kv, ", "))
			}
		}
	}
	return b.String()
}
