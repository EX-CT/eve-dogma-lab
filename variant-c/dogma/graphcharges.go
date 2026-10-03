package dogma

import "sort"

// Application-profile support (round 2, approach G2): the fit's dominant weapon group and, for every charge
// that group can load, the full primitive set of the fit re-built with that charge in all of those weapons.
// The portable evaluator evaluates the damage model once per charge variant and keeps the best per point.

// dominantWeaponGroup: the module group with the most charge-using weapon modules (ties: lower first
// module index). Returns the group id and the request indices of its modules.
func dominantWeaponGroup(f *Fit) (uint32, []int) {
	count := map[uint32][]int{}
	var order []uint32
	for i := range f.Items {
		it := &f.Items[i]
		if it.Kind != KModule || it.ReqIndex < 0 || it.State < Active {
			continue
		}
		if _, kind := f.moduleVolley(i); kind == "" {
			continue
		}
		if !moduleTakesCharges(f.DS, it.T) {
			continue
		}
		g := it.T.Group
		if _, ok := count[g]; !ok {
			order = append(order, g)
		}
		count[g] = append(count[g], int(it.ReqIndex))
	}
	best := uint32(0)
	for _, g := range order {
		if best == 0 || len(count[g]) > len(count[best]) {
			best = g
		}
	}
	return best, count[best]
}

func moduleTakesCharges(ds *Dataset, t *TypeInfo) bool {
	for k := 1; k <= 5; k++ {
		if x, ok := t.Attr(ds.AttrID(chargeGroupNames[k-1])); ok && uint32(x) != 0 {
			return true
		}
	}
	return false
}

// validCharges: published, on-market charges of the module's charge groups with matching size and volume.
func validCharges(ds *Dataset, mt *TypeInfo) []uint32 {
	var out []uint32
	seen := map[uint32]bool{}
	ms, hasSize := mt.Attr(ds.AttrID("chargeSize"))
	for k := 1; k <= 5; k++ {
		x, ok := mt.Attr(ds.AttrID(chargeGroupNames[k-1]))
		if !ok || uint32(x) == 0 || seen[uint32(x)] {
			continue
		}
		seen[uint32(x)] = true
		for _, cid := range ds.TypesInGroup(uint32(x)) {
			ct := ds.typ(cid)
			if ct == nil || !ct.Published || ct.MarketGroup == nil {
				continue
			}
			if cs, ok := ct.Attr(ds.AttrID("chargeSize")); hasSize && ok && cs != ms {
				continue
			}
			if mt.Capacity > 0 && ct.Volume > mt.Capacity {
				continue
			}
			out = append(out, cid)
		}
	}
	sort.Slice(out, func(a, b int) bool { return out[a] < out[b] })
	return out
}

// chargeVariants builds the application-profile primitive block.
func chargeVariants(ds *Dataset, req FitRequest) obj {
	f, err := Build(ds, &req)
	if err != nil {
		return nil
	}
	g, idx := dominantWeaponGroup(f)
	var mt *TypeInfo
	if len(idx) > 0 {
		mt = ds.typ(req.Modules[idx[0]].TypeID)
	}
	f.Release()
	if mt == nil {
		return obj{"group": 0, "modules": []any{}, "variants": []any{}}
	}
	mods := []any{}
	for _, i := range idx {
		mods = append(mods, i)
	}
	vars := []any{}
	for _, cid := range validCharges(ds, mt) {
		r := req
		r.Modules = make([]ModuleReq, len(req.Modules))
		copy(r.Modules, req.Modules)
		for _, i := range idx {
			c := cid
			r.Modules[i].ChargeTypeID = &c
		}
		p, vf, err := fitPrim(ds, &r)
		if err != nil {
			continue
		}
		vf.Release()
		ct := ds.typ(cid)
		v := obj{"type_id": cid, "name": ct.Name, "group": ct.Group, "source": obj{"items": p["items"], "stats": p["stats"]}}
		if ct.MetaGroup != nil {
			v["meta_group"] = *ct.MetaGroup
		}
		if ct.MetaLevel != nil {
			v["meta_level"] = *ct.MetaLevel
		}
		vars = append(vars, v)
	}
	return obj{"group": g, "modules": mods, "variants": vars}
}
