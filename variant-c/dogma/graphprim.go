package dogma

// Graph primitives (round 2, approach G2): everything a portable evaluator needs to compute any point of
// the nine graphs, exported once per fit. The engine does all dogma work (modified attributes, cycle and
// volley parameters, capacitor drains, re-evaluated fits such as subwarp speed and scrammed targets, and
// the stacking inputs of a target's speed/signature); the evaluator does the per-point geometry and maths.

import (
	"encoding/json"
	"math"
)

// GraphPrimRequest is the subset of a GraphRequest the engine needs.
type GraphPrimRequest struct {
	Graph  string          `json:"graph"`
	Fit    json.RawMessage `json:"fit"`
	Target *struct {
		Fit json.RawMessage `json:"fit"`
	} `json:"target"`
	Params map[string]any `json:"params"`
}

func (f *Fit) effectNames(i int) []any {
	out := []any{}
	for _, e := range f.Items[i].Effects {
		if ef := f.DS.effect(e.ID); ef != nil {
			out = append(out, ef.Name)
		}
	}
	return out
}

func (f *Fit) typeInfo(i int) obj {
	it := &f.Items[i]
	o := obj{"type_id": it.TypeID, "name": it.T.Name, "group_id": it.Group, "category_id": it.Category}
	if gi := f.DS.group(it.Group); gi != nil {
		o["group"] = gi.Name
	}
	return o
}

func stateName(s State) string { return s.String() }

// itemPrim: modified attributes + effects + cycle/volley data of one module, drone or fighter.
func (f *Fit) itemPrim(i int) obj {
	w := &f.DS.ids
	it := &f.Items[i]
	o := f.typeInfo(i)
	o["index"] = optIdx(it.ReqIndex)
	o["state"] = stateName(it.State)
	o["effects"] = f.effectNames(i)
	o["attrs"] = f.DumpAttrs(i)
	// per-effect range / falloff / tracking values (the attributes each effect names in the SDE)
	er := obj{}
	for _, e := range it.Effects {
		ef := f.DS.effect(e.ID)
		if ef == nil || (ef.RangeAttr == 0 && ef.FalloffAttr == 0) {
			continue
		}
		x := obj{"category": ef.Category, "offensive": ef.IsOffensive, "assistance": ef.IsAssistance}
		if ef.RangeAttr != 0 {
			x["range"] = f.Get(i, ef.RangeAttr)
		}
		if ef.FalloffAttr != 0 {
			x["falloff"] = f.Get(i, ef.FalloffAttr)
		}
		if ef.TrackingAttr != 0 {
			x["tracking"] = f.Get(i, ef.TrackingAttr)
		}
		if ef.ResistanceAttr != 0 {
			if a := f.DS.attr(ef.ResistanceAttr); a != nil {
				x["resistance_attr"] = a.Name
			}
		}
		er[ef.Name] = x
	}
	o["effect_ranges"] = er
	switch it.Kind {
	case KModule:
		o["kind"] = "module"
		vol, kind := f.moduleVolley(i)
		o["weapon_kind"] = kind
		o["volley"] = []any{vol.em, vol.th, vol.ki, vol.ex}
		o["cycle"] = obj{"raw_ms": f.rawCycleMs(i), "reactivation_ms": f.Get(i, w.reactivation), "reload_ms": f.Get(i, w.reload),
			"shots": f.numShots(i), "charges": f.numCharges(i), "avg_ms": f.avgCycleMs(i, false), "avg_reload_ms": f.avgCycleMs(i, true)}
		if it.Charge >= 0 {
			c := int(it.Charge)
			co := f.typeInfo(c)
			co["attrs"] = f.DumpAttrs(c)
			co["effects"] = f.effectNames(c)
			o["charge"] = co
		}
		if it.Spool != nil {
			o["spool"] = obj{"type": it.Spool.Type, "amount": it.Spool.Amount}
		}
	case KDrone:
		o["kind"] = "drone"
		o["quantity"] = it.Quantity
		o["active"] = it.ActiveCount
	case KFighter:
		o["kind"] = "fighter"
		o["quantity"] = it.Quantity
		o["active"] = it.ActiveCount
		ab := []any{}
		for _, a := range it.Abilities {
			if ef := f.DS.effect(a); ef != nil {
				ab = append(ab, ef.Name)
			}
		}
		o["abilities"] = ab
	}
	return o
}

