package dogma

import (
	"fmt"
	"math"
	"sort"
)

type obj = map[string]any

// RangeFactor is the effectiveness of a projected effect at distance (nil distance = 1).
func RangeFactor(optimal, falloff float64, distance *float64, restricted bool) float64 {
	if distance == nil {
		return 1
	}
	d := *distance
	if falloff > 0 {
		if restricted && d > optimal+3*falloff {
			return 0
		}
		x := math.Max(d-optimal, 0) / falloff
		return math.Pow(0.5, x*x)
	}
	if d <= optimal {
		return 1
	}
	return 0
}

func lockTime(scanRes, sig float64) any {
	if scanRes <= 0 || sig <= 0 {
		return nil
	}
	a := math.Asinh(sig)
	return math.Min(40000/scanRes/(a*a), 1800)
}

func floatUnerr(v float64) float64 { return math.Round(v*1e9) / 1e9 }

func toU32(v float64) uint32 {
	if !(v > 0) {
		return 0
	}
	if v >= 4294967295 {
		return 4294967295
	}
	return uint32(v)
}

// Spoolup mirrors Pyfa eos/utils/spoolSupport.calculateSpoolup -> (value, cycles, time).
func Spoolup(maxV, step, cycleS float64, sp Spool) (float64, float64, float64) {
	if maxV == 0 || step == 0 {
		return 0, 0, 0
	}
	var cycles float64
	switch sp.Type {
	case "cycle_scale":
		cycles = math.Round(sp.Amount * math.Ceil(floatUnerr(maxV/step)))
	case "time":
		cycles = math.Min(math.Floor(floatUnerr(sp.Amount/cycleS)), math.Ceil(floatUnerr(maxV/step)))
	case "cycles":
		cycles = math.Min(math.Floor(sp.Amount), math.Ceil(floatUnerr(maxV/step)))
	default: // spool_scale
		cycles = math.Ceil(floatUnerr(maxV * sp.Amount / step))
	}
	return math.Min(cycles*step, maxV), cycles, cycles * cycleS
}

type dmg struct{ em, th, ki, ex float64 }

func (d dmg) total() float64      { return d.em + d.th + d.ki + d.ex }
func (d dmg) scale(k float64) dmg { return dmg{d.em * k, d.th * k, d.ki * k, d.ex * k} }
func (d *dmg) add(o dmg)          { d.em += o.em; d.th += o.th; d.ki += o.ki; d.ex += o.ex }
func (d dmg) vs(r Resists) float64 {
	return d.em*(1-r.EM) + d.th*(1-r.Thermal) + d.ki*(1-r.Kinetic) + d.ex*(1-r.Explosive)
}
func (d dmg) json() obj {
	return obj{"em": d.em, "thermal": d.th, "kinetic": d.ki, "explosive": d.ex, "total": d.total()}
}

func (f *Fit) hasEffect(i int, eid uint32) bool {
	if eid == 0 {
		return false
	}
	for _, e := range f.Items[i].Effects {
		if e.ID == eid {
			return true
		}
	}
	return false
}

func (f *Fit) g(i int, name string) float64 { return f.Get(i, f.DS.AttrID(name)) }

func (f *Fit) rawCycleMs(i int) float64 {
	w := &f.DS.ids
	v := math.Max(f.Get(i, w.speed), f.Get(i, w.duration))
	for _, a := range w.durationExtra {
		v = math.Max(v, f.Get(i, a))
	}
	return v
}

func (f *Fit) numCharges(i int) uint32 {
	c := f.Items[i].Charge
	if c < 0 {
		return 0
	}
	vol := f.Get(int(c), 161)
	if vol <= 0 {
		return 0
	}
	return toU32(math.Floor(floatUnerr(f.Base(i, 38) / vol)))
}

func (f *Fit) numShots(i int) uint32 {
	w := &f.DS.ids
	c := int(f.Items[i].Charge)
	if c < 0 {
		return 0
	}
	n := f.numCharges(i)
	if n > 0 && f.Has(i, w.chargeRate) {
		r := f.Get(i, w.chargeRate)
		if r > 0 {
			return toU32(math.Floor(float64(n) / r))
		}
		return 0
	}
	cgd := f.DS.AttrID("crystalsGetDamaged")
	if n > 0 && f.Has(c, cgd) {
		if f.Get(c, cgd) == 1 {
			hp := f.Get(c, 9)
			chance := f.g(c, "crystalVolatilityChance")
			dm := f.g(c, "crystalVolatilityDamage")
			if dm*chance > 0 {
				return toU32(math.Floor(float64(n) * hp / (dm * chance)))
			}
		}
		return 0
	}
	return 0
}

func (f *Fit) avgCycleMs(i int, factorReload bool) float64 {
	w := &f.DS.ids
	active := f.rawCycleMs(i)
	if active == 0 {
		return 0
	}
	inactive := f.Get(i, w.reactivation)
	shots := f.numShots(i)
	reload := f.Get(i, w.reload)
	if !factorReload || shots == 0 || inactive >= reload {
		return active + inactive
	}
	early := float64(shots) - 1
	return ((active+inactive)*early + (active + reload)) / float64(shots)
}

