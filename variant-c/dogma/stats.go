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
		x := max(d-optimal, 0) / falloff
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
	return min(40000/scanRes/(a*a), 1800)
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
		cycles = min(math.Floor(floatUnerr(sp.Amount/cycleS)), math.Ceil(floatUnerr(maxV/step)))
	case "cycles":
		cycles = min(math.Floor(sp.Amount), math.Ceil(floatUnerr(maxV/step)))
	default: // spool_scale
		cycles = math.Ceil(floatUnerr(maxV * sp.Amount / step))
	}
	return min(cycles*step, maxV), cycles, cycles * cycleS
}

type dmg struct{ em, th, ki, ex float64 }

func (d dmg) total() float64      { return d.em + d.th + d.ki + d.ex }
func (d dmg) scale(k float64) dmg { return dmg{d.em * k, d.th * k, d.ki * k, d.ex * k} }
func (d *dmg) add(o dmg)          { d.em += o.em; d.th += o.th; d.ki += o.ki; d.ex += o.ex }
func (d dmg) vs(r Resists) float64 {
	return d.em*(1-r.EM) + d.th*(1-r.Thermal) + d.ki*(1-r.Kinetic) + d.ex*(1-r.Explosive)
}
func (d dmg) json() *fobj {
	return &fobj{keysDmg, [5]float64{d.em, d.ex, d.ki, d.th, d.total()}}
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
	v := max(f.Get(i, w.speed), f.Get(i, w.duration))
	for _, a := range w.durationExtra {
		v = max(v, f.Get(i, a))
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

func usage(u, t float64) *fobj { return &fobj{k: keysUsage, v: [5]float64{t, u}} }

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
		if t := ds.typ(c.TypeID); t != nil {
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
	resources := newK(10).
		A("cpu", usage(cpuUsed, f.Get(ship, w.cpuOut))).
		A("power", usage(pgUsed, f.Get(ship, w.powerOut))).
		A("calibration", usage(calibUsed, f.Get(ship, w.upgradeCap))).
		A("drone_bandwidth", usage(bwUsed, g(ship, "droneBandwidth"))).
		A("drone_bay", usage(bayUsed, g(ship, "droneCapacity"))).
		A("fighter_bay", usage(fbayUsed, g(ship, "fighterCapacity"))).
		A("cargo", usage(cargoUsed, f.Get(ship, 38))).
		A("slots", newK(6).
			A("high", usage(float64(slotCount[SlotHigh]), g(ship, "hiSlots"))).
			A("mid", usage(float64(slotCount[SlotMid]), g(ship, "medSlots"))).
			A("low", usage(float64(slotCount[SlotLow]), g(ship, "lowSlots"))).
			A("rig", usage(float64(slotCount[SlotRig]), g(ship, "rigSlots"))).
			A("subsystem", usage(float64(slotCount[SlotSubsystem]), g(ship, "maxSubSystems"))).
			A("service", usage(float64(slotCount[SlotService]), g(ship, "serviceSlots")))).
		A("hardpoints", newK(2).
			A("turret", usage(float64(turrets), g(ship, "turretSlotsLeft"))).
			A("launcher", usage(float64(launchers), g(ship, "launcherSlotsLeft")))).
		A("fighter_tubes", newK(4).
			A("total", usage(float64(tubes), g(ship, "fighterTubes"))).
			A("light", usage(classUsed["light"], g(ship, "fighterLightSlots"))).
			A("support", usage(classUsed["support"], g(ship, "fighterSupportSlots"))).
			A("heavy", usage(classUsed["heavy"], g(ship, "fighterHeavySlots"))))

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
		// doomsdays / lances deal their volley every doomsdayDamageCycleTime during doomsdayDamageDuration
		// (Pyfa getVolleyParameters subcycles; the Reaper slash hits once); volley = one tick
		subcycles := 1.0
		if dd, dsub := g(i, "doomsdayDamageDuration"), g(i, "doomsdayDamageCycleTime"); dd != 0 && dsub != 0 && !f.hasEffectNamed(i, "doomsdaySlash") {
			subcycles = max(math.Floor(floatUnerr7(dd/dsub)), 0)
		}
		var dps dmg
		if cyc > 0 {
			dps = vs.scale(subcycles * 1000 / cyc)
		}
		wVol.add(vs)
		wDps.add(dps)
		var chargeID any
		if it.Charge >= 0 {
			chargeID = f.Items[it.Charge].TypeID
		}
		wo := newK(14).A("module_index", optIdx(it.ReqIndex)).U("type_id", uint64(it.TypeID)).S("name", it.T.Name).S("kind", kind).
			A("charge_type_id", chargeID).A("volley", vs.json()).A("dps", dps.json()).F("cycle_time_ms", cyc)
		switch kind {
		case "turret":
			wo.F("optimal_m", g(i, "maxRange")).F("falloff_m", g(i, "falloff")).F("tracking", g(i, "trackingSpeed"))
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
						acc := min(t, accelCap)
						return vel/2*acc + vel*(t-acc)
					}
					lt, ht := math.Floor(ft), math.Ceil(ft)
					lr, hr := rangeAt(lt), rangeAt(ht)
					if f.hasEffect(c, ds.EffectID("fofMissileLaunching")) {
						if lim := g(c, "maxFOFTargetRange"); lim > 0 {
							lr, hr = min(lr, lim), min(hr, lim)
						}
					}
					lr, hr = max(lr-radius, 0), max(hr-radius, 0)
					hc := ft - lt
					wo.F("range_m", lr*(1-hc)+hr*hc)
				}
				wo.F("explosion_radius", g(c, "aoeCloudSize")).F("explosion_velocity", g(c, "aoeVelocity"))
			}
		case "smartbomb":
			wo.F("range_m", g(i, "empFieldRange"))
		}
		if spv > 0 {
			wo.F("spool_multiplier", 1+spv).A("volley_unspooled", base.json())
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
		droneOut = append(droneOut, newK(11).A("drone_index", optIdx(it.ReqIndex)).U("type_id", uint64(it.TypeID)).S("name", it.T.Name).
			F("count", n).A("volley", v.json()).A("dps", dps.json()).F("optimal_m", g(i, "maxRange")).F("falloff_m", g(i, "falloff")).
			F("tracking", g(i, "trackingSpeed")).F("max_velocity", g(i, "maxVelocity")).F("signature_radius", g(i, "signatureRadius")))
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
			fighterOut = append(fighterOut, newK(8).A("fighter_index", optIdx(it.ReqIndex)).U("type_id", uint64(it.TypeID)).S("name", it.T.Name).
				F("squadron_size", n).A("volley", fv.json()).A("dps", fd.json()).F("max_velocity", g(i, "maxVelocity")).
				F("signature_radius", g(i, "signatureRadius")))
		}
	}
	tVol, tDps := wVol, wDps
	tVol.add(dVol)
	tVol.add(fVol)
	tDps.add(dDps)
	tDps.add(fDps)
	offense := newK(5).A("weapons", weapons).A("drones", droneOut).A("fighters", fighterOut).
		A("total", newK(8).F("weapon_dps", wDps.total()).F("weapon_volley", wVol.total()).F("drone_dps", dDps.total()).F("drone_volley", dVol.total()).
			F("fighter_dps", fDps.total()).F("fighter_volley", fVol.total()).A("dps", tDps.json()).A("volley", tVol.json())).
		A("vs_target_profile", newK(2).F("dps", tDps.vs(tpRes)).F("volley", tVol.vs(tpRes)))

	// ---------------- defense
	dp := Resists{25, 25, 25, 25}
	if req.DamagePattern != nil {
		dp = *req.DamagePattern
	}
	dpTot := max(dp.EM+dp.Thermal+dp.Kinetic+dp.Explosive, 1e-12)
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
	resJ := func(r [4]float64) *fobj { return &fobj{k: keysRes, v: [5]float64{r[0], r[3], r[2], r[1]}} }
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
	tank := newK(4).
		A("raw", tankRow(passive, shieldRep, armorRep, hullRep)).
		A("effective", tankRow(effectivify(passive, rs), effectivify(shieldRep, rs), effectivify(armorRep, ra), effectivify(hullRep, rh)))
	defense := newK(5).
		A("hp", &fobj{k: keysLayer, v: [5]float64{hpA, hpH, hpS, hpS + hpA + hpH}}).
		A("resonance", newK(3).A("shield", resJ(rs)).A("armor", resJ(ra)).A("hull", resJ(rh))).
		A("ehp", &fobj{k: keysLayer, v: [5]float64{eA, eH, eS, eS + eA + eH}}).
		A("damage_pattern", &fobj{k: keysRes, v: [5]float64{dp.EM, dp.Explosive, dp.Kinetic, dp.Thermal}}).
		A("tank", tank)

	// ---------------- capacitor
	capC := g(ship, "capacitorCapacity")
	rr := g(ship, "rechargeRate")
	peak := 0.0
	if rr > 0 {
		peak = 10 / (rr / 1000) * 0.5 * 0.5 * capC
	}
	var drains []Drain
	var capUsed, capAdded float64
	moduleRows := make([]any, 0, len(modules))
	rowBuf := make([]modRow, len(modules)) // one allocation for all rows
	for _, i := range modules {
		it := &f.Items[i]
		capNeed := f.Get(i, w.capNeed)
		isInj := false
		if gi := ds.group(it.Group); gi != nil && gi.Name == "Capacitor Booster" {
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
		row := &rowBuf[len(moduleRows)]
		*row = modRow{idx: it.ReqIndex, typeID: it.TypeID, name: it.T.Name, slot: it.Slot,
			state: it.State, cpu: f.Get(i, w.cpu), power: f.Get(i, w.power), cycleMs: cycRaw, hasCycle: cycRaw > 0}
		if it.State >= Active && capNeed != 0 && full > 0 {
			// Pyfa forces reload into capacitor boosters' average cycle (module.forceReload)
			avg := f.avgCycleMs(i, factorReload || isInj)
			use := 0.0
			if avg > 0 {
				use = capNeed / (avg / 1000)
			}
			if use > 0 {
				capUsed += use
			} else {
				capAdded -= use
			}
			row.capUse, row.hasCapUse = use, true
			drains = append(drains, Drain{Duration: math.Trunc(full), CapNeed: capNeed, ClipSize: f.numShots(i),
				ReloadMs: f.Get(i, w.reload), IsInjector: isInj, DisableStagger: f.hasEffect(i, w.eTurret)})
		}
		moduleRows = append(moduleRows, row)
	}
	// incoming neuts / nos / cap transfers (Pyfa fit.addDrain): no stagger, after the fit's own modules
	sigNow := g(ship, "signatureRadius")
	for _, ps := range f.ProjSpecials {
		if ps.Rep || ps.Ecm {
			continue
		}
		need := f.Get(ps.Item, ps.Amount) * ps.Factor * ps.Sign
		if ps.Resist != 0 {
			need *= f.Get(ship, ps.Resist)
		}
		if sres := g(ps.Item, "energyNeutralizerSignatureResolution"); sres != 0 {
			need *= min(sigNow/sres, 1)
		}
		if dur := f.Get(ps.Item, ps.Duration); need != 0 && dur > 0 {
			if need > 0 {
				capUsed += need / (math.Trunc(dur) / 1000)
			} else {
				capAdded -= need / (math.Trunc(dur) / 1000)
			}
			drains = append(drains, Drain{Duration: math.Trunc(dur), CapNeed: need})
		}
	}
	capj := newK(11).F("capacity", capC).F("recharge_time_s", rr/1000).F("peak_recharge_gj_s", peak).F("use_gj_s", capUsed).
		F("injected_gj_s", capAdded).F("delta_gj_s", peak+capAdded-capUsed)
	capStable := true
	if len(drains) == 0 {
		capj.A("stable", true).F("stable_percent", 100.0)
	} else {
		o := req.Options.CapSim
		tmax := 6.0 * 3600
		if o.MaxTimeS != nil {
			tmax = *o.MaxTimeS
		}
		r := SimulateCap(capC, rr, drains, 1, o.Reload || factorReload, true, tmax*1000)
		st := (r.StableLow + r.StableHigh) / 2
		stable := r.Stable && st > 0
		capStable = stable
		capj.A("stable", stable)
		if stable {
			capj.F("stable_percent", min(st*100, 100))
		} else {
			capj.F("depletes_in_s", r.TS)
		}
		capj.F("eve_stable_percent", r.EveStable*100).U("sim_iterations", r.Iterations)
	}
	f.sustainableTank(tank, capStable, factorReload, modules,
		[3]float64{shieldRep, armorRep, hullRep}, passive, capUsed, peak+capAdded,
		func(v float64, l int) float64 { return effectivify(v, [3][4]float64{rs, ra, rh}[l]) })

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
	navigation := newK(8).F("max_velocity", maxSpeed).F("align_time_s", -math.Log(0.25)*agility*mass/1e6).F("mass", mass).
		F("agility", agility).F("signature_radius", sig).F("warp_speed_au_s", baseWarp*warpMult).
		F("max_warp_distance_au", maxWarp).F("warp_scramble_status", g(ship, "warpScrambleStatus"))

	// ---------------- targeting
	bestN, bestV := "none", 0.0
	for _, s := range [4][2]string{{"radar", "scanRadarStrength"}, {"ladar", "scanLadarStrength"}, {"magnetometric", "scanMagnetometricStrength"}, {"gravimetric", "scanGravimetricStrength"}} {
		if v := g(ship, s[1]); v > bestV {
			bestN, bestV = s[0], v
		}
	}
	// ECM jam chance (Pyfa Fit.jamChance): strengths vs the strongest sensor type (tie -> multispectral -> 0)
	jam := 0.0
	{
		maxS, ty := -1.0, ""
		for _, t := range [4]string{"Magnetometric", "Ladar", "Radar", "Gravimetric"} {
			if v := g(ship, "scan"+t+"Strength"); v > maxS {
				maxS, ty = v, t
			} else if v == maxS {
				ty = ""
			}
		}
		retain, any := 1.0, false
		for _, ps := range f.ProjSpecials {
			if !ps.Ecm {
				continue
			}
			any = true
			if ty == "" {
				continue
			}
			attr := "scan" + ty + "StrengthBonus"
			if ps.Fighter {
				attr = "fighterAbilityECMStrength" + ty
			}
			st := g(ps.Item, attr) * ps.Factor
			if ps.Resist != 0 {
				if r := f.Get(ship, ps.Resist); r != 0 {
					st *= r
				}
			}
			if maxS > 0 {
				retain *= 1 - min(st/maxS, 1)
			}
		}
		if any {
			jam = (1 - retain) * 100
		}
	}
	scanRes := g(ship, "scanResolution")
	var probe any
	if bestV > 0 {
		probe = max(sig/bestV, 1.08)
	}
	var ltTP any
	if tp.SignatureRadius != nil {
		ltTP = lockTime(scanRes, *tp.SignatureRadius)
	}
	targeting := newK(9).
		F("max_targets", min(g(ship, "maxLockedTargets"), max(g(ch, "maxLockedTargets"), 0))).
		F("max_range_m", g(ship, "maxTargetRange")).F("scan_resolution", scanRes).F("sensor_strength", bestV).S("sensor_type", bestN).
		F("jam_chance_percent", jam).A("probe_size", probe).
		A("lock_time_s", newK(5).A("sig_25m", lockTime(scanRes, 25)).A("sig_40m", lockTime(scanRes, 40)).A("sig_125m", lockTime(scanRes, 125)).
			A("sig_400m", lockTime(scanRes, 400)).A("sig_target_profile", ltTP))
	activeDrones := uint32(0)
	for _, i := range drones {
		activeDrones += f.Items[i].ActiveCount
	}
	dronesJ := newK(3).U("active", uint64(activeDrones)).F("max_active", g(ch, "maxActiveDrones")).F("control_range_m", g(ch, "droneControlDistance"))

	st := f.Items[ship].T
	var grp any
	if gi := ds.group(st.Group); gi != nil {
		grp = gi.Name
	}
	out := obj{
		"meta":      newK(4).U("schema_version", 1).S("engine", engineName).U("sde_build", ds.Build).S("dataset_sha256", ds.SHA256),
		"ship":      newK(3).U("type_id", uint64(st.ID)).S("name", st.Name).A("group", grp),
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
		if ai := f.DS.attr(a); ai != nil {
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
	modules := make([]int, 0, len(f.Items))
	for i := range f.Items {
		if f.Items[i].Kind == KModule {
			modules = append(modules, i)
		}
	}
	slotNamesV := [...]string{SlotHigh: "High", SlotMid: "Mid", SlotLow: "Low", SlotRig: "Rig", SlotSubsystem: "Subsystem", SlotService: "Service"}
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
	groupAttrs, typeAttrs := ds.canFitGroupA, ds.canFitTypeA
	nm := len(modules)
	fittedGroup, fittedType, activeGroup, onlineGroup := make(map[uint32]uint32, nm), make(map[uint32]uint32, nm), make(map[uint32]uint32, nm), make(map[uint32]uint32, nm)
	var grBuf [20]uint32
	var tyBuf [11]uint32
	for _, i := range modules {
		it := &f.Items[i]
		idx := it.ReqIndex
		mt := it.T
		name := mt.Name
		if it.Slot == SlotNone {
			push("NOT_FITTABLE", fmt.Sprintf("%s is not a fittable module", name), idx)
		}
		gr, ty := grBuf[:0], tyBuf[:0]
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
				if x, ok := mt.Attr(ds.AttrID(chargeGroupNames[k-1])); ok && uint32(x) != 0 {
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
		if t := ds.typ(m.s); t != nil {
			sn = t.Name
		}
		push("MISSING_SKILL", fmt.Sprintf("%s %v required by %s", sn, m.need, ds.typ(m.by).Name), -1)
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

// sustainableTank adds tank.sustained{,_effective} (Pyfa Fit.sustainableTank, eos LGPL): when the
// capacitor is unstable or reload is factored, local cap-using repairers run only as far as peak
// recharge plus injected cap allow, most cap-efficient first.
func (f *Fit) sustainableTank(tank *kobj, stable, factorReload bool, modules []int, sus [3]float64, passive, used, totalPeak float64,
	eff func(float64, int) float64) {
	ds, w := f.DS, &f.DS.ids
	g := func(i int, name string) float64 { return f.Get(i, ds.AttrID(name)) }
	if !stable || factorReload {
		spec := func(gname string) (int, string, bool) {
			switch gname {
			case "Shield Booster", "Ancillary Shield Booster":
				return 0, "shieldBonus", true
			case "Armor Repair Unit", "Ancillary Armor Repairer":
				return 1, "armorDamageAmount", true
			case "Hull Repair Unit":
				return 2, "structureDamageAmount", true
			}
			return 0, "", false
		}
		pasteMult := func(i int) float64 {
			if c := f.Items[i].Charge; c >= 0 && f.Items[c].T.Name == "Nanite Repair Paste" {
				if m := g(i, "chargedArmorDamageMultiplier"); m != 0 {
					return m
				}
			}
			return 1
		}
		type rep struct {
			i, l   int
			attr   string
			capUse float64
		}
		var adj [3]float64
		var reps []rep
		for layer := 0; layer < 3; layer++ {
			for _, i := range modules {
				if f.Items[i].State < Active {
					continue
				}
				gname := ""
				if gi := ds.group(f.Items[i].Group); gi != nil {
					gname = gi.Name
				}
				l, attr, ok := spec(gname)
				if !ok || l != layer {
					continue
				}
				capNeed := f.Get(i, w.capNeed)
				avg := f.avgCycleMs(i, factorReload)
				capUse := 0.0
				if capNeed != 0 && avg > 0 {
					capUse = capNeed / (avg / 1000)
				}
				cyc := f.rawCycleMs(i)
				if cyc <= 0 {
					continue
				}
				amount := g(i, attr)
				if capUse != 0 {
					used -= capUse
					adj[l] -= amount * pasteMult(i) / (cyc / 1000)
					reps = append(reps, rep{i, l, attr, capUse})
				} else if gname == "Ancillary Shield Booster" {
					reload := 0.0
					if factorReload && f.Items[i].Charge >= 0 {
						reload = f.Get(i, w.reload)
					}
					shots := float64(max(f.numShots(i), 1))
					off := reload / (shots*cyc + reload)
					adj[l] -= amount * off / (cyc / 1000)
				}
			}
		}
		effic := func(r rep) float64 {
			m := g(r.i, "chargedArmorDamageMultiplier")
			if m == 0 {
				m = 1
			}
			return g(r.i, r.attr) * m / f.Get(r.i, w.capNeed)
		}
		sort.SliceStable(reps, func(a, b int) bool { return effic(reps[a]) > effic(reps[b]) })
		for _, r := range reps {
			if used > totalPeak {
				break
			}
			i := r.i
			reload := 0.0
			if factorReload && f.Items[i].Charge >= 0 {
				reload = f.Get(i, w.reload)
			}
			cyc := f.rawCycleMs(i)
			sustain := min((totalPeak-used)/r.capUse, 1)
			amount := g(i, r.attr)
			if f.Items[i].Charge < 0 {
				adj[r.l] += sustain * amount / (cyc / 1000)
			} else {
				shots := float64(max(f.numShots(i), 1))
				on := shots * cyc / (shots*cyc + reload)
				adj[r.l] += sustain * amount * on * pasteMult(i) / (cyc / 1000)
			}
			used += r.capUse
		}
		for l := range sus {
			sus[l] += adj[l]
		}
	}
	tank.A("sustained", tankRow(passive, sus[0], sus[1], sus[2])).
		A("sustained_effective", tankRow(eff(passive, 0), eff(sus[0], 0), eff(sus[1], 1), eff(sus[2], 2)))
}

var canFitGroupNames, canFitTypeNames, chargeGroupNames = func() (g, t, c []string) {
	for k := 1; k <= 20; k++ {
		g = append(g, fmt.Sprintf("canFitShipGroup%02d", k))
	}
	for k := 1; k <= 11; k++ {
		t = append(t, fmt.Sprintf("canFitShipType%d", k))
	}
	for k := 1; k <= 5; k++ {
		c = append(c, fmt.Sprintf("chargeGroup%d", k))
	}
	return
}()

func (f *Fit) hasEffectNamed(i int, name string) bool {
	for _, e := range f.Items[i].Effects {
		if ef := f.DS.effect(e.ID); ef != nil && ef.Name == name {
			return true
		}
	}
	return false
}
