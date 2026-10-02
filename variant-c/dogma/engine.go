package dogma

import (
	"fmt"
	"math"
	"sort"
	"strconv"
	"strings"
)

// Kind of an item in the fit graph.
type Kind int8

const (
	KShip Kind = iota
	KChar
	KSkill
	KModule
	KCharge
	KDrone
	KFighter
	KImplant
	KBooster
	KMode
	KBeacon
	KProjected
)

// Loc is the location an item lives in (what location-scoped modifiers can reach).
type Loc int8

const (
	LShip Loc = iota
	LChar
	LSpace
	LNowhere
)

const (
	attrSkillLevel    = 280
	effectSkillEffect = 132
)

var hullResonances = [4]uint32{113, 111, 109, 110}

func exemptCategory(c uint32) bool {
	switch c {
	case 6, 8, 16, 20, 32, 65:
		return true
	}
	return false
}

var structureSkillEffectNames = [...]string{
	"targetingMaxTargetBonusModAddMaxLockedTargetsLocationChar",
	"skillStructureMissileDamageBonus",
	"skillStructureElectronicSystemsCapNeedBonus",
	"skillStructureEngineeringSystemsCapNeedBonus",
	"skillStructureDoomsdayDurationBonus",
}

// Item is one node of the fit graph (ship, character, skill, module, charge, drone, ...).
type Item struct {
	T           *TypeInfo
	TypeID      uint32
	Group       uint32
	Category    uint32
	Kind        Kind
	State       State
	Loc         Loc
	Owned       bool
	Parent      int32 // -1 = none
	Charge      int32 // -1 = none
	Slot        Slot
	ReqIndex    int // index in the request list (-1 = none)
	Quantity    uint32
	ActiveCount uint32
	ReqSkills   []uint32
	Effects     []TypeEffect
	Abilities   []uint32 // fighter abilities in use
	HasAbil     bool     // Abilities is meaningful (fighters, incl. projected): filter fighterAbility* effects
	SideEffects []uint32 // booster side effects selected
	Spool       *Spool
	Distance    *float64
	overlay     attrSet // per-item base values that differ from the type (mutations, overrides, skill level)
}

func (it *Item) hasSkill(s uint32) bool {
	for _, x := range it.ReqSkills {
		if x == s {
			return true
		}
	}
	return false
}

// ---- modifier registry ----

type srcKind uint8

const (
	srcAttr srcKind = iota
	srcConst
	srcProp
	srcProj
)

type amod struct {
	op     int8
	pen    bool
	kind   srcKind
	mul    bool
	item   int32 // source item (attr owner)
	ship   int32 // prop: ship ; proj: target
	attr   uint32
	a2, a3 uint32     // prop: thrust, mass ; proj: resist in a2
	c      float64    // const value / projected range factor
	from   int32      // item that registered the modifier
	sel    bucketKind // target selector kind
	x      uint32     // selector argument: item index, group id or skill type id
}

type bucketKind uint8

const (
	bItem bucketKind = iota
	bShipLoc
	bShipGroup
	bShipSkill
	bOwnerSkill
	bCharLoc
	bCharGroup
	bCharSkill
)

// attrMods holds every modifier registered on one attribute id, in registration order. Modifiers
// registered before the skill phase come first (mods[:split]), then the shared skill template, then the rest.
type attrMods struct {
	mods  []amod
	split int
	used  bool // listed in Fit.regUsed
}

// Fit is the evaluated object graph for one request. Not safe for concurrent use.
type Fit struct {
	DS          *Dataset
	Items       []Item
	Ship, Char  int
	Warnings    []string
	IsStructure bool

	reg      []attrMods // dense, indexed by attribute id (len = Dataset.maxAttr+1)
	regUsed  []uint32   // attribute ids with registered modifiers, in first-registration order
	skillTpl [][]amod   // shared, immutable skill modifiers indexed by attr id (nil = skills registered per fit)
	prePhase bool
	// skillsCanonical: items 2.. are exactly ds.PublishedSkills (enables the shared skill template)
	skillsCanonical bool
	cache           nodeCache
	stack           []uint64 // nodes being evaluated (cycle guard + dependency recording)

	// TrackDeps enables reverse-dependency recording so SetBase can invalidate precisely.
	ProjSpecials []ProjSpecial // incoming reps / cap transfers / neuts (feed tank and capsim)

	TrackDeps bool
	rdeps     map[uint64][]uint64

	skillLo, skillHi int // Items[skillLo:skillHi] are the character's skills, sorted by type id

	lvIDs  []uint32 // backing storage for skill-level overlays (reused via the pool)
	lvVals []float64
	noPool bool // the template fit: its registry is shared by every fit, never recycle it
}

// ProjSpecial is a projected effect that does not modify attributes but feeds tank or capacitor stats
// (Pyfa: fit._armorRr, addDrain).
type ProjSpecial struct {
	Rep      bool    // true: remote repair; false: capacitor drain/fill (or ECM when Ecm is set)
	Ecm      bool    // ECM jammer: jam strength vs the target's strongest sensor type
	Fighter  bool    // ECM from a fighter ability (strength attrs fighterAbilityECMStrength*)
	Item     int     // projected item
	Layer    int     // rep: 0 shield, 1 armor, 2 hull
	Amount   uint32  // attribute with the amount per cycle
	Duration uint32  // drain: attribute with the cycle time
	Mult     float64 // rep multiplier (paste)
	Factor   float64 // range factor
	Resist   uint32  // drain: target resistance attribute
	Sign     float64 // drain: +1 drain, -1 fill
}

// EngineError is a request-level error (unknown type etc.).
type EngineError struct {
	Code, Message, Path string
}

func (e *EngineError) Error() string { return e.Code + ": " + e.Message }

func stateOK(cat uint8, s State) bool {
	switch cat {
	case 0, 4:
		return s >= Online
	case 1:
		return s >= Active
	case 5:
		return s >= Overheated
	case 7:
		return true
	}
	return false
}

func nodeKey(item int, attr uint32) uint64 { return uint64(item)<<32 | uint64(attr) }

func (f *Fit) newItem(typeID uint32, kind Kind, loc Loc, path ipath) (int, error) {
	t := f.DS.typ(typeID)
	if t == nil {
		return 0, &EngineError{"UNKNOWN_TYPE", fmt.Sprintf("unknown type_id %d", typeID), path.String()}
	}
	owned := false
	switch kind {
	case KModule, KCharge, KDrone, KFighter, KShip:
		owned = true
	}
	f.Items = append(f.Items, Item{T: t, TypeID: typeID, Group: t.Group, Category: t.Category, Kind: kind,
		State: Online, Loc: loc, Owned: owned, Parent: -1, Charge: -1, ReqIndex: -1, Quantity: 1,
		ReqSkills: t.ReqSkills, Effects: t.Effects})
	return len(f.Items) - 1, nil
}

func (f *Fit) setOverlay(i int, attr uint32, v float64) { f.Items[i].overlay.set(attr, v) }