func (f *Fit) moduleVolley(i int) (dmg, string) {
	w := &f.DS.ids
	it := &f.Items[i]
	kind := "other"
	switch {
	case f.hasEffect(i, w.eTurret):
		kind = "turret"
	case f.hasEffect(i, w.eLauncher):
		kind = "missile"
	case f.hasEffect(i, w.eEmpWave):
		kind = "smartbomb"
	case f.hasEffect(i, w.eChain):
		kind = "vorton"
	}
	src := i
	if it.Charge >= 0 {
		src = int(it.Charge)
	}
	mult := 1.0
	if f.Has(i, w.dmgMult) {
		mult = f.Get(i, w.dmgMult)
	}
	if kind == "missile" && it.Charge >= 0 {
		mult *= f.g(f.Char, "missileDamageMultiplier")
	}
	return dmg{f.Get(src, w.dmg[0]) * mult, f.Get(src, w.dmg[1]) * mult, f.Get(src, w.dmg[2]) * mult, f.Get(src, w.dmg[3]) * mult}, kind
}

func optIdx(i int) any {
	if i < 0 {
		return nil
	}
	return i
}

func usage(u, t float64) obj { return obj{"used": u, "total": t} }

// ComputeStats evaluates the fit statistics (FitStats v1).
func (f *Fit) ComputeStats(req *FitRequest, engineName string) obj {
	ds := f.DS
	w := &ds.ids
	ship, ch := f.Ship, f.Char
	g := f.g
	factorReload := req.Options.FactorReload
	var modules, drones, fighters []int
	for i := range f.Items {
		switch f.Items[i].Kind {
		case KModule:
			modules = append(modules, i)
		case KDrone:
			drones = append(drones, i)
		case KFighter:
			fighters = append(fighters, i)
		}
	}
	// ---------------- resources
	var cpuUsed, pgUsed, calibUsed, bwUsed, bayUsed, fbayUsed, cargoUsed float64
	var slotCount [7]int
	turrets, launchers := 0, 0
	for _, i := range modules {
		it := &f.Items[i]
		if it.State >= Online {
			cpuUsed += f.Get(i, w.cpu)
			pgUsed += f.Get(i, w.power)
		}
		if it.Slot == SlotRig {
			calibUsed += f.Get(i, w.upgradeCost)
		}
		slotCount[it.Slot]++
		if f.hasEffect(i, w.eTurret) {
			turrets++
		}
		if f.hasEffect(i, w.eLauncher) {
			launchers++
		}
	}
	for _, i := range drones {
		bwUsed += g(i, "droneBandwidthUsed") * float64(f.Items[i].ActiveCount)
		bayUsed += f.Get(i, 161) * float64(f.Items[i].Quantity)
	}
	for _, i := range fighters {
		fbayUsed += f.Get(i, 161) * float64(f.Items[i].Quantity)
	}
	for _, c := range req.Cargo {
		if t := ds.Types[c.TypeID]; t != nil {
			cargoUsed += t.Volume * float64(c.Quantity)
		}
	}
	fighterClass := func(i int) string {
		if g(i, "fighterSquadronIsHeavy") > 0 {
			return "heavy"
		} else if g(i, "fighterSquadronIsSupport") > 0 {
			return "support"
		}
		return "light"
	}
	tubes := 0
	classUsed := map[string]float64{"light": 0, "support": 0, "heavy": 0}
	for _, i := range fighters {
		if f.Items[i].ActiveCount > 0 {
			tubes++
			classUsed[fighterClass(i)]++
		}
	}
	resources := obj{
		"cpu":             usage(cpuUsed, f.Get(ship, w.cpuOut)),
		"power":           usage(pgUsed, f.Get(ship, w.powerOut)),
		"calibration":     usage(calibUsed, f.Get(ship, w.upgradeCap)),
		"drone_bandwidth": usage(bwUsed, g(ship, "droneBandwidth")),
		"drone_bay":       usage(bayUsed, g(ship, "droneCapacity")),
		"fighter_bay":     usage(fbayUsed, g(ship, "fighterCapacity")),
		"cargo":           usage(cargoUsed, f.Get(ship, 38)),
		"slots": obj{
			"high":      usage(float64(slotCount[SlotHigh]), g(ship, "hiSlots")),
			"mid":       usage(float64(slotCount[SlotMid]), g(ship, "medSlots")),
			"low":       usage(float64(slotCount[SlotLow]), g(ship, "lowSlots")),
			"rig":       usage(float64(slotCount[SlotRig]), g(ship, "rigSlots")),
			"subsystem": usage(float64(slotCount[SlotSubsystem]), g(ship, "maxSubSystems")),
			"service":   usage(float64(slotCount[SlotService]), g(ship, "serviceSlots")),
		},
		"hardpoints": obj{
			"turret":   usage(float64(turrets), g(ship, "turretSlotsLeft")),
			"launcher": usage(float64(launchers), g(ship, "launcherSlotsLeft")),
		},
		"fighter_tubes": obj{
			"total":   usage(float64(tubes), g(ship, "fighterTubes")),
			"light":   usage(classUsed["light"], g(ship, "fighterLightSlots")),
			"support": usage(classUsed["support"], g(ship, "fighterSupportSlots")),
			"heavy":   usage(classUsed["heavy"], g(ship, "fighterHeavySlots")),
		},
	}

	// ---------------- offense
	tp := TargetProfile{}
	if req.TargetProfile != nil {
		tp = *req.TargetProfile
	}
	tpRes := Resists{tp.EM, tp.Thermal, tp.Kinetic, tp.Explosive}
	defSpool := Spool{Type: "spool_scale", Amount: 1}
	if req.Options.DefaultSpool != nil {
		defSpool = *req.Options.DefaultSpool
	}
	weapons := []any{}
	var wVol, wDps dmg
	for _, i := range modules {
		it := &f.Items[i]
		if it.State < Active {
			continue
		}
		base, kind := f.moduleVolley(i)
		if base.total() == 0 {
			continue
		}
		cyc := f.avgCycleMs(i, factorReload)
		raw := f.rawCycleMs(i)
		sp := defSpool
		if it.Spool != nil {
			sp = *it.Spool
		}
		spv, _, _ := Spoolup(g(i, "damageMultiplierBonusMax"), g(i, "damageMultiplierBonusPerCycle"), raw/1000, sp)
		vs := base.scale(1 + spv)
		var dps dmg
		if cyc > 0 {
			dps = vs.scale(1000 / cyc)
		}
		wVol.add(vs)
		wDps.add(dps)
		var chargeID any
		if it.Charge >= 0 {
			chargeID = f.Items[it.Charge].TypeID
		}
		wo := obj{"module_index": optIdx(it.ReqIndex), "type_id": it.TypeID, "name": it.T.Name, "kind": kind,
			"charge_type_id": chargeID, "volley": vs.json(), "dps": dps.json(), "cycle_time_ms": cyc}
		switch kind {
		case "turret":
			wo["optimal_m"] = g(i, "maxRange")
			wo["falloff_m"] = g(i, "falloff")
			wo["tracking"] = g(i, "trackingSpeed")
		case "missile":
			if it.Charge >= 0 {
				c := int(it.Charge)
				// Pyfa missileMaxRangeData: flight time + ship radius bonus, acceleration phase, floor/ceil blend,
				// FoF limit, centre-to-surface (eos/saveddata/module.py, LGPL)
				if vel := g(c, "maxVelocity"); vel > 0 {
					radius := g(ship, "radius")
					ft := floatUnerr(g(c, "explosionDelay")/1000 + radius/vel)
					accelCap := g(c, "mass") * g(c, "agility") / 1e6
					rangeAt := func(t float64) float64 {
						acc := math.Min(t, accelCap)
						return vel/2*acc + vel*(t-acc)
					}
					lt, ht := math.Floor(ft), math.Ceil(ft)
					lr, hr := rangeAt(lt), rangeAt(ht)
					if f.hasEffect(c, ds.EffectID("fofMissileLaunching")) {
						if lim := g(c, "maxFOFTargetRange"); lim > 0 {
							lr, hr = math.Min(lr, lim), math.Min(hr, lim)
						}
					}
					lr, hr = math.Max(lr-radius, 0), math.Max(hr-radius, 0)
					hc := ft - lt
					wo["range_m"] = lr*(1-hc) + hr*hc
				}
				wo["explosion_radius"] = g(c, "aoeCloudSize")
				wo["explosion_velocity"] = g(c, "aoeVelocity")
			}
		case "smartbomb":
			wo["range_m"] = g(i, "empFieldRange")
		}
		if spv > 0 {
			wo["spool_multiplier"] = 1 + spv
			wo["volley_unspooled"] = base.json()
		}
		weapons = append(weapons, wo)
	}
	var dVol, dDps dmg
	droneOut := []any{}
	for _, i := range drones {
		it := &f.Items[i]
		n := float64(it.ActiveCount)
		if n == 0 {
			continue
		}
		mult := 1.0
		if f.Has(i, w.dmgMult) {
			mult = f.Get(i, w.dmgMult)
		}
		v := dmg{f.Get(i, w.dmg[0]), f.Get(i, w.dmg[1]), f.Get(i, w.dmg[2]), f.Get(i, w.dmg[3])}.scale(mult * n)
		cyc := f.rawCycleMs(i)
		if v.total() == 0 || cyc == 0 {
			continue
		}
		dps := v.scale(1000 / cyc)
		dVol.add(v)
		dDps.add(dps)
		droneOut = append(droneOut, obj{"drone_index": optIdx(it.ReqIndex), "type_id": it.TypeID, "name": it.T.Name,
			"count": n, "volley": v.json(), "dps": dps.json()})
	}
	var fVol, fDps dmg
	fighterOut := []any{}
	for _, i := range fighters {
		it := &f.Items[i]
		n := float64(it.ActiveCount)
		if n == 0 {
			continue
		}
		var fv, fd dmg
		for _, ab := range [2]struct {
			eid    uint32
			prefix string
		}{{w.eFighterAttackM, "fighterAbilityAttackMissile"}, {w.eFighterMissiles, "fighterAbilityMissiles"}} {
			if !f.hasEffect(i, ab.eid) || !containsU32(it.Abilities, ab.eid) {
				continue
			}
			m := g(i, ab.prefix+"DamageMultiplier")
			if m == 0 {
				m = 1
			}
			v := dmg{g(i, ab.prefix+"DamageEM"), g(i, ab.prefix+"DamageTherm"), g(i, ab.prefix+"DamageKin"), g(i, ab.prefix+"DamageExp")}.scale(m * n)
			dur := g(i, ab.prefix+"Duration")
			fv.add(v)
			if dur > 0 {
				fd.add(v.scale(1000 / dur))
			}
		}
		if fv.total() > 0 {
			fVol.add(fv)
			fDps.add(fd)
			fighterOut = append(fighterOut, obj{"fighter_index": optIdx(it.ReqIndex), "type_id": it.TypeID, "name": it.T.Name,
				"squadron_size": n, "volley": fv.json(), "dps": fd.json()})
		}
	}
	tVol, tDps := wVol, wDps
	tVol.add(dVol)
	tVol.add(fVol)
	tDps.add(dDps)
	tDps.add(fDps)
	offense := obj{
		"weapons": weapons, "drones": droneOut, "fighters": fighterOut,
		"total": obj{"weapon_dps": wDps.total(), "weapon_volley": wVol.total(), "drone_dps": dDps.total(), "drone_volley": dVol.total(),
			"fighter_dps": fDps.total(), "fighter_volley": fVol.total(), "dps": tDps.json(), "volley": tVol.json()},
		"vs_target_profile": obj{"dps": tDps.vs(tpRes), "volley": tVol.vs(tpRes)},
	}

	// ---------------- defense
	dp := Resists{25, 25, 25, 25}
	if req.DamagePattern != nil {
		dp = *req.DamagePattern
	}
	dpTot := math.Max(dp.EM+dp.Thermal+dp.Kinetic+dp.Explosive, 1e-12)
	layer := func(prefix string) [4]float64 {
		if prefix == "" {
			return [4]float64{g(ship, "emDamageResonance"), g(ship, "thermalDamageResonance"), g(ship, "kineticDamageResonance"), g(ship, "explosiveDamageResonance")}
		}
		return [4]float64{g(ship, prefix+"EmDamageResonance"), g(ship, prefix+"ThermalDamageResonance"), g(ship, prefix+"KineticDamageResonance"), g(ship, prefix+"ExplosiveDamageResonance")}
	}
	effectivify := func(amount float64, r [4]float64) float64 {
		div := (dp.EM*r[0] + dp.Thermal*r[1] + dp.Kinetic*r[2] + dp.Explosive*r[3]) / dpTot
		if div == 0 {
			return amount
		}
		return amount / div
	}
	rs, ra, rh := layer("shield"), layer("armor"), layer("")
	hpS, hpA, hpH := g(ship, "shieldCapacity"), g(ship, "armorHP"), f.Get(ship, 9)
	eS, eA, eH := effectivify(hpS, rs), effectivify(hpA, ra), effectivify(hpH, rh)
	resJ := func(r [4]float64) obj { return obj{"em": r[0], "thermal": r[1], "kinetic": r[2], "explosive": r[3]} }
	var shieldRep, armorRep, hullRep float64
	for _, i := range modules {
		if f.Items[i].State < Active {
			continue
		}
		dur := f.Get(i, w.duration) / 1000
		if dur <= 0 {
			continue
		}
		if f.hasEffect(i, w.eShieldBoost) || f.hasEffect(i, w.eFueledShieldBoost) {
			shieldRep += g(i, "shieldBonus") / dur
		}
		if f.hasEffect(i, w.eArmorRep) {
			armorRep += g(i, "armorDamageAmount") / dur
		}
		if f.hasEffect(i, w.eFueledArmorRep) {
			k := 1.0
			if c := f.Items[i].Charge; c >= 0 && f.Items[c].T.Name == "Nanite Repair Paste" {
				k = 3
			}
			armorRep += g(i, "armorDamageAmount") * k / dur
		}
		if f.hasEffect(i, w.eHullRep) {
			hullRep += g(i, "structureDamageAmount") / dur
		}
	}
	// incoming remote repairs (Pyfa __getAppliedRr diminishing-returns formula)
	{
		type rr struct{ a, c float64 }
		var lists [3][]rr
		for _, ps := range f.ProjSpecials {
			if !ps.Rep {
				continue
			}
			if dur := f.Get(ps.Item, w.duration) / 1000; dur > 0 {
				lists[ps.Layer] = append(lists[ps.Layer], rr{f.Get(ps.Item, ps.Amount) * ps.Mult * ps.Factor, dur})
			}
		}
		applied := func(l []rr) float64 {
			total := 0.0
			for _, x := range l {
				total += x.a / math.Trunc(x.c)
			}
			sum := 0.0
			for _, x := range l {
				rrps := x.a / math.Trunc(x.c)
				m := 7000 + rrps*20
				q := (rrps+m)/(total+m) - 1
				sum += (1 - q*q) * x.a / x.c
			}
			return sum
		}
		shieldRep += applied(lists[0])
		armorRep += applied(lists[1])
		hullRep += applied(lists[2])
	}
	srr := g(ship, "shieldRechargeRate") / 1000
	passive := 0.0
	if srr > 0 {
		passive = 10 / srr * 0.5 * 0.5 * hpS
	}
	defense := obj{
		"hp":             obj{"shield": hpS, "armor": hpA, "hull": hpH, "total": hpS + hpA + hpH},
		"resonance":      obj{"shield": resJ(rs), "armor": resJ(ra), "hull": resJ(rh)},
		"ehp":            obj{"shield": eS, "armor": eA, "hull": eH, "total": eS + eA + eH},
		"damage_pattern": obj{"em": dp.EM, "thermal": dp.Thermal, "kinetic": dp.Kinetic, "explosive": dp.Explosive},
		"tank": obj{
			"raw": obj{"passive_shield": passive, "shield_repair": shieldRep, "armor_repair": armorRep, "hull_repair": hullRep},
			"effective": obj{"passive_shield": effectivify(passive, rs), "shield_repair": effectivify(shieldRep, rs),
				"armor_repair": effectivify(armorRep, ra), "hull_repair": effectivify(hullRep, rh)},
		},
	}

	// ---------------- capacitor
	capC := g(ship, "capacitorCapacity")
	rr := g(ship, "rechargeRate")
	peak := 0.0
	if rr > 0 {
		peak = 10 / (rr / 1000) * 0.5 * 0.5 * capC
	}
	var drains []Drain
	var capUsed, capAdded float64
	moduleRows := []any{}
	for _, i := range modules {
		it := &f.Items[i]
		capNeed := f.Get(i, w.capNeed)
		isInj := false
		if gi := ds.Groups[it.Group]; gi != nil && gi.Name == "Capacitor Booster" {
			isInj = true
			capNeed = 0
			if it.Charge >= 0 {
				capNeed = -g(int(it.Charge), "capacitorBonus")
			}
		}
		if f.hasEffect(i, w.eNos) && !req.Options.NosNoTargetCap {
			capNeed = -g(i, "powerTransferAmount")
		}
		cycRaw := f.rawCycleMs(i)
		full := cycRaw + f.Get(i, w.reactivation)
		row := obj{"module_index": optIdx(it.ReqIndex), "type_id": it.TypeID, "name": it.T.Name, "slot": it.Slot,
			"state": it.State, "cpu": f.Get(i, w.cpu), "power": f.Get(i, w.power)}
		if cycRaw > 0 {
			row["cycle_time_ms"] = cycRaw
		}
		if it.State >= Active && capNeed != 0 && full > 0 {
			avg := f.avgCycleMs(i, factorReload)
			use := 0.0
			if avg > 0 {
				use = capNeed / (avg / 1000)
			}
			if use > 0 {
				capUsed += use
			} else {
				capAdded -= use
			}
			row["cap_use_gj_s"] = use
			drains = append(drains, Drain{Duration: math.Trunc(full), CapNeed: capNeed, ClipSize: f.numShots(i),
				ReloadMs: f.Get(i, w.reload), IsInjector: isInj, DisableStagger: f.hasEffect(i, w.eTurret)})
		}
		moduleRows = append(moduleRows, row)
	}
	// incoming neuts / nos / cap transfers (Pyfa fit.addDrain): no stagger, after the fit's own modules
	sigNow := g(ship, "signatureRadius")
	for _, ps := range f.ProjSpecials {
		if ps.Rep {
			continue
		}
		need := f.Get(ps.Item, ps.Amount) * ps.Factor * ps.Sign
		if ps.Resist != 0 {
			need *= f.Get(ship, ps.Resist)
		}
		if sres := g(ps.Item, "energyNeutralizerSignatureResolution"); sres != 0 {
			need *= math.Min(sigNow/sres, 1)
		}
		if dur := f.Get(ps.Item, ps.Duration); need != 0 && dur > 0 {
			drains = append(drains, Drain{Duration: math.Trunc(dur), CapNeed: need})
		}
	}
	capj := obj{"capacity": capC, "recharge_time_s": rr / 1000, "peak_recharge_gj_s": peak, "use_gj_s": capUsed,
		"injected_gj_s": capAdded, "delta_gj_s": peak + capAdded - capUsed}
	if len(drains) == 0 {
		capj["stable"] = true
		capj["stable_percent"] = 100.0
	} else {
		o := req.Options.CapSim
		tmax := 6.0 * 3600
		if o.MaxTimeS != nil {
			tmax = *o.MaxTimeS
		}
		r := SimulateCap(capC, rr, drains, 1, o.Reload || factorReload, true, tmax*1000)
		st := (r.StableLow + r.StableHigh) / 2
		stable := r.Stable && st > 0
		capj["stable"] = stable
		if stable {
			capj["stable_percent"] = math.Min(st*100, 100)
		} else {
			capj["depletes_in_s"] = r.TS
		}
		capj["eve_stable_percent"] = r.EveStable * 100
		capj["sim_iterations"] = r.Iterations
	}

	// ---------------- navigation
	maxv := g(ship, "maxVelocity")
	limit := g(ship, "speedLimit")
	maxSpeed := maxv
	if limit > 0 && maxv > limit {
		maxSpeed = limit
	}
	mass := f.Get(ship, 4)
	agility := g(ship, "agility")
	baseWarp := g(ship, "baseWarpSpeed")
	if baseWarp == 0 {
		baseWarp = 1
	}
	warpMult := g(ship, "warpSpeedMultiplier")
	if warpMult == 0 {
		warpMult = 1
	}
	warpNeed := g(ship, "warpCapacitorNeed")
	sig := g(ship, "signatureRadius")
	maxWarp := 0.0
	if warpNeed > 0 && mass > 0 {
		maxWarp = capC / (mass * warpNeed)
	}
	navigation := obj{"max_velocity": maxSpeed, "align_time_s": -math.Log(0.25) * agility * mass / 1e6, "mass": mass,
		"agility": agility, "signature_radius": sig, "warp_speed_au_s": baseWarp * warpMult,
		"max_warp_distance_au": maxWarp, "warp_scramble_status": g(ship, "warpScrambleStatus")}

	// ---------------- targeting
	bestN, bestV := "none", 0.0
	for _, s := range [4][2]string{{"radar", "scanRadarStrength"}, {"ladar", "scanLadarStrength"}, {"magnetometric", "scanMagnetometricStrength"}, {"gravimetric", "scanGravimetricStrength"}} {
		if v := g(ship, s[1]); v > bestV {
			bestN, bestV = s[0], v
		}
	}
	scanRes := g(ship, "scanResolution")
	var probe any
	if bestV > 0 {
		probe = math.Max(sig/bestV, 1.08)
	}
	var ltTP any
	if tp.SignatureRadius != nil {
		ltTP = lockTime(scanRes, *tp.SignatureRadius)
	}
	targeting := obj{
		"max_targets": math.Min(g(ship, "maxLockedTargets"), math.Max(g(ch, "maxLockedTargets"), 0)),
		"max_range_m": g(ship, "maxTargetRange"), "scan_resolution": scanRes, "sensor_strength": bestV, "sensor_type": bestN,
		"probe_size": probe,
		"lock_time_s": obj{"sig_25m": lockTime(scanRes, 25), "sig_40m": lockTime(scanRes, 40), "sig_125m": lockTime(scanRes, 125),
			"sig_400m": lockTime(scanRes, 400), "sig_target_profile": ltTP},
	}
	activeDrones := uint32(0)
	for _, i := range drones {
		activeDrones += f.Items[i].ActiveCount
	}
	dronesJ := obj{"active": activeDrones, "max_active": g(ch, "maxActiveDrones"), "control_range_m": g(ch, "droneControlDistance")}

	st := f.Items[ship].T
	var grp any
	if gi := ds.Groups[st.Group]; gi != nil {
		grp = gi.Name
	}
	out := obj{
		"meta":      obj{"schema_version": 1, "engine": engineName, "sde_build": ds.Build, "dataset_sha256": ds.SHA256},
		"ship":      obj{"type_id": st.ID, "name": st.Name, "group": grp},
		"resources": resources, "offense": offense, "defense": defense, "capacitor": capj,
		"navigation": navigation, "targeting": targeting, "drones": dronesJ, "modules": moduleRows,
	}
	if req.Options.Validate {
		out["violations"] = f.validate(cpuUsed, pgUsed, calibUsed, bwUsed)
	}
	if len(f.Warnings) > 0 {
		out["warnings"] = f.Warnings
	}
	if ia := req.Options.IncludeAttributes; ia != nil {
		switch *ia {
		case "ship":
			out["attributes"] = obj{"ship": f.DumpAttrs(ship)}
		case "all":
			mods := []any{}
			for _, i := range modules {
				var c any
				if f.Items[i].Charge >= 0 {
					c = f.DumpAttrs(int(f.Items[i].Charge))
				}
				mods = append(mods, obj{"module_index": optIdx(f.Items[i].ReqIndex), "type_id": f.Items[i].TypeID, "attributes": f.DumpAttrs(i), "charge": c})
			}
			dr := []any{}
			for _, i := range drones {
				dr = append(dr, obj{"drone_index": optIdx(f.Items[i].ReqIndex), "attributes": f.DumpAttrs(i)})
			}
			out["attributes"] = obj{"ship": f.DumpAttrs(ship), "character": f.DumpAttrs(ch), "modules": mods, "drones": dr}
		}
	}
	return out
}