// stackInputs exports base value and modifier list of (item, attr) so an evaluator can re-fold the value
// with extra stacking-penalised multipliers (Pyfa "extended" attributes, e.g. a target slowed by webs).
func (f *Fit) stackInputs(i int, attr uint32) obj {
	base, ok := f.baseOK(i, attr)
	if !ok {
		base = f.DS.AttrDefault(attr)
	}
	var mbuf [foldBuf]*amod
	ms := f.collect(i, attr, mbuf[:0])
	mods := []any{}
	for _, m := range ms {
		mods = append(mods, obj{"op": m.op, "value": f.srcValue(m), "penalized": m.pen})
	}
	hig := true
	o := obj{"base": base, "mods": mods, "value": f.Get(i, attr)}
	if a := f.DS.attr(attr); a != nil {
		hig = a.HighIsGood
		if a.MaxAttr != 0 {
			o["max"] = f.Get(i, a.MaxAttr)
		}
		if a.MinAttr != 0 {
			o["min"] = f.Get(i, a.MinAttr)
		}
	}
	o["high_is_good"] = hig
	return o
}

func (f *Fit) shipPrim(req *FitRequest) obj {
	ship := f.Ship
	o := f.typeInfo(ship)
	o["attrs"] = f.DumpAttrs(ship)
	o["effects"] = f.effectNames(ship)
	o["character"] = f.DumpAttrs(f.Char)
	o["stack"] = obj{"maxVelocity": f.stackInputs(ship, f.DS.ids.maxVelocity), "signatureRadius": f.stackInputs(ship, f.DS.ids.signatureRadius)}
	return o
}

// capDrains mirrors the capacitor section of ComputeStats (same drains the cap simulation uses).
func (f *Fit) capDrains(req *FitRequest) []any {
	w := &f.DS.ids
	g := f.g
	out := []any{}
	for i := range f.Items {
		it := &f.Items[i]
		if it.Kind != KModule {
			continue
		}
		capNeed := f.Get(i, w.capNeed)
		isInj := false
		if gi := f.DS.group(it.Group); gi != nil && gi.Name == "Capacitor Booster" {
			isInj = true
			capNeed = 0
			if it.Charge >= 0 {
				capNeed = -g(int(it.Charge), "capacitorBonus")
			}
		}
		if f.hasEffect(i, w.eNos) && !req.Options.NosNoTargetCap {
			capNeed = -g(i, "powerTransferAmount")
		}
		full := f.rawCycleMs(i) + f.Get(i, w.reactivation)
		if it.State >= Active && capNeed != 0 && full > 0 {
			out = append(out, obj{"index": optIdx(it.ReqIndex), "duration_ms": math.Trunc(full), "cap_need": capNeed, "clip_size": f.numShots(i),
				"reload_ms": f.Get(i, w.reload), "is_injector": isInj, "disable_stagger": f.hasEffect(i, w.eTurret)})
		}
	}
	sigNow := g(f.Ship, "signatureRadius")
	for _, ps := range f.ProjSpecials {
		if ps.Rep || ps.Ecm {
			continue
		}
		need := f.Get(ps.Item, ps.Amount) * ps.Factor * ps.Sign
		if ps.Resist != 0 {
			need *= f.Get(f.Ship, ps.Resist)
		}
		if sres := g(ps.Item, "energyNeutralizerSignatureResolution"); sres != 0 {
			need *= min(sigNow/sres, 1)
		}
		if dur := f.Get(ps.Item, ps.Duration); need != 0 && dur > 0 {
			out = append(out, obj{"duration_ms": math.Trunc(dur), "cap_need": need, "clip_size": 0, "reload_ms": 0, "is_injector": false, "disable_stagger": false, "projected": true})
		}
	}
	return out
}

// fitPrim: the full primitive set of one fit.
func fitPrim(ds *Dataset, req *FitRequest) (obj, *Fit, error) {
	f, err := Build(ds, req)
	if err != nil {
		return nil, nil, err
	}
	items := []any{}
	for i := range f.Items {
		switch f.Items[i].Kind {
		case KModule, KDrone, KFighter:
			items = append(items, f.itemPrim(i))
		}
	}
	stats := Tidy(f.ComputeStats(req, EngineName))
	o := obj{"ship": f.shipPrim(req), "items": items, "cap_drains": f.capDrains(req), "stats": stats}
	return o, f, nil
}