func (f *Fit) applyMutation(idx int, m *Mutation) {
	ds := f.DS
	it := &f.Items[idx]
	if base := ds.typ(m.BaseTypeID); base != nil {
		own := it.T
		for k, a := range base.raw.ids {
			it.overlay.set(a, base.raw.vals[k])
		}
		for k, a := range own.raw.ids {
			it.overlay.set(a, own.raw.vals[k])
		}
		effs := append([]TypeEffect(nil), it.Effects...)
		for _, e := range base.Effects {
			if !own.HasEffect(e.ID) {
				effs = append(effs, e)
			}
		}
		it.Effects = effs
		if len(it.ReqSkills) == 0 {
			it.ReqSkills = base.ReqSkills
		}
		if f.baseOf(idx, 4) == 0 && base.Mass != 0 {
			it.overlay.set(4, base.Mass)
		}
	}
	var muta *MutaInfo
	if m.MutaplasmidTypeID != nil {
		muta = ds.Mutaplasmids[*m.MutaplasmidTypeID]
	}
	baseT := ds.typ(m.BaseTypeID)
	keys := make([]string, 0, len(m.Attributes))
	for k := range m.Attributes {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	for _, k := range keys {
		v := m.Attributes[k]
		aid := u32(k)
		if aid == 0 && k != "0" {
			continue
		}
		if muta != nil && baseT != nil {
			rng, ok1 := muta.Attrs[k]
			bv, ok2 := baseT.raw.get(aid)
			if ok1 && ok2 {
				a, b := bv*rng[0], bv*rng[1]
				mn, mx := a, b
				if b < a {
					mn, mx = b, a
				}
				if bv != 0 {
					v = min(max(v, mn), mx)
				}
			}
		}
		it.overlay.set(aid, v)
	}
}

func (f *Fit) addModule(i int, m *ModuleReq, path ipath) error {
	idx, err := f.newItem(m.TypeID, KModule, LShip, path)
	if err != nil {
		return err
	}
	it := &f.Items[idx]
	slot := it.T.Slot
	if m.Slot != nil {
		slot = *m.Slot
	}
	it.Slot = slot
	it.ReqIndex = i
	it.Spool = m.Spool
	it.State = Online
	if m.State != nil {
		it.State = *m.State
	}
	if (slot == SlotRig || slot == SlotSubsystem) && it.State != Offline {
		it.State = Online
	}
	if m.Mutation != nil {
		f.applyMutation(idx, m.Mutation)
	}
	if m.ChargeTypeID != nil {
		c, err := f.newItem(*m.ChargeTypeID, KCharge, LShip, path.with("/charge_type_id"))
		if err != nil {
			return err
		}
		f.Items[c].Parent = int32(idx)
		f.Items[c].ReqIndex = i
		f.Items[idx].Charge = int32(c)
	}
	return nil
}

// Build constructs the object graph for a request and registers all modifiers. Nothing is evaluated
// except what the Reactive Armor Hardener simulation needs.
func Build(ds *Dataset, req *FitRequest) (*Fit, error) {
	nItems := 2 + len(ds.PublishedSkills) + 2*len(req.Modules) + len(req.Drones) + len(req.Fighters) + len(req.Implants) + len(req.Boosters) + 4
	f := acquireFit(ds, nItems)
	ship, err := f.newItem(req.Ship.TypeID, KShip, LShip, ipath{"/ship/type_id", -1, ""})
	if err != nil {
		return nil, err
	}
	f.Ship = ship
	f.IsStructure = f.Items[ship].Category == 65
	ch, err := f.newItem(1373, KChar, LChar, ipath{"/character", -1, ""})
	if err != nil {
		return nil, err
	}
	f.Char = ch
	if req.Character.SecurityStatus != nil {
		if a := ds.AttrID("pilotSecurityStatus"); a != 0 {
			f.setOverlay(ch, a, *req.Character.SecurityStatus)
		}
	}
	// skills: every published skill exists (untrained = level 0)
	def := uint8(0)
	if req.Character.Skills.DefaultLevel != nil {
		def = *req.Character.Skills.DefaultLevel
	}
	var extra map[uint32]uint8
	if len(req.Character.Skills.Levels) > 0 {
		extra = make(map[uint32]uint8, len(req.Character.Skills.Levels))
		for k, v := range req.Character.Skills.Levels {
			if id := u32(k); id != 0 {
				extra[id] = v
			} else if id, ok := ds.TypeByName(k); ok {
				extra[id] = v
			}
		}
	}
	skillIDs := ds.PublishedSkills
	f.skillsCanonical = true
	if len(extra) > 0 {
		set := make(map[uint32]bool, len(skillIDs)+len(extra))
		for _, s := range skillIDs {
			set[s] = true
		}
		for s := range extra {
			if !set[s] {
				f.skillsCanonical = false
				skillIDs = append(append([]uint32(nil), skillIDs...), s)
				set[s] = true
			}
		}
		sort.Slice(skillIDs, func(i, j int) bool { return skillIDs[i] < skillIDs[j] })
	}
	// one backing array for all skill-level overlays (avoids 2 allocations per skill)
	f.skillLo = len(f.Items)
	if cap(f.lvIDs) < len(skillIDs) {
		f.lvIDs, f.lvVals = make([]uint32, len(skillIDs)), make([]float64, len(skillIDs))
	}
	lvIDs, lvVals := f.lvIDs[:len(skillIDs)], f.lvVals[:len(skillIDs)]
	level := func(s uint32) float64 {
		l, ok := extra[s]
		if !ok {
			l = def
		}
		return float64(min(l, 5))
	}
	if tpl := ds.skillItemTemplate(); f.skillsCanonical && len(tpl) == len(skillIDs) {
		// canonical skill set: copy the prebuilt items (one memmove), then set each level overlay
		f.Items = append(f.Items, tpl...)
		sk := f.Items[f.skillLo:]
		for k := range sk {
			lvIDs[k], lvVals[k] = attrSkillLevel, level(sk[k].TypeID)
			sk[k].overlay = attrSet{lvIDs[k : k+1 : k+1], lvVals[k : k+1 : k+1]}
		}
	} else {
		for k, s := range skillIDs {
			if ds.typ(s) == nil {
				continue
			}
			idx, _ := f.newItem(s, KSkill, LChar, ipath{"/character/skills", -1, ""})
			lvIDs[k], lvVals[k] = attrSkillLevel, level(s)
			f.Items[idx].overlay = attrSet{lvIDs[k : k+1 : k+1], lvVals[k : k+1 : k+1]}
			f.Items[idx].Owned = false
		}
	}
	f.skillHi = len(f.Items)
	// tactical destroyer default mode
	var modeID *uint32
	if req.Ship.ModeTypeID != nil {
		modeID = req.Ship.ModeTypeID
	} else if st := ds.typ(req.Ship.TypeID); st != nil {
		sn := strings.ToLower(st.Name)
		for k, id := range ds.modeTypes {
			if strings.HasPrefix(ds.modeNames[k], sn) {
				m := id
				modeID = &m
				f.Warnings = append(f.Warnings, fmt.Sprintf("no tactical mode given; defaulted to type %d", m))
				break
			}
		}
	}
	if modeID != nil {
		idx, err := f.newItem(*modeID, KMode, LNowhere, ipath{"/ship/mode_type_id", -1, ""})
		if err != nil {
			return nil, err
		}
		f.Items[idx].Owned = false
	}
	for i := range req.Modules {
		if err := f.addModule(i, &req.Modules[i], ipath{"/modules/", i, ""}); err != nil {
			return nil, err
		}
	}
	for i := range req.Drones {
		d := &req.Drones[i]
		idx, err := f.newItem(d.TypeID, KDrone, LSpace, ipath{"/drones/", i, ""})
		if err != nil {
			return nil, err
		}
		if d.Mutation != nil {
			f.applyMutation(idx, d.Mutation)
		}
		it := &f.Items[idx]
		it.Quantity = max(d.Quantity, 1)
		if d.Active != nil {
			it.ActiveCount = min(*d.Active, it.Quantity)
		}
		it.State = Offline
		if it.ActiveCount > 0 {
			it.State = Active
		}
		it.ReqIndex = i
	}
	sqAttr := ds.AttrID("fighterSquadronMaxSize")
	for i := range req.Fighters {
		fr := &req.Fighters[i]
		idx, err := f.newItem(fr.TypeID, KFighter, LSpace, ipath{"/fighters/", i, ""})
		if err != nil {
			return nil, err
		}
		maxsq := uint32(1)
		if v, ok := f.baseOK(idx, sqAttr); ok {
			maxsq = uint32(v)
		}
		it := &f.Items[idx]
		q := maxsq
		if fr.Quantity != nil {
			q = *fr.Quantity
		}
		it.Quantity = min(max(q, 1), max(maxsq, 1))
		if fr.Quantity != nil && *fr.Quantity > maxsq {
			f.Warnings = append(f.Warnings, fmt.Sprintf("fighters/%d: squadron size %d capped to %d", i, *fr.Quantity, maxsq))
		}
		if fr.Active {
			it.ActiveCount, it.State = it.Quantity, Active
		} else {
			it.ActiveCount, it.State = 0, Offline
		}
		if fr.Abilities != nil {
			it.Abilities = *fr.Abilities
		} else {
			it.Abilities = defaultFighterAbilities(ds, it.Effects)
		}
		it.HasAbil = true
		it.ReqIndex = i
	}
	for i, imp := range req.Implants {
		idx, err := f.newItem(imp, KImplant, LChar, ipath{"/implants/", i, ""})
		if err != nil {
			return nil, err
		}
		f.Items[idx].Owned = false
		f.Items[idx].ReqIndex = i
	}
	for i := range req.Boosters {
		b := &req.Boosters[i]
		idx, err := f.newItem(b.TypeID, KBooster, LChar, ipath{"/boosters/", i, ""})
		if err != nil {
			return nil, err
		}
		f.Items[idx].Owned = false
		f.Items[idx].SideEffects = b.SideEffects
		f.Items[idx].ReqIndex = i
	}
	for i, e := range req.Environment.EffectTypeIDs {
		idx, err := f.newItem(e, KBeacon, LNowhere, ipath{"/environment/effect_type_ids/", i, ""})
		if err != nil {
			return nil, err
		}
		f.Items[idx].Owned = false
	}
	for i := range req.Projected {
		p := &req.Projected[i]
		switch p.Kind {
		case "module":
			if p.Module != nil {
				for k := uint32(0); k < max(p.Amount, 1); k++ {
					idx, err := f.newItem(p.Module.TypeID, KProjected, LNowhere, ipath{"/projected/", i, ""})
					if err != nil {
						return nil, err
					}
					it := &f.Items[idx]
					it.Owned = false
					it.State = Active
					if p.Module.State != nil {
						it.State = *p.Module.State
					}
					it.Distance = p.DistanceM
					it.ReqIndex = i
					if p.Module.ChargeTypeID != nil {
						c, err := f.newItem(*p.Module.ChargeTypeID, KCharge, LNowhere, ipath{"/projected/", i, "/module/charge_type_id"})
						if err != nil {
							return nil, err
						}
						f.Items[c].Parent = int32(idx)
						f.Items[c].Owned = false
						f.Items[idx].Charge = int32(c)
					}
				}
			}
		case "drone":
			if p.Drone != nil {
				for k := uint32(0); k < max(p.Amount, 1)*max(p.Drone.Quantity, 1); k++ {
					idx, err := f.newItem(p.Drone.TypeID, KProjected, LNowhere, ipath{"/projected/", i, ""})
					if err != nil {
						return nil, err
					}
					it := &f.Items[idx]
					it.Owned = false
					it.State = Active
					it.Distance = p.DistanceM
				}
			}
		case "fighter":
			if fr := p.Fighter; fr != nil {
				for k := uint32(0); k < max(p.Amount, 1); k++ {
					idx, err := f.newItem(fr.TypeID, KProjected, LNowhere, ipath{"/projected/", i, ""})
					if err != nil {
						return nil, err
					}
					maxsq := uint32(1)
					if v, ok := f.baseOK(idx, ds.AttrID("fighterSquadronMaxSize")); ok {
						maxsq = max(uint32(v), 1)
					}
					it := &f.Items[idx]
					it.Owned = false
					it.State = Offline
					if fr.Active {
						it.State = Active
					}
					q := maxsq
					if fr.Quantity != nil {
						q = *fr.Quantity
					}
					it.Quantity = min(max(q, 1), maxsq)
					it.ActiveCount = it.Quantity
					it.Distance = p.DistanceM
					it.ReqIndex = i
					if fr.Abilities != nil {
						it.Abilities = *fr.Abilities
					} else {
						it.Abilities = defaultFighterAbilities(ds, it.Effects)
					}
					it.HasAbil = true
				}
			}
		case "fit":
			// whole projected fit: compute the source fit on its own, then project each active module / drone as
			// a frozen item carrying the source-modified values.
			if p.Fit != nil {
				sreq := *p.Fit
				sreq.Projected = nil
				src, err := Build(ds, &sreq)
				if err != nil {
					f.Warnings = append(f.Warnings, fmt.Sprintf("projected[%d] fit: %v", i, err))
					continue
				}
				type frozen struct {
					typeID  uint32
					copies  uint32
					vals    attrSet
					fighter bool
					qty     uint32
					abil    []uint32
				}
				var fr []frozen
				for si := range src.Items {
					it := &src.Items[si]
					copies := uint32(0)
					switch {
					case it.Kind == KModule && it.State >= Active:
						copies = 1
					case it.Kind == KDrone:
						copies = it.ActiveCount
					case it.Kind == KFighter && it.State >= Active:
						copies = 1
					}
					if copies == 0 {
						continue
					}
					ids := src.AttrIDs(si)
					vals := make([]float64, len(ids))
					for k, a := range ids {
						vals[k] = src.Get(si, a)
					}
					fr = append(fr, frozen{it.TypeID, copies, attrSet{ids, vals}, it.Kind == KFighter, it.Quantity, it.Abilities})
				}
				src.Release()
				for _, z := range fr {
					for k := uint32(0); k < z.copies*max(p.Amount, 1); k++ {
						idx, err := f.newItem(z.typeID, KProjected, LNowhere, ipath{"/projected/", i, ""})
						if err != nil {
							return nil, err
						}
						it := &f.Items[idx]
						it.Owned = false
						it.State = Active
						it.Distance = p.DistanceM
						it.ReqIndex = i
						it.overlay = attrSet{append([]uint32(nil), z.vals.ids...), append([]float64(nil), z.vals.vals...)}
						if z.fighter {
							it.Quantity, it.ActiveCount = z.qty, z.qty
							it.Abilities, it.HasAbil = z.abil, true
						}
					}
				}
			}
		default:
			f.Warnings = append(f.Warnings, fmt.Sprintf("projected kind '%s' not supported yet (index %d)", p.Kind, i))
		}
	}
	// system security -> securityModifier (default nullsec, like Pyfa)
	{
		sec := "nullsec"
		if req.Environment.SystemSecurity != nil {
			sec = strings.ToLower(*req.Environment.SystemSecurity)
		}
		var src string
		switch sec {
		case "hisec", "highsec", "high":
			src = "hiSecModifier"
		case "lowsec", "low":
			src = "lowSecModifier"
		case "nullsec", "null", "wspace", "wormhole", "w-space":
			src = "nullSecModifier"
		default:
			f.Warnings = append(f.Warnings, fmt.Sprintf("unknown system_security '%s', using nullsec", sec))
			src = "nullSecModifier"
		}
		srcID, dstID := ds.AttrID(src), ds.AttrID("securityModifier")
		for i := range f.Items {
			if v, ok := f.baseOK(i, srcID); ok {
				f.setOverlay(i, dstID, v)
			}
		}
	}
	for _, o := range req.Overrides {
		for i := range f.Items {
			if f.Items[i].TypeID == o.TypeID {
				f.setOverlay(i, o.AttributeID, o.Value)
			}
		}
	}
	f.registerAll(req)
	f.applyRAH(req)
	return f, nil
}

func defaultFighterAbilities(ds *Dataset, effs []TypeEffect) []uint32 {
	ids := make([]uint32, 0, len(effs))
	for _, e := range effs {
		ids = append(ids, e.ID)
	}
	sort.Slice(ids, func(i, j int) bool { return ids[i] < ids[j] })
	on := []uint32{}
	stdSeen := false
	for _, e := range ids {
		ei := ds.effect(e)
		if ei == nil || !strings.HasPrefix(ei.Name, "fighterAbility") {
			continue
		}
		switch {
		case ei.Name == "fighterAbilityAttackM":
			on = append(on, e)
			stdSeen = true
		case !stdSeen && ei.Name != "fighterAbilityMicroWarpDrive" && ei.Name != "fighterAbilityEvasiveManeuvers" && ei.Name != "fighterAbilityMicroJumpDrive":
			on = append(on, e)
		}
	}
	return on
}

// ---------------------------------------------------------------- registration

func (f *Fit) push(k bucketKind, x uint32, attr uint32, m amod, sourceCat uint32) {
	stackable := true
	if a := f.DS.attr(attr); a != nil {
		stackable = a.Stackable
	}
	m.pen = !stackable && !exemptCategory(sourceCat)
	m.sel, m.x = k, x
	if int(attr) >= len(f.reg) { // attribute id outside the dataset's attribute table
		f.reg = append(f.reg, make([]attrMods, int(attr)+1-len(f.reg))...)
	}
	am := &f.reg[attr]
	if !am.used {
		am.used = true
		f.regUsed = append(f.regUsed, attr)
	}
	am.mods = append(am.mods, m)
	if f.prePhase {
		am.split = len(am.mods)
	}
}

// selector maps (func, domain, extra) of a modifier declared on item src to a registry bucket.
func (f *Fit) selector(src int, fn, dom int8, extra uint32) (bucketKind, uint32, bool) {
	s := &f.Items[src]
	switch dom {
	case DomItem:
		if fn == FuncItem {
			return bItem, uint32(src), true
		}
	case DomOther:
		if s.Charge >= 0 {
			return bItem, uint32(s.Charge), true
		} else if s.Parent >= 0 {
			return bItem, uint32(s.Parent), true
		}
	case DomShip, DomStructure:
		if dom == DomStructure && !f.IsStructure {
			return 0, 0, false
		}
		switch fn {
		case FuncItem:
			return bItem, uint32(f.Ship), true
		case FuncLocation:
			return bShipLoc, 0, true
		case FuncLocationGroup:
			return bShipGroup, extra, true
		case FuncLocationRequiredSkill:
			return bShipSkill, extra, true
		case FuncOwnerRequiredSkill:
			return bOwnerSkill, extra, true
		}
	case DomChar:
		switch fn {
		case FuncItem:
			return bItem, uint32(f.Char), true
		case FuncLocation:
			return bCharLoc, 0, true
		case FuncLocationGroup:
			return bCharGroup, extra, true
		case FuncLocationRequiredSkill, FuncOwnerRequiredSkill:
			return bCharSkill, extra, true
		}
	}
	return 0, 0, false
}

func (f *Fit) effectiveState(i int) State {
	it := &f.Items[i]
	switch it.Kind {
	case KCharge:
		if it.Parent >= 0 {
			return f.Items[it.Parent].State
		}
		return Online
	case KShip, KChar, KSkill, KImplant, KBooster, KMode, KBeacon:
		return Online
	case KDrone, KFighter:
		if it.ActiveCount > 0 {
			return Active
		}
		return Offline
	}
	return it.State
}

func containsU32(l []uint32, v uint32) bool {
	for _, x := range l {
		if x == v {
			return true
		}
	}
	return false
}

func (f *Fit) registerAll(req *FitRequest) {
	tpl := f.DS.skillTemplate()
	useTpl := !f.IsStructure && f.skillsCanonical
	f.prePhase = true
	for i := range f.Items {
		if i == 2 {
			f.prePhase = false
			if useTpl {
				f.skillTpl = tpl
			}
		}
		if useTpl && f.Items[i].Kind == KSkill {
			continue
		}
		f.registerItem(i)
	}
	f.prePhase = false
	f.registerBuffs(req)
}

// registerItem registers all modifiers sourced by item i.
func (f *Fit) registerItem(i int) {
	ds := f.DS
	id := &ds.ids
	it := &f.Items[i]
	kind := it.Kind
	if kind == KProjected {
		f.registerProjected(i)
		return
	}
	if f.IsStructure && (kind == KDrone || kind == KImplant || kind == KBooster) {
		return
	}
	state := f.effectiveState(i)
	srcCat := it.Category
	for _, te := range it.Effects {
		eid := te.ID
		if eid == effectSkillEffect {
			continue
		}
		e := ds.effect(eid)
		if e == nil {
			continue
		}
		if f.IsStructure && kind == KSkill && !containsU32(id.structureSkillEffects[:], eid) {
			allItem := true
			for _, m := range e.Mods {
				if m.Domain != DomItem {
					allItem = false
					break
				}
			}
			if !allItem {
				continue
			}
		}
		if e.FittingChance != 0 && !containsU32(it.SideEffects, eid) {
			continue
		}
		if kind == KFighter && e.Category != 0 && !containsU32(it.Abilities, eid) {
			continue
		}
		// Pyfa 'active' handlers for SDE effects without modifiers (some are target-category in the SDE)
		if len(e.Mods) == 0 && kind == KModule && state >= Active && f.localSpecial(i, e.Name, srcCat) {
			continue
		}
		if kind == KBeacon && e.Name == "OffensiveDefensiveReduction" {
			f.incursionEffect(i)
			continue
		}
		if !stateOK(e.Category, state) {
			continue
		}
		if kind == KFighter && len(e.Mods) == 0 {
			// fighter self abilities (Pyfa hand-written handlers, eos LGPL)
			if fm := fighterSelfMods[e.Name]; fm != nil {
				for _, x := range fm {
					f.push(bItem, uint32(i), ds.AttrID(x.target), amod{op: x.op, kind: srcAttr, item: int32(i), attr: ds.AttrID(x.src), from: int32(i)}, srcCat)
				}
				continue
			}
		}
		ship := uint32(f.Ship)
		switch eid {
		case id.eAB, id.eMWD:
			f.push(bItem, ship, 4, amod{op: 2, kind: srcAttr, item: int32(i), attr: id.massAddition, from: int32(i)}, srcCat)
			f.push(bItem, ship, id.maxVelocity, amod{op: 4, kind: srcProp, item: int32(i), ship: int32(f.Ship),
				attr: id.speedFactor, a2: id.speedBoostFactor, a3: 4, from: int32(i)}, srcCat)
			if eid == id.eMWD {
				f.push(bItem, ship, id.signatureRadius, amod{op: 6, kind: srcAttr, item: int32(i), attr: id.signatureRadiusBonus, from: int32(i)}, srcCat)
			}
			continue
		case id.eMJD:
			f.push(bItem, ship, id.signatureRadius, amod{op: 6, kind: srcAttr, item: int32(i), attr: id.signatureRadiusBonusPercent, from: int32(i)}, 6)
			continue
		case id.eSlot:
			for k := 0; k < 3; k++ {
				f.push(bItem, ship, id.slotAttrs[k], amod{op: 2, kind: srcAttr, item: int32(i), attr: id.slotModAttrs[k], from: int32(i)}, srcCat)
			}
			continue
		case id.eHardpoint:
			for k := 0; k < 2; k++ {
				f.push(bItem, ship, id.hpAttrs[k], amod{op: 2, kind: srcAttr, item: int32(i), attr: id.hpModAttrs[k], from: int32(i)}, srcCat)
			}
			continue
		}
		for _, m := range e.Mods {
			if m.Func == FuncEffectStopper || m.Op == 9 || m.Domain == DomTargetID || m.Domain == DomTarget {
				continue
			}
			extra := m.Extra
			if extra == 0 && (m.Func == FuncLocationRequiredSkill || m.Func == FuncOwnerRequiredSkill) {
				extra = it.TypeID // EXCT convention: skill filter 0 = the type owning the effect
			}
			bk, x, ok := f.selector(i, m.Func, m.Domain, extra)
			if !ok {
				continue
			}
			cat := srcCat
			if eid == id.eBastion && (m.Modified == hullResonances[0] || m.Modified == hullResonances[1] || m.Modified == hullResonances[2] || m.Modified == hullResonances[3]) {
				cat = 6
			}
			f.push(bk, x, m.Modified, amod{op: int8(m.Op), kind: srcAttr, item: int32(i), attr: m.Modifying, from: int32(i)}, cat)
		}
	}
}

type selfMod struct {
	target, src string
	op          int8
}

var fighterSelfMods = map[string][]selfMod{
	"fighterAbilityMicroWarpDrive": {{"maxVelocity", "fighterAbilityMicroWarpDriveSpeedBonus", 6},
		{"signatureRadius", "fighterAbilityMicroWarpDriveSignatureRadiusBonus", 6}},
	"fighterAbilityAfterburner": {{"maxVelocity", "fighterAbilityAfterburnerSpeedBonus", 6}},
	"fighterAbilityEvasiveManeuvers": {{"maxVelocity", "fighterAbilityEvasiveManeuversSpeedBonus", 6},
		{"signatureRadius", "fighterAbilityEvasiveManeuversSignatureRadiusBonus", 6},
		{"shieldEmDamageResonance", "fighterAbilityEvasiveManeuversEmResonance", 4},
		{"shieldThermalDamageResonance", "fighterAbilityEvasiveManeuversThermResonance", 4},
		{"shieldKineticDamageResonance", "fighterAbilityEvasiveManeuversKinResonance", 4},
		{"shieldExplosiveDamageResonance", "fighterAbilityEvasiveManeuversExpResonance", 4}},
}

func (f *Fit) registerProjected(i int) {
	ds := f.DS
	it := &f.Items[i]
	srcCat := it.Category
	ship := uint32(f.Ship)
	for _, te := range it.Effects {
		e := ds.effect(te.ID)
		if e == nil || (e.Category != 2 && e.Category != 3 && e.Name != "ECMBurstJammer" && !strings.HasPrefix(e.Name, "doomsdayAOE")) {
			continue
		}
		if it.HasAbil && strings.HasPrefix(e.Name, "fighterAbility") && !containsU32(it.Abilities, te.ID) {
			continue
		}
		if it.State < Active {
			continue
		}
		opt, fo := 0.0, 0.0
		if e.RangeAttr != 0 {
			opt, _ = f.baseOK(i, e.RangeAttr)
		}
		if e.FalloffAttr != 0 {
			fo, _ = f.baseOK(i, e.FalloffAttr)
		}
		factor := RangeFactor(opt, fo, it.Distance, true)
		resist := e.ResistanceAttr
		if resist == 0 {
			look := func(n string) uint32 { v, _ := f.baseOK(i, ds.AttrID(n)); return uint32(v) }
			if strings.HasPrefix(e.Name, "fighterAbility") {
				if resist = look(e.Name + "ResistanceID"); resist == 0 {
					resist = look(e.Name + "RemoteResistanceID")
				}
			} else {
				resist = look("remoteResistanceID")
			}
		}
		targetOffenseOK := true
		if v, ok := f.baseOK(f.Ship, ds.AttrID("disallowOffensiveModifiers")); ok {
			targetOffenseOK = v == 0
		}
		qty := float64(max(it.Quantity, 1))
		push := func(target, src uint32, op int8) {
			f.push(bItem, ship, target, amod{op: op, kind: srcProj, item: int32(i), ship: int32(f.Ship), attr: src,
				c: factor, a2: resist, mul: op == 4 || op == 0, from: int32(i)}, srcCat)
		}
		// burst projectors and the Standup weapon disruptor stay engine-side even if a dataset revision gives them
		// modifiers (eve-sde-pipeline proposed patch 0101): the generic path has no AoE full-strength rule
		engineSide := strings.HasPrefix(e.Name, "doomsdayAOE") || e.Name == "structureModuleEffectWeaponDisruption"
		if len(e.Mods) > 0 && !engineSide {
			for _, m := range e.Mods {
				if (m.Domain == DomTargetID || m.Domain == DomTarget || m.Domain == DomShip) && m.Func == FuncItem {
					push(m.Modified, m.Modifying, int8(m.Op))
				}
			}
			continue
		}
		a := ds.AttrID
		n := e.Name
		pbase := func(name string) float64 { v, _ := f.baseOK(i, a(name)); return v }
		switch n {
		case "fighterAbilityStasisWebifier":
			if targetOffenseOK {
				fac := RangeFactor(pbase("fighterAbilityStasisWebifierOptimalRange"), pbase("fighterAbilityStasisWebifierFalloffRange"), it.Distance, true) * qty
				f.push(bItem, ship, a("maxVelocity"), amod{op: 6, kind: srcProj, item: int32(i), ship: int32(f.Ship),
					attr: a("fighterAbilityStasisWebifierSpeedPenalty"), c: fac, a2: resist, from: int32(i)}, srcCat)
			}
			continue
		case "fighterAbilityWarpDisruption":
			d := 0.0
			if it.Distance != nil {
				d = *it.Distance
			}
			if targetOffenseOK && pbase("fighterAbilityWarpDisruptionRange") >= d {
				f.push(bItem, ship, a("warpScrambleStatus"), amod{op: 2, kind: srcProj, item: int32(i), ship: int32(f.Ship),
					attr: a("fighterAbilityWarpDisruptionPointStrength"), c: qty, a2: resist, from: int32(i)}, srcCat)
			}
			continue
		}
		// burst projectors (Pyfa Effect6476-6482/6513): full strength on every ship in the AoE (no range factor)
		full := func(target, src uint32) {
			f.push(bItem, ship, target, amod{op: 6, kind: srcProj, item: int32(i), ship: int32(f.Ship), attr: src, c: 1, a2: resist, from: int32(i)}, srcCat)
		}
		switch n {
		case "doomsdayAOEWeb":
			if targetOffenseOK {
				full(a("maxVelocity"), a("speedFactor"))
			}
			continue
		case "doomsdayAOEPaint":
			if targetOffenseOK {
				full(a("signatureRadius"), a("signatureRadiusBonus"))
			}
			continue
		case "doomsdayAOEDamp":
			if targetOffenseOK {
				full(a("maxTargetRange"), a("maxTargetRangeBonus"))
				full(a("scanResolution"), a("scanResolutionBonus"))
			}
			continue
		case "doomsdayAOENeut":
			f.ProjSpecials = append(f.ProjSpecials, ProjSpecial{Item: i, Amount: a("energyNeutralizerAmount"), Duration: a("duration"), Factor: 1, Resist: resist, Sign: 1})
			continue
		case "doomsdayAOEECM":
			if targetOffenseOK {
				f.ProjSpecials = append(f.ProjSpecials, ProjSpecial{Ecm: true, Item: i, Factor: 1, Resist: resist})
			}
			continue
		case "doomsdayAOEBubble", "doomsdayAOEGuide":
			continue
		}
		weaponDisruption := n == "doomsdayAOETrack" || n == "structureModuleEffectWeaponDisruption"
		switch {
		case weaponDisruption:
			// AoE weapon disruption burst (full strength) / Standup Weapon Disruptor (range factor): turrets and missiles
			if targetOffenseOK {
				tf := 1.0
				if n != "doomsdayAOETrack" {
					tf = RangeFactor(pbase("maxRange"), pbase("falloffEffectiveness"), it.Distance, true)
				}
				gun, _ := ds.TypeByName("Gunnery")
				mls, _ := ds.TypeByName("Missile Launcher Operation")
				for t := range f.Items {
					ti := &f.Items[t]
					if ti.Loc != LShip || !ti.Owned {
						continue
					}
					var pairs [][2]string
					if ti.Kind == KModule && containsU32(ti.ReqSkills, gun) {
						pairs = [][2]string{{"trackingSpeedBonus", "trackingSpeed"}, {"maxRangeBonus", "maxRange"}, {"falloffBonus", "falloff"}}
					} else if ti.Kind == KCharge && containsU32(ti.ReqSkills, mls) {
						pairs = [][2]string{{"aoeCloudSizeBonus", "aoeCloudSize"}, {"aoeVelocityBonus", "aoeVelocity"}, {"missileVelocityBonus", "maxVelocity"}, {"explosionDelayBonus", "explosionDelay"}}
					} else {
						continue
					}
					for _, pr := range pairs {
						f.push(bItem, uint32(t), a(pr[1]), amod{op: 6, kind: srcProj, item: int32(i), ship: int32(f.Ship),
							attr: a(pr[0]), c: tf, a2: resist, from: int32(i)}, srcCat)
					}
				}
			}
		case strings.HasPrefix(n, "remoteWebifier") || n == "structureModuleEffectStasisWebifier":
			push(a("maxVelocity"), a("speedFactor"), 6)
		case strings.HasPrefix(n, "remoteTargetPaint") || n == "structureModuleEffectTargetPainter":
			push(a("signatureRadius"), a("signatureRadiusBonus"), 6)
		case strings.HasPrefix(n, "remoteSensorDamp") || n == "structureModuleEffectRemoteSensorDampener":
			push(a("maxTargetRange"), a("maxTargetRangeBonus"), 6)
			push(a("scanResolution"), a("scanResolutionBonus"), 6)
		case n == "shipModuleTrackingDisruptor" || n == "shipModuleGuidanceDisruptor" || n == "shipModuleRemoteTrackingComputer" || n == "npcEntityWeaponDisruptor":
			// Pyfa Effect6424 / Effect6423 / shipModuleRemoteTrackingComputer: the target's gunnery
			// modules (TD, RTC) / missile charges (GD). RTCs are assistance-gated, TD/GD offense-gated.
			allowed := targetOffenseOK
			if n == "shipModuleRemoteTrackingComputer" {
				allowed = true
				if v, ok := f.baseOK(f.Ship, a("disallowAssistance")); ok {
					allowed = v == 0
				}
			}
			if allowed {
				skill, kind := "Gunnery", KModule
				pairs := [][2]string{{"trackingSpeedBonus", "trackingSpeed"}, {"maxRangeBonus", "maxRange"}, {"falloffBonus", "falloff"}}
				if n == "shipModuleGuidanceDisruptor" {
					skill, kind = "Missile Launcher Operation", KCharge
					pairs = [][2]string{{"aoeCloudSizeBonus", "aoeCloudSize"}, {"aoeVelocityBonus", "aoeVelocity"}, {"missileVelocityBonus", "maxVelocity"}, {"explosionDelayBonus", "explosionDelay"}}
				}
				sk, _ := ds.TypeByName(skill)
				var tf float64
				if n == "npcEntityWeaponDisruptor" {
					// TD drones (Pyfa Effect6694): full strength inside maxRange, nothing beyond
					tf = 1
					if it.Distance != nil && pbase("maxRange") < *it.Distance {
						tf = 0
					}
				} else {
					tf = RangeFactor(pbase("maxRange"), pbase("falloffEffectiveness"), it.Distance, true)
				}
				for t := range f.Items {
					ti := &f.Items[t]
					if ti.Loc != LShip || !ti.Owned || ti.Kind != kind || !containsU32(ti.ReqSkills, sk) {
						continue
					}
					for _, pr := range pairs {
						f.push(bItem, uint32(t), a(pr[1]), amod{op: 6, kind: srcProj, item: int32(i), ship: int32(f.Ship),
							attr: a(pr[0]), c: tf, a2: resist, from: int32(i)}, srcCat)
					}
				}
			}
		case strings.HasPrefix(n, "remoteSensorBoost"):
			push(a("maxTargetRange"), a("maxTargetRangeBonus"), 6)
			push(a("scanResolution"), a("scanResolutionBonus"), 6)
			for _, t := range [4]string{"Gravimetric", "Ladar", "Magnetometric", "Radar"} {
				push(a("scan"+t+"Strength"), a("scan"+t+"StrengthPercent"), 6)
			}
		default:
			if ps, ok := f.projSpecialFor(i, n, resist); ok {
				f.ProjSpecials = append(f.ProjSpecials, ps...)
			} else if !projDamageEffects[n] {
				f.Warnings = append(f.Warnings, fmt.Sprintf("projected effect '%s' not modelled yet", n))
			}
		}
	}
}

var projDamageEffects = map[string]bool{"projectileFired": true, "targetAttack": true, "useMissiles": true, "barrage": true,
	"targetDisintegratorAttack": true, "missileLaunchingForEntity": true, "fighterAbilityAttackM": true, "fighterAbilityMissiles": true,
	"superWeaponAmarr": true, "superWeaponCaldari": true, "superWeaponGallente": true, "superWeaponMinmatar": true, "mining": true,
	"miningLaser": true, "miningClouds": true, "dotMissileLaunching": true, "ChainLightning": true, "salvageDroneEffect": true}

// localSpecial: local module effects that have no modifierInfo in the SDE but a hand-written Pyfa handler
// (eos/effects.py, LGPL; re-expressed). Source category 6 marks a boost applied without stacking penalty.
func (f *Fit) localSpecial(i int, name string, srcCat uint32) bool {
	ds := f.DS
	ship := uint32(f.Ship)
	a := ds.AttrID
	attr := func(target uint32, op int8, src uint32, cat uint32) {
		f.push(bItem, ship, target, amod{op: op, kind: srcAttr, item: int32(i), attr: src, from: int32(i)}, cat)
	}
	switch name {
	case "superWeaponAmarr", "superWeaponCaldari", "superWeaponGallente", "superWeaponMinmatar", "doomsdaySlash",
		"doomsdayBeamDOT", "doomsdayConeDOT", "doomsdayHOG", "debuffLance":
		attr(a("maxVelocity"), 6, a("speedFactor"), srcCat)
		attr(a("warpScrambleStatus"), 2, a("siegeModeWarpStatus"), srcCat)
	case "emergencyHullEnergizer":
		for _, t := range [4]string{"Em", "Thermal", "Kinetic", "Explosive"} {
			attr(a(strings.ToLower(t)+"DamageResonance"), 4, a("hull"+t+"DamageResonance"), srcCat)
		}
	case "entosisLink":
		attr(a("disallowAssistance"), 7, a("disallowAssistance"), 6)
		for _, t := range [4]string{"Gravimetric", "Magnetometric", "Radar", "Ladar"} {
			attr(a("scan"+t+"Strength"), 6, a("scan"+t+"StrengthPercent"), srcCat)
		}
	case "moduleBonusBreacherPodDamageControl":
		attr(a("breacherPodDamageResistance"), 6, a("breacherPodActivatedDamageReceivedPercentage"), 6)
	case "microJumpPortalDrive", "microJumpPortalDriveCapital":
		attr(a("signatureRadius"), 6, a("signatureRadiusBonusPercent"), srcCat)
	case "warpDisruptSphere":
		f.push(bItem, ship, a("disallowAssistance"), amod{op: 7, kind: srcConst, c: 1, from: int32(i)}, 6)
		if f.Items[i].Charge < 0 {
			attr(4, 6, a("massBonusPercentage"), 6)
			attr(a("signatureRadius"), 6, a("signatureRadiusBonus"), 6)
			for t := range f.Items {
				ti := &f.Items[t]
				if ti.Kind != KModule || ti.Loc != LShip {
					continue
				}
				if g := ds.group(ti.T.Group); g == nil || g.Name != "Propulsion Module" {
					continue
				}
				f.push(bItem, uint32(t), a("speedBoostFactor"), amod{op: 6, kind: srcAttr, item: int32(i), attr: a("speedBoostFactorBonus"), from: int32(i)}, 6)
				f.push(bItem, uint32(t), a("speedFactor"), amod{op: 6, kind: srcAttr, item: int32(i), attr: a("speedFactorBonus"), from: int32(i)}, 6)
			}
		}
	default:
		return false
	}
	return true
}

// incursionEffect: Sansha / Drifter incursion system effects (Pyfa Effect4728 OffensiveDefensiveReduction, LGPL;
// re-expressed): unpenalised PostPercent of missile-charge and smartbomb damage, turret and drone
// damageMultiplier by systemEffectDamageReduction, and of the ship's armor/shield resonances by the beacon's
// resistance bonuses.
func (f *Fit) incursionEffect(b int) {
	ds := f.DS
	a := ds.AttrID
	red := a("systemEffectDamageReduction")
	mls, _ := ds.TypeByName("Missile Launcher Operation")
	gunnery, _ := ds.TypeByName("Gunnery")
	smartbomb := uint32(0)
	for gid, g := range ds.Groups {
		if g != nil && g.Name == "Smart Bomb" {
			smartbomb = gid
			break
		}
	}
	mod := func(t int, attr, src uint32) {
		f.push(bItem, uint32(t), attr, amod{op: 6, kind: srcAttr, item: int32(b), attr: src, from: int32(b)}, 6)
	}
	for t := range f.Items {
		it := &f.Items[t]
		if !it.Owned || (it.Loc != LShip && it.Kind != KDrone) {
			continue
		}
		dmgB, mult := false, false
		switch it.Kind {
		case KCharge:
			dmgB = containsU32(it.ReqSkills, mls)
		case KModule:
			dmgB = it.Group == smartbomb
			mult = containsU32(it.ReqSkills, gunnery)
		case KDrone:
			mult = true
		}
		if dmgB {
			for _, d := range [4]string{"em", "thermal", "kinetic", "explosive"} {
				mod(t, a(d+"Damage"), red)
			}
		}
		if mult {
			mod(t, a("damageMultiplier"), red)
		}
	}
	for _, d := range [4]string{"Em", "Thermal", "Kinetic", "Explosive"} {
		for _, l := range [2]string{"armor", "shield"} {
			mod(f.Ship, a(l+d+"DamageResonance"), a(l+d+"DamageResistanceBonus"))
		}
	}
}

// pyRound2 is Python round(v, 2): correctly rounded on the exact binary value (ties to even).
func pyRound2(v float64) float64 {
	if math.IsInf(v, 0) || math.IsNaN(v) {
		return v
	}
	r, err := strconv.ParseFloat(strconv.FormatFloat(v, 'f', 2, 64), 64)
	if err != nil {
		return v
	}
	return r
}

// floatUnerr7 is Pyfa eos.utils.float.floatUnerr: round away float noise, keeping 7 significant digits.
func floatUnerr7(v float64) float64 {
	if v == 0 || math.IsInf(v, 0) || math.IsNaN(v) {
		return v
	}
	rf := 7 - int(math.Ceil(math.Log10(math.Abs(v))))
	if rf >= 0 {
		r, err := strconv.ParseFloat(strconv.FormatFloat(v, 'f', rf, 64), 64)
		if err != nil {
			return v
		}
		return r
	}
	p := math.Pow(10, float64(-rf))
	return math.Round(v/p) * p
}

// projSpecialFor mirrors Pyfa's 'projected' handlers for remote reps, cap transfers and neuts/nos (eos, LGPL).
func (f *Fit) projSpecialFor(i int, name string, resist uint32) ([]ProjSpecial, bool) {
	ds := f.DS
	a := ds.AttrID
	it := &f.Items[i]
	base := func(n string) float64 { v, _ := f.baseOK(i, a(n)); return v }
	dist := it.Distance
	falloff := func() float64 { return RangeFactor(base("maxRange"), base("falloffEffectiveness"), dist, true) }
	gate := func(opt float64) float64 {
		d := 0.0
		if dist != nil {
			d = *dist
		}
		if opt < d {
			return 0
		}
		return 1
	}
	noAssist := false
	if v, ok := f.baseOK(f.Ship, a("disallowAssistance")); ok && v != 0 {
		noAssist = true
	}
	rep := func(layer int, amt string, mult, factor float64) []ProjSpecial {
		if noAssist {
			return nil
		}
		return []ProjSpecial{{Rep: true, Item: i, Layer: layer, Amount: a(amt), Mult: mult, Factor: factor}}
	}
	drain := func(amt, dur string, factor, sign float64) []ProjSpecial {
		return []ProjSpecial{{Item: i, Amount: a(amt), Duration: a(dur), Factor: factor, Resist: resist, Sign: sign}}
	}
	noOffense := false
	if v, ok := f.baseOK(f.Ship, a("disallowOffensiveModifiers")); ok && v != 0 {
		noOffense = true
	}
	ecm := func(fighter bool, factor float64) []ProjSpecial {
		if noOffense {
			return nil
		}
		return []ProjSpecial{{Ecm: true, Fighter: fighter, Item: i, Factor: factor, Resist: resist}}
	}
	qty := float64(max(it.Quantity, 1))
	paste := it.Charge >= 0 && f.Items[it.Charge].T.Name == "Nanite Repair Paste"
	switch name {
	case "fighterAbilityEnergyNeutralizer":
		fac := RangeFactor(base("fighterAbilityEnergyNeutralizerOptimalRange"), base("fighterAbilityEnergyNeutralizerFalloffRange"), dist, true)
		return drain("fighterAbilityEnergyNeutralizerAmount", "fighterAbilityEnergyNeutralizerDuration", fac*qty, 1), true
	case "remoteECMFalloff", "structureModuleEffectECM":
		return ecm(false, falloff()), true
	case "entityECMFalloff":
		return ecm(false, gate(base("ECMRangeOptimal"))), true
	case "ECMBurstJammer":
		return ecm(false, gate(base("ecmBurstRange"))), true
	case "fighterAbilityECM":
		fac := RangeFactor(base("fighterAbilityECMRangeOptimal"), base("fighterAbilityECMRangeFalloff"), dist, true)
		return ecm(true, fac*qty), true
	case "shipModuleRemoteShieldBooster", "shipModuleAncillaryRemoteShieldBooster":
		return rep(0, "shieldBonus", 1, falloff()), true
	case "shipModuleRemoteArmorRepairer", "ShipModuleRemoteArmorMutadaptiveRepairer":
		return rep(1, "armorDamageAmount", 1, falloff()), true
	case "shipModuleAncillaryRemoteArmorRepairer":
		m := 1.0
		if paste {
			m = 3
		}
		return rep(1, "armorDamageAmount", m, falloff()), true
	case "shipModuleRemoteHullRepairer":
		return rep(2, "structureDamageAmount", 1, falloff()), true
	case "npcEntityRemoteShieldBooster":
		return rep(0, "shieldBonus", 1, gate(base("maxRange"))), true
	case "npcEntityRemoteArmorRepairer":
		return rep(1, "armorDamageAmount", 1, gate(base("maxRange"))), true
	case "npcEntityRemoteHullRepairer":
		return rep(2, "structureDamageAmount", 1, gate(base("maxRange"))), true
	case "shipModuleRemoteCapacitorTransmitter":
		if noAssist {
			return nil, true
		}
		return drain("powerTransferAmount", "duration", gate(base("maxRange")), -1), true
	case "energyNeutralizerFalloff":
		return drain("energyNeutralizerAmount", "duration", falloff(), 1), true
	case "energyNosferatuFalloff":
		return drain("powerTransferAmount", "duration", falloff(), 1), true
	case "structureEnergyNeutralizerFalloff":
		return drain("energyNeutralizerAmount", "duration", 1, 1), true
	case "entityEnergyNeutralizerFalloff":
		return drain("energyNeutralizerAmount", "energyNeutralizerDuration", gate(base("energyNeutralizerRangeOptimal")), 1), true
	}
	return nil, false
}

func (f *Fit) registerBuffs(req *FitRequest) {
	ds := f.DS
	agg := map[uint32]float64{}
	for _, b := range req.Fleet.Buffs {
		info := ds.Dbuffs[b.BuffID]
		if info == nil {
			f.Warnings = append(f.Warnings, fmt.Sprintf("unknown warfare buff %d", b.BuffID))
			continue
		}
		cur, ok := agg[b.BuffID]
		if !ok {
			cur = b.Value
		}
		if info.Aggregate != nil && *info.Aggregate == "Minimum" {
			cur = min(cur, b.Value)
		} else {
			cur = max(cur, b.Value)
		}
		agg[b.BuffID] = cur
	}
	// Pyfa keeps, per buff id, the single strongest (by |value|) source among the fit's own bursts and the
	// fleet booster fits; explicit fleet.buffs override both.
	type cand struct {
		v   float64
		src amod
	}
	best := map[uint32]cand{}
	var order []uint32
	offer := func(id uint32, v float64, src amod) {
		if old, ok := best[id]; ok && math.Abs(old.v) >= math.Abs(v) {
			return
		} else if !ok {
			order = append(order, id)
		}
		best[id] = cand{v, src}
	}
	w := &ds.ids
	for i := range f.Items {
		if f.Items[i].Kind != KModule || f.Items[i].State < Active {
			continue
		}
		for k := 0; k < 4; k++ {
			id := uint32(0)
			if f.Has(i, w.warfareID[k]) {
				id = uint32(f.Get(i, w.warfareID[k]))
			}
			if _, explicit := agg[id]; id == 0 || explicit {
				continue
			}
			offer(id, f.Get(i, w.warfareVal[k]), amod{kind: srcAttr, item: int32(i), attr: w.warfareVal[k], from: int32(i)})
		}
	}
	// abyssal weather / AoE cloud beacons (Pyfa weather_* / aoe_beacon_* effects): warfareBuff1/2 of the
	// environment item join the same command-bonus pool (strongest |value| per buff id)
	for i := range f.Items {
		if f.Items[i].Kind != KBeacon {
			continue
		}
		weather := false
		for _, e := range f.Items[i].Effects {
			if ei := ds.effect(e.ID); ei != nil && (strings.HasPrefix(ei.Name, "weather_") || strings.HasPrefix(ei.Name, "aoe_beacon_")) {
				weather = true
				break
			}
		}
		if !weather {
			continue
		}
		for k := 0; k < 2; k++ {
			id := uint32(0)
			if f.Has(i, w.warfareID[k]) {
				id = uint32(f.Get(i, w.warfareID[k]))
			}
			if _, explicit := agg[id]; id == 0 || explicit {
				continue
			}
			v := f.Get(i, w.warfareVal[k])
			offer(id, v, amod{kind: srcConst, c: v, from: int32(f.Ship)})
		}
	}
	for k := range req.Fleet.BoosterFits {
		breq := req.Fleet.BoosterFits[k]
		breq.Fleet.BoosterFits = nil
		b, err := Build(ds, &breq)
		if err != nil {
			f.Warnings = append(f.Warnings, fmt.Sprintf("fleet.booster_fits[%d]: %v", k, err))
			continue
		}
		for i := range b.Items {
			if b.Items[i].Kind != KModule || b.Items[i].State < Active {
				continue
			}
			for q := 0; q < 4; q++ {
				id := uint32(0)
				if b.Has(i, w.warfareID[q]) {
					id = uint32(b.Get(i, w.warfareID[q]))
				}
				if _, explicit := agg[id]; id == 0 || explicit {
					continue
				}
				v := b.Get(i, w.warfareVal[q])
				offer(id, v, amod{kind: srcConst, c: v, from: int32(f.Ship)})
			}
		}
		b.Release()
	}
	for id, v := range agg {
		if _, ok := best[id]; !ok {
			order = append(order, id)
		}
		best[id] = cand{v, amod{kind: srcConst, c: v, from: int32(f.Ship)}}
	}
	sort.Slice(order, func(a, b int) bool { return order[a] < order[b] })
	for _, id := range order {
		f.applyBuff(id, best[id].src)
	}
	f.Invalidate()
}

var droneBuffAttrs = map[uint32][]string{
	79: {"signatureRadius"},
	90: {"shieldEmDamageResonance", "armorEmDamageResonance", "emDamageResonance"},
	93: {"shieldExplosiveDamageResonance", "armorExplosiveDamageResonance", "explosiveDamageResonance"},
	95: {"shieldThermalDamageResonance", "armorThermalDamageResonance", "thermalDamageResonance"},
	99: {"shieldKineticDamageResonance", "armorKineticDamageResonance", "kineticDamageResonance"},
	94: {"shieldCapacity"},
	96: {"armorHP"},
	97: {"maxRange", "falloff"},
	98: {"maxVelocity"},
}

func (f *Fit) applyBuff(id uint32, src amod) {
	info := f.DS.Dbuffs[id]
	if info == nil {
		return
	}
	src.op = int8(info.Op)
	// Pyfa applies most buffs stacking-penalised; the abyssal weather resistance/HP/velocity buffs are not
	cat := uint32(0)
	switch id {
	case 90, 93, 94, 95, 96, 98, 99:
		cat = 6
	}
	for _, a := range info.Item {
		f.push(bItem, uint32(f.Ship), a, src, cat)
	}
	// AoE cloud / weather buffs also hit drones that require the Drones skill (Pyfa fit.py commandBonus)
	if names := droneBuffAttrs[id]; len(names) > 0 {
		for d := range f.Items {
			if f.Items[d].Kind != KDrone || !containsU32(f.Items[d].ReqSkills, 3436) {
				continue
			}
			for _, n := range names {
				if a := f.DS.AttrID(n); a != 0 {
					f.push(bItem, uint32(d), a, src, cat)
				}
			}
		}
	}
	for _, a := range info.Location {
		f.push(bShipLoc, 0, a, src, cat)
	}
	for _, p := range info.LocationGroup {
		f.push(bShipGroup, p[1], p[0], src, cat)
	}
	for _, p := range info.LocationSkill {
		f.push(bShipSkill, p[1], p[0], src, cat)
	}
	f.Invalidate()
}

// Invalidate drops every cached value (O(1) amortised: the map is cleared in place).
func (f *Fit) Invalidate() {
	f.cache.clearAll()
	if f.rdeps != nil {
		clear(f.rdeps)
	}
}

// applyRAH simulates Reactive Armor Hardener adaptation (Pyfa/eos algorithm, LGPL).
func (f *Fit) applyRAH(req *FitRequest) {
	ds := f.DS
	eid := ds.EffectID("adaptiveArmorHardener")
	if eid == 0 {
		return
	}
	names := [4]string{"armorEmDamageResonance", "armorThermalDamageResonance", "armorKineticDamageResonance", "armorExplosiveDamageResonance"}
	var attrs [4]uint32
	for k, n := range names {
		attrs[k] = ds.AttrID(n)
	}
	shiftAttr := ds.AttrID("resistanceShiftAmount")
	var rahs []int
	for i := range f.Items {
		it := &f.Items[i]
		if it.Kind == KModule && it.State >= Active {
			for _, e := range it.Effects {
				if e.ID == eid {
					rahs = append(rahs, i)
					break
				}
			}
		}
	}
	if len(rahs) == 0 {
		return
	}
	disable := req.Options.Rah != nil && *req.Options.Rah == "disable"
	dp := Resists{25, 25, 25, 25}
	if req.DamagePattern != nil {
		dp = *req.DamagePattern
	}
	pattern := [4]float64{dp.EM, dp.Thermal, dp.Kinetic, dp.Explosive}
	ship := f.Ship
	for _, m := range rahs {
		f.Invalidate()
		var res [4]float64
		for k := range res {
			res[k] = f.Get(m, attrs[k])
		}
		if !disable {
			var base [4]float64
			for k := range base {
				base[k] = pattern[k] * f.Get(ship, attrs[k])
			}
			shift := f.Get(m, shiftAttr) / 100
			var cycles [][4]float64
			loopStart := -20
			type tk struct {
				k   int
				dmg float64
				res float64
			}
			for iter := 0; iter < 50; iter++ {
				t := []tk{}
				for _, k := range [4]int{0, 3, 2, 1} {
					t = append(t, tk{k, base[k] * res[k], res[k]})
				}
				sort.SliceStable(t, func(a, b int) bool { return t[a].dmg < t[b].dmg })
				var c0, c1, c2, c3 float64
				if t[2].dmg == 0 {
					c0, c1, c2 = 1-t[0].res, 1-t[1].res, 1-t[2].res
					c3 = -(c0 + c1 + c2)
				} else if t[1].dmg == 0 {
					c0, c1 = 1-t[0].res, 1-t[1].res
					c2 = -(c0 + c1) / 2
					c3 = c2
				} else {
					c0, c1 = min(shift, 1-t[0].res), min(shift, 1-t[1].res)
					c2 = -(c0 + c1) / 2
					c3 = c2
				}
				res[t[0].k] = t[0].res + c0
				res[t[1].k] = t[1].res + c1
				res[t[2].k] = t[2].res + c2
				res[t[3].k] = t[3].res + c3
				found := -1
				for ci, v := range cycles {
					if math.Abs(res[0]-v[0]) <= 1e-6 && math.Abs(res[1]-v[1]) <= 1e-6 && math.Abs(res[2]-v[2]) <= 1e-6 && math.Abs(res[3]-v[3]) <= 1e-6 {
						found = ci
						break
					}
				}
				if found >= 0 {
					loopStart = found
					break
				}
				cycles = append(cycles, res)
			}
			start := 0
			if loopStart >= 0 {
				start = loopStart
			} else if len(cycles) > 20 {
				start = len(cycles) - 20
			}
			lp := cycles[start:]
			if len(lp) > 0 {
				for k := 0; k < 4; k++ {
					s := 0.0
					for _, v := range lp {
						s += v[k]
					}
					res[k] = math.Round(s/float64(len(lp))*1000) / 1000
				}
			}
		}
		cat := f.Items[m].Category
		for k := 0; k < 4; k++ {
			if !disable {
				f.push(bItem, uint32(m), attrs[k], amod{op: 7, kind: srcConst, c: res[k], from: int32(m)}, cat)
			}
			f.push(bItem, uint32(ship), attrs[k], amod{op: 0, kind: srcConst, c: res[k], from: int32(m)}, cat)
		}
	}
	f.Invalidate()
}

// ---------------------------------------------------------------- evaluation

// baseOK returns the unmodified value of an attribute on an item and whether the item carries it.
func (f *Fit) baseOK(i int, attr uint32) (float64, bool) {
	it := &f.Items[i]
	if len(it.overlay.ids) > 0 {
		if v, ok := it.overlay.get(attr); ok {
			return v, true
		}
	}
	return it.T.base.get(attr)
}

func (f *Fit) baseOf(i int, attr uint32) float64 {
	v, _ := f.baseOK(i, attr)
	return v
}

// Base returns the unmodified value (dataset default if absent).
func (f *Fit) Base(i int, attr uint32) float64 {
	if v, ok := f.baseOK(i, attr); ok {
		return v
	}
	return f.DS.AttrDefault(attr)
}

// applies reports whether a modifier with selector (k, x) reaches item i.
func (f *Fit) applies(i int, it *Item, k bucketKind, x uint32) bool {
	switch k {
	case bItem:
		return x == uint32(i)
	case bShipLoc:
		return it.Loc == LShip
	case bShipGroup:
		return it.Loc == LShip && it.Group == x
	case bShipSkill:
		return it.Loc == LShip && it.hasSkill(x)
	case bOwnerSkill:
		return it.Owned && it.hasSkill(x)
	case bCharLoc:
		return it.Loc == LChar
	case bCharGroup:
		return it.Loc == LChar && it.Group == x
	case bCharSkill:
		return (it.Owned || it.Loc == LChar) && it.Kind != KSkill && it.hasSkill(x)
	}
	return false
}

// collect appends, in registration order, the modifiers that reach (item, attr).
func (f *Fit) collect(i int, attr uint32, out []*amod) []*amod {
	it := &f.Items[i]
	var pre, post []amod
	if int(attr) < len(f.reg) {
		am := &f.reg[attr]
		pre, post = am.mods[:am.split], am.mods[am.split:]
	}
	for k := range pre {
		if m := &pre[k]; f.applies(i, it, m.sel, m.x) {
			out = append(out, m)
		}
	}
	if f.skillTpl != nil && int(attr) < len(f.skillTpl) {
		l := f.skillTpl[attr]
		for k := range l {
			if m := &l[k]; f.applies(i, it, m.sel, m.x) {
				out = append(out, m)
			}
		}
	}
	for k := range post {
		if m := &post[k]; f.applies(i, it, m.sel, m.x) {
			out = append(out, m)
		}
	}
	return out
}

// Has reports whether the item carries the attribute (from its type or because a modifier targets it).
func (f *Fit) Has(i int, attr uint32) bool {
	if _, ok := f.baseOK(i, attr); ok {
		return true
	}
	var b [foldBuf]*amod
	return len(f.collect(i, attr, b[:0])) > 0
}

// Get returns the modified value of an attribute.
func (f *Fit) Get(i int, attr uint32) float64 {
	key := nodeKey(i, attr)
	if f.TrackDeps && len(f.stack) > 0 {
		f.rdeps[key] = append(f.rdeps[key], f.stack[len(f.stack)-1])
	}
	if v, ok := f.cache.get(key); ok {
		return v
	}
	base, hasBase := f.baseOK(i, attr)
	var mbuf [foldBuf]*amod
	ms := f.collect(i, attr, mbuf[:0])
	if !hasBase && len(ms) == 0 {
		return f.DS.AttrDefault(attr)
	}
	if !hasBase {
		base = f.DS.AttrDefault(attr)
	}
	for _, k := range f.stack {
		if k == key {
			return base // cycle guard
		}
	}
	f.stack = append(f.stack, key)
	val := f.fold(attr, base, ms)
	info := f.DS.attr(attr)
	if info != nil {
		if info.MinAttr != 0 {
			val = max(val, f.Get(i, info.MinAttr))
		}
		if info.MaxAttr != 0 {
			val = min(val, f.Get(i, info.MaxAttr))
		}
		if info.round2 {
			val = pyRound2(val)
		}
	}
	f.stack = f.stack[:len(f.stack)-1]
	f.cache.put(key, val)
	return val
}

// GetOpt returns the value and whether the item carries the attribute.
func (f *Fit) GetOpt(i int, attr uint32) (float64, bool) {
	if !f.Has(i, attr) {
		return 0, false
	}
	return f.Get(i, attr), true
}

func (f *Fit) srcValue(m *amod) float64 {
	switch m.kind {
	case srcAttr:
		return f.Get(int(m.item), m.attr)
	case srcConst:
		return m.c
	case srcProp:
		mass := f.Get(int(m.ship), m.a3)
		if mass == 0 {
			return 1
		}
		return 1 + f.Get(int(m.item), m.attr)/100*f.Get(int(m.item), m.a2)/mass
	case srcProj:
		fac := m.c
		if m.a2 != 0 {
			fac *= f.Get(int(m.ship), m.a2)
		}
		v := f.Get(int(m.item), m.attr)
		if m.mul {
			return (v-1)*fac + 1
		}
		return v * fac
	}
	return 0
}

// penaltyFactors[i] = exp(-(i^2)/7.1289), the values glibc's exp produces (bit-identical with Rust/Python).
var penaltyFactors = [16]float64{1.0, 0.8691199808003975, 0.5705831435105602, 0.28295515402326116, 0.10599264974270436,
	0.02999116653328048, 0.006410183117533512, 0.001034920482668705, 0.00012621268254589462, 1.1626753929630923e-05,
	8.090464068743095e-07, 4.2525345863587603e-08, 1.688424883993958e-09, 5.063783187479741e-11,
	1.1471703921167754e-12, 1.9630900654863316e-14}

func penalty(i int) float64 {
	if i < len(penaltyFactors) {
		return penaltyFactors[i]
	}
	return math.Exp(-float64(i*i) / 7.1289)
}

const foldBuf = 64

// fold applies all modifiers in CCP operator order with stacking penalties. Within an operator, modifiers
// are applied in registration order so results are bit-identical with eve-dogma-rs.
func (f *Fit) fold(attr uint32, val float64, ms []*amod) float64 {
	var vbuf [foldBuf]float64
	vals := vbuf[:0]
	var present uint16
	for _, m := range ms {
		vals = append(vals, f.srcValue(m))
		if m.op >= -1 && m.op <= 7 {
			present |= 1 << uint(m.op+1)
		}
	}
	hig := true
	if a := f.DS.attr(attr); a != nil {
		hig = a.HighIsGood
	}
	var posB, negB [foldBuf]float64
	for op := int8(-1); op <= 7; op++ {
		if present&(1<<uint(op+1)) == 0 {
			continue
		}
		pos, neg := posB[:0], negB[:0]
		hasAssign := false
		assign := 0.0
		for k, m := range ms {
			if m.op != op {
				continue
			}
			v := vals[k]
			switch op {
			case -1, 7:
				if !hasAssign {
					assign, hasAssign = v, true
				} else if hig {
					assign = max(assign, v)
				} else {
					assign = min(assign, v)
				}
			case 2:
				val += v
			case 3:
				val -= v
			default:
				var x float64
				switch op {
				case 0, 4:
					x = v
				case 1, 5:
					if v == 0 {
						x = 1
					} else {
						x = 1 / v
					}
				case 6:
					x = 1 + v/100
				default:
					x = 1
				}
				if m.pen {
					if x > 1 {
						pos = append(pos, x)
					} else if x < 1 {
						neg = append(neg, x)
					}
				} else {
					val *= x
				}
			}
		}
		if hasAssign {
			val = assign
		}
		for _, lst := range [2][]float64{pos, neg} {
			// strongest first (stable insertion sort)
			for i := 1; i < len(lst); i++ {
				for j := i; j > 0 && math.Abs(lst[j-1]-1) < math.Abs(lst[j]-1); j-- {
					lst[j-1], lst[j] = lst[j], lst[j-1]
				}
			}
			for k, m := range lst {
				val *= 1 + (m-1)*penalty(k)
			}
		}
	}
	return val
}

// SetBase changes an item's base attribute value and invalidates dependent cached values. With TrackDeps
// enabled only the dependents are invalidated; otherwise the whole cache is dropped.
func (f *Fit) SetBase(i int, attr uint32, v float64) {
	f.setOverlay(i, attr, v)
	if !f.TrackDeps {
		f.Invalidate()
		return
	}
	var walk func(k uint64)
	seen := map[uint64]bool{}
	walk = func(k uint64) {
		if seen[k] {
			return
		}
		seen[k] = true
		f.cache.del(k)
		for _, d := range f.rdeps[k] {
			walk(d)
		}
		delete(f.rdeps, k)
	}
	walk(nodeKey(i, attr))
}

// EnableDepTracking turns on reverse-dependency recording (call before evaluating).
func (f *Fit) EnableDepTracking() {
	f.TrackDeps = true
	if f.rdeps == nil {
		f.rdeps = map[uint64][]uint64{}
	}
	f.Invalidate()
}

// AttrIDs lists attributes an item carries (base + modified), sorted.
func (f *Fit) AttrIDs(i int) []uint32 {
	set := map[uint32]bool{}
	it := &f.Items[i]
	for _, a := range it.T.base.ids {
		set[a] = true
	}
	for _, a := range it.overlay.ids {
		set[a] = true
	}
	for _, a := range f.regUsed {
		if !set[a] && f.Has(i, a) {
			set[a] = true
		}
	}
	for a, l := range f.skillTpl {
		if len(l) > 0 && !set[uint32(a)] && f.Has(i, uint32(a)) {
			set[uint32(a)] = true
		}
	}
	out := make([]uint32, 0, len(set))
	for a := range set {
		out = append(out, a)
	}
	sort.Slice(out, func(a, b int) bool { return out[a] < out[b] })
	return out
}

// ipath is a JSON pointer into the request, formatted only when an error needs it.
type ipath struct {
	prefix string
	i      int // -1 = none
	suffix string
}

func (p ipath) with(s string) ipath { p.suffix += s; return p }

func (p ipath) String() string {
	if p.i < 0 {
		return p.prefix + p.suffix
	}
	return p.prefix + strconv.Itoa(p.i) + p.suffix
}