// DumpAttrs returns name -> modified value for every attribute the item carries.
func (f *Fit) DumpAttrs(i int) obj {
	m := obj{}
	for _, a := range f.AttrIDs(i) {
		name := fmt.Sprint(a)
		if ai := f.DS.Attrs[a]; ai != nil {
			name = ai.Name
		}
		m[name] = f.Get(i, a)
	}
	return m
}

func (f *Fit) validate(cpu, pg, calib, bw float64) []any {
	ds := f.DS
	ship := f.Ship
	g := f.g
	v := []any{}
	push := func(code, msg string, idx int) {
		v = append(v, obj{"code": code, "message": msg, "module_index": optIdx(idx)})
	}
	if cpu > g(ship, "cpuOutput")+1e-9 {
		push("CPU_OVERLOAD", fmt.Sprintf("CPU used %.2f > output %.2f", cpu, g(ship, "cpuOutput")), -1)
	}
	if pg > g(ship, "powerOutput")+1e-9 {
		push("POWER_OVERLOAD", fmt.Sprintf("Powergrid used %.2f > output %.2f", pg, g(ship, "powerOutput")), -1)
	}
	if calib > g(ship, "upgradeCapacity")+1e-9 {
		push("CALIBRATION_OVERLOAD", fmt.Sprintf("Calibration used %v > %v", calib, g(ship, "upgradeCapacity")), -1)
	}
	if bw > g(ship, "droneBandwidth")+1e-9 {
		push("DRONE_BANDWIDTH", fmt.Sprintf("Drone bandwidth used %v > %v", bw, g(ship, "droneBandwidth")), -1)
	}
	var modules []int
	for i := range f.Items {
		if f.Items[i].Kind == KModule {
			modules = append(modules, i)
		}
	}
	slotNamesV := map[Slot]string{SlotHigh: "High", SlotMid: "Mid", SlotLow: "Low", SlotRig: "Rig", SlotSubsystem: "Subsystem", SlotService: "Service"}
	for _, sa := range []struct {
		s Slot
		a string
	}{{SlotHigh, "hiSlots"}, {SlotMid, "medSlots"}, {SlotLow, "lowSlots"}, {SlotRig, "rigSlots"}, {SlotSubsystem, "maxSubSystems"}, {SlotService, "serviceSlots"}} {
		used := 0.0
		for _, i := range modules {
			if f.Items[i].Slot == sa.s {
				used++
			}
		}
		if used > g(ship, sa.a) {
			push("SLOTS_EXCEEDED", fmt.Sprintf("%s slots used %v > %v", slotNamesV[sa.s], used, g(ship, sa.a)), -1)
		}
	}
	w := &ds.ids
	t, l := 0.0, 0.0
	for _, i := range modules {
		if f.hasEffect(i, w.eTurret) {
			t++
		}
		if f.hasEffect(i, w.eLauncher) {
			l++
		}
	}
	if t > g(ship, "turretSlotsLeft") {
		push("TURRET_HARDPOINTS", fmt.Sprintf("turrets %v > hardpoints %v", t, g(ship, "turretSlotsLeft")), -1)
	}
	if l > g(ship, "launcherSlotsLeft") {
		push("LAUNCHER_HARDPOINTS", fmt.Sprintf("launchers %v > hardpoints %v", l, g(ship, "launcherSlotsLeft")), -1)
	}
	shipT := f.Items[ship].T
	var groupAttrs, typeAttrs []uint32
	for k := 1; k <= 20; k++ {
		if a := ds.AttrID(fmt.Sprintf("canFitShipGroup%02d", k)); a != 0 {
			groupAttrs = append(groupAttrs, a)
		}
	}
	for k := 1; k <= 11; k++ {
		if a := ds.AttrID(fmt.Sprintf("canFitShipType%d", k)); a != 0 {
			typeAttrs = append(typeAttrs, a)
		}
	}
	fittedGroup, fittedType, activeGroup, onlineGroup := map[uint32]uint32{}, map[uint32]uint32{}, map[uint32]uint32{}, map[uint32]uint32{}
	for _, i := range modules {
		it := &f.Items[i]
		idx := it.ReqIndex
		mt := it.T
		name := mt.Name
		if it.Slot == SlotNone {
			push("NOT_FITTABLE", fmt.Sprintf("%s is not a fittable module", name), idx)
		}
		var gr, ty []uint32
		for _, a := range groupAttrs {
			if x, ok := mt.Attr(a); ok && uint32(x) != 0 {
				gr = append(gr, uint32(x))
			}
		}
		for _, a := range typeAttrs {
			if x, ok := mt.Attr(a); ok && uint32(x) != 0 {
				ty = append(ty, uint32(x))
			}
		}
		if (len(gr) > 0 || len(ty) > 0) && !containsU32(gr, shipT.Group) && !containsU32(ty, shipT.ID) {
			push("SHIP_RESTRICTION", fmt.Sprintf("%s cannot be fitted to %s", name, shipT.Name), idx)
		}
		if it.Slot == SlotRig {
			rsz, _ := mt.Attr(ds.AttrID("rigSize"))
			srs := g(ship, "rigSize")
			if rsz != 0 && rsz != srs {
				push("RIG_SIZE", fmt.Sprintf("%s rig size %v != ship rig size %v", name, rsz, srs), idx)
			}
		}
		fittedGroup[it.Group]++
		fittedType[it.TypeID]++
		if it.State >= Online {
			onlineGroup[it.Group]++
		}
		if it.State >= Active {
			activeGroup[it.Group]++
		}
		check := func(attr string, m map[uint32]uint32, key uint32) (float64, uint32, bool) {
			lim, ok := mt.Attr(ds.AttrID(attr))
			if !ok {
				return 0, 0, false
			}
			n := m[key]
			return lim, n, lim > 0 && float64(n) > lim
		}
		if lim, n, bad := check("maxGroupFitted", fittedGroup, it.Group); bad {
			push("MAX_GROUP_FITTED", fmt.Sprintf("%s: %d fitted of group, max %v", name, n, lim), idx)
		}
		if lim, n, bad := check("maxTypeFitted", fittedType, it.TypeID); bad {
			push("MAX_TYPE_FITTED", fmt.Sprintf("%s: %d fitted, max %v", name, n, lim), idx)
		}
		if lim, n, bad := check("maxGroupOnline", onlineGroup, it.Group); bad {
			push("MAX_GROUP_ONLINE", fmt.Sprintf("%s: %d online of group, max %v", name, n, lim), idx)
		}
		if lim, n, bad := check("maxGroupActive", activeGroup, it.Group); bad {
			push("MAX_GROUP_ACTIVE", fmt.Sprintf("%s: %d active of group, max %v", name, n, lim), idx)
		}
		if it.Charge >= 0 {
			ct := f.Items[it.Charge].T
			var cg []uint32
			for k := 1; k <= 5; k++ {
				if x, ok := mt.Attr(ds.AttrID(fmt.Sprintf("chargeGroup%d", k))); ok && uint32(x) != 0 {
					cg = append(cg, uint32(x))
				}
			}
			if !containsU32(cg, ct.Group) {
				push("CHARGE_GROUP", fmt.Sprintf("%s cannot be loaded into %s", ct.Name, name), idx)
			}
			ms, ok1 := mt.Attr(ds.AttrID("chargeSize"))
			cs, ok2 := ct.Attr(ds.AttrID("chargeSize"))
			if ok1 && ok2 && ms != cs {
				push("CHARGE_SIZE", fmt.Sprintf("%s size %v != launcher size %v", ct.Name, cs, ms), idx)
			}
			if ct.Volume > mt.Capacity && mt.Capacity > 0 {
				push("CHARGE_CAPACITY", fmt.Sprintf("%s does not fit into %s", ct.Name, name), idx)
			}
		}
	}
	type miss struct {
		s    uint32
		need float64
		by   uint32
	}
	var missing []miss
	for i := range f.Items {
		it := &f.Items[i]
		switch it.Kind {
		case KShip, KModule, KCharge, KDrone, KFighter, KImplant, KBooster:
		default:
			continue
		}
		t := it.T
		for k := 1; k <= 6; k++ {
			sv, _ := t.Attr(ds.AttrID(reqSkillNames[k-1][0]))
			s := uint32(sv)
			if s == 0 {
				continue
			}
			need, ok := t.Attr(ds.AttrID(reqSkillNames[k-1][1]))
			if !ok {
				need = 1
			}
			dup := false
			for _, m := range missing {
				if m.s == s && m.need >= need {
					dup = true
				}
			}
			if f.skillLevel(s) < need && !dup {
				missing = append(missing, miss{s, need, it.TypeID})
			}
		}
	}
	for _, m := range missing {
		sn := "?"
		if t := ds.Types[m.s]; t != nil {
			sn = t.Name
		}
		push("MISSING_SKILL", fmt.Sprintf("%s %v required by %s", sn, m.need, ds.Types[m.by].Name), -1)
	}
	return v
}

var reqSkillNames = [6][2]string{{"requiredSkill1", "requiredSkill1Level"}, {"requiredSkill2", "requiredSkill2Level"},
	{"requiredSkill3", "requiredSkill3Level"}, {"requiredSkill4", "requiredSkill4Level"},
	{"requiredSkill5", "requiredSkill5Level"}, {"requiredSkill6", "requiredSkill6Level"}}

// skillLevel returns the character's level of skill s (0 if absent). Skill items are contiguous and
// sorted by type id (see Build).
func (f *Fit) skillLevel(s uint32) float64 {
	sk := f.Items[f.skillLo:f.skillHi]
	i := sort.Search(len(sk), func(i int) bool { return sk[i].TypeID >= s })
	if i < len(sk) && sk[i].TypeID == s {
		lv, _ := sk[i].overlay.get(attrSkillLevel)
		return lv
	}
	return 0
}