// subwarpSpeed: max velocity with propulsion/cloak/siege/doomsday/cyno/jump-portal modules online and no
// projected effects (Pyfa SubwarpSpeedCache behaviour).
func subwarpSpeed(ds *Dataset, req FitRequest) float64 {
	r := req
	r.Projected = nil
	mods := make([]ModuleReq, len(req.Modules))
	copy(mods, req.Modules)
	groups := map[string]bool{"Propulsion Module": true, "Cloaking Device": true, "Siege Module": true, "Super Weapon": true,
		"Cynosural Field Generator": true, "Jump Portal Generator": true, "Mass Entanglers": true}
	for k := range mods {
		t := ds.typ(mods[k].TypeID)
		if t == nil {
			continue
		}
		gi := ds.group(t.Group)
		if gi != nil && groups[gi.Name] && (mods[k].State == nil || *mods[k].State != Offline) {
			s := Online
			mods[k].State = &s
		}
	}
	r.Modules = mods
	f, err := Build(ds, &r)
	if err != nil {
		return 0
	}
	defer f.Release()
	v := f.g(f.Ship, "maxVelocity")
	return v
}

// scrammed: the fit with MWDs and MJDs set to online (a warp scrambler in range shuts them off).
func scrammedReq(ds *Dataset, req FitRequest) (FitRequest, bool) {
	r := req
	mods := make([]ModuleReq, len(req.Modules))
	copy(mods, req.Modules)
	changed := false
	for k := range mods {
		t := ds.typ(mods[k].TypeID)
		if t == nil || (mods[k].State != nil && *mods[k].State < Active) {
			continue
		}
		if t.HasEffect(ds.ids.eMWD) || t.HasEffect(ds.ids.eMJD) {
			s := Online
			mods[k].State = &s
			changed = true
		}
	}
	r.Modules = mods
	return r, changed
}

// GraphPrimitives returns the primitive JSON object for one graph request.
func GraphPrimitives(ds *Dataset, line []byte) obj {
	var gr GraphPrimRequest
	if err := json.Unmarshal(line, &gr); err != nil {
		return obj{"error": obj{"code": "BAD_REQUEST", "message": err.Error(), "path": ""}}
	}
	errObj := func(err error) obj {
		if e, ok := err.(*EngineError); ok {
			return obj{"error": obj{"code": e.Code, "message": e.Message, "path": e.Path}}
		}
		return obj{"error": obj{"code": "BAD_REQUEST", "message": err.Error(), "path": ""}}
	}
	var req FitRequest
	if err := DecodeRequest(gr.Fit, &req); err != nil {
		return errObj(err)
	}
	src, f, err := fitPrim(ds, &req)
	if err != nil {
		return errObj(err)
	}
	f.Release()
	out := obj{"schema": "eve-dogma-graph-primitives/1", "engine": EngineName, "source": src}
	if gr.Graph == "warp_time" {
		out["subwarp_speed"] = subwarpSpeed(ds, req)
	}
	if gr.Graph == "application_profile" {
		out["charges"] = chargeVariants(ds, req)
	}
	if gr.Target != nil && len(gr.Target.Fit) > 0 && string(gr.Target.Fit) != "null" {
		var treq FitRequest
		if err := DecodeRequest(gr.Target.Fit, &treq); err != nil {
			return errObj(err)
		}
		tp, tf, err := fitPrim(ds, &treq)
		if err != nil {
			return errObj(err)
		}
		tf.Release()
		t := obj{"normal": tp}
		if sr, ok := scrammedReq(ds, treq); ok {
			sp, sf, err := fitPrim(ds, &sr)
			if err == nil {
				sf.Release()
				t["scrammed"] = sp
			}
		}
		out["target"] = t
	}
	return out
}

// GraphPrimitivesJSON encodes primitives (floats kept at full precision).
func GraphPrimitivesJSON(ds *Dataset, line []byte) []byte {
	return appendJSON(nil, GraphPrimitives(ds, line), false)
}
