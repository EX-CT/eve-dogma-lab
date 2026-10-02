package dogma

import (
	"encoding/json"
	"fmt"
)

// State of a module (ordered).
type State int8

const (
	Offline State = iota
	Online
	Active
	Overheated
)

var stateNames = [...]string{"offline", "online", "active", "overheated"}

func (s State) String() string               { return stateNames[s] }
func (s State) MarshalJSON() ([]byte, error) { return json.Marshal(stateNames[s]) }
func (s *State) UnmarshalJSON(b []byte) error {
	if n := len(b); n >= 2 && b[0] == '"' && b[n-1] == '"' {
		for i, name := range stateNames {
			if string(b[1:n-1]) == name { // no allocation: the conversion is only compared
				*s = State(i)
				return nil
			}
		}
	}
	var v string
	if err := json.Unmarshal(b, &v); err != nil {
		return err
	}
	for i, n := range stateNames {
		if n == v {
			*s = State(i)
			return nil
		}
	}
	return fmt.Errorf("unknown variant `%s`, expected one of `offline`, `online`, `active`, `overheated`", v)
}

// Slot kind. SlotNone = not fittable / unknown.
type Slot int8

const (
	SlotNone Slot = iota
	SlotHigh
	SlotMid
	SlotLow
	SlotRig
	SlotSubsystem
	SlotService
)

var slotNames = [...]string{"", "high", "mid", "low", "rig", "subsystem", "service"}

func (s Slot) String() string { return slotNames[s] }
func (s Slot) MarshalJSON() ([]byte, error) {
	if s == SlotNone {
		return []byte("null"), nil
	}
	return json.Marshal(slotNames[s])
}
func (s *Slot) UnmarshalJSON(b []byte) error {
	if string(b) == "null" {
		*s = SlotNone
		return nil
	}
	if n := len(b); n >= 2 && b[0] == '"' && b[n-1] == '"' {
		for i, name := range slotNames {
			if i > 0 && string(b[1:n-1]) == name {
				*s = Slot(i)
				return nil
			}
		}
	}
	var v string
	if err := json.Unmarshal(b, &v); err != nil {
		return err
	}
	for i, n := range slotNames {
		if i > 0 && n == v {
			*s = Slot(i)
			return nil
		}
	}
	return fmt.Errorf("unknown slot `%s`", v)
}

type Spool struct {
	Type   string  `json:"type"` // spool_scale | cycle_scale | time | cycles
	Amount float64 `json:"amount"`
}

type Mutation struct {
	BaseTypeID        uint32             `json:"base_type_id"`
	MutaplasmidTypeID *uint32            `json:"mutaplasmid_type_id"`
	Attributes        map[string]float64 `json:"attributes,omitempty"`
}

type ModuleReq struct {
	TypeID       uint32    `json:"type_id"`
	Slot         *Slot     `json:"slot"`
	State        *State    `json:"state"`
	ChargeTypeID *uint32   `json:"charge_type_id"`
	Mutation     *Mutation `json:"mutation"`
	Spool        *Spool    `json:"spool"`
}

type DroneReq struct {
	TypeID   uint32    `json:"type_id"`
	Quantity uint32    `json:"quantity"`
	Active   *uint32   `json:"active"`
	Mutation *Mutation `json:"mutation"`
}

func (d *DroneReq) UnmarshalJSON(b []byte) error {
	type alias DroneReq
	a := alias{Quantity: 1}
	if err := json.Unmarshal(b, &a); err != nil {
		return err
	}
	*d = DroneReq(a)
	return nil
}

type FighterReq struct {
	TypeID    uint32    `json:"type_id"`
	Quantity  *uint32   `json:"quantity"`
	Active    bool      `json:"active"`
	Abilities *[]uint32 `json:"abilities"`
}

func (f *FighterReq) UnmarshalJSON(b []byte) error {
	type alias FighterReq
	a := alias{Active: true}
	if err := json.Unmarshal(b, &a); err != nil {
		return err
	}
	*f = FighterReq(a)
	return nil
}

type BoosterReq struct {
	TypeID      uint32   `json:"type_id"`
	SideEffects []uint32 `json:"side_effects,omitempty"`
}

type CargoReq struct {
	TypeID   uint32 `json:"type_id"`
	Quantity uint32 `json:"quantity"`
}

func (c *CargoReq) UnmarshalJSON(b []byte) error {
	type alias CargoReq
	a := alias{Quantity: 1}
	if err := json.Unmarshal(b, &a); err != nil {
		return err
	}
	*c = CargoReq(a)
	return nil
}

type Skills struct {
	DefaultLevel *uint8           `json:"default_level"`
	Levels       map[string]uint8 `json:"levels,omitempty"`
}

type Character struct {
	Skills         Skills   `json:"skills"`
	SecurityStatus *float64 `json:"security_status"`
}

type Buff struct {
	BuffID uint32  `json:"buff_id"`
	Value  float64 `json:"value"`
}

type Fleet struct {
	Buffs       []Buff       `json:"buffs,omitempty"`
	BoosterFits []FitRequest `json:"booster_fits,omitempty"`
}

type Projected struct {
	Kind      string      `json:"kind"`
	Module    *ModuleReq  `json:"module"`
	Drone     *DroneReq   `json:"drone"`
	Fit       *FitRequest `json:"fit"`
	Fighter   *FighterReq `json:"fighter"`
	Amount    uint32      `json:"amount"`
	DistanceM *float64    `json:"distance_m"`
}

func (p *Projected) UnmarshalJSON(b []byte) error {
	type alias Projected
	a := alias{Amount: 1}
	if err := json.Unmarshal(b, &a); err != nil {
		return err
	}
	*p = Projected(a)
	return nil
}

type Environment struct {
	EffectTypeIDs  []uint32 `json:"effect_type_ids,omitempty"`
	SystemSecurity *string  `json:"system_security"`
}

type Resists struct {
	EM        float64 `json:"em"`
	Thermal   float64 `json:"thermal"`
	Kinetic   float64 `json:"kinetic"`
	Explosive float64 `json:"explosive"`
}

type TargetProfile struct {
	EM              float64  `json:"em"`
	Thermal         float64  `json:"thermal"`
	Kinetic         float64  `json:"kinetic"`
	Explosive       float64  `json:"explosive"`
	SignatureRadius *float64 `json:"signature_radius"`
	MaxVelocity     *float64 `json:"max_velocity"`
	Radius          *float64 `json:"radius"`
}

type Override struct {
	TypeID      uint32  `json:"type_id"`
	AttributeID uint32  `json:"attribute_id"`
	Value       float64 `json:"value"`
}

type CapSimOpts struct {
	Reload   bool     `json:"reload"`
	Stagger  bool     `json:"stagger"`
	MaxTimeS *float64 `json:"max_time_s"`
}

type Options struct {
	NosNoTargetCap    bool       `json:"nos_no_target_cap"`
	FactorReload      bool       `json:"factor_reload"`
	DefaultSpool      *Spool     `json:"default_spool"`
	Rah               *string    `json:"rah"`
	IncludeAttributes *string    `json:"include_attributes"`
	Sources           bool       `json:"sources"`
	Validate          bool       `json:"validate"`
	CapSim            CapSimOpts `json:"cap_sim"`
}

func (o *Options) UnmarshalJSON(b []byte) error {
	type alias Options
	a := alias{Validate: true}
	if err := json.Unmarshal(b, &a); err != nil {
		return err
	}
	*o = Options(a)
	return nil
}

type ShipReq struct {
	TypeID     uint32  `json:"type_id"`
	ModeTypeID *uint32 `json:"mode_type_id"`
}

// FitRequest v1 (see eve-fit-docs/schema/fit-request.schema.json).
type FitRequest struct {
	SchemaVersion *uint32        `json:"schema_version"`
	Ship          ShipReq        `json:"ship"`
	Character     Character      `json:"character"`
	Modules       []ModuleReq    `json:"modules,omitempty"`
	Drones        []DroneReq     `json:"drones,omitempty"`
	Fighters      []FighterReq   `json:"fighters,omitempty"`
	Implants      []uint32       `json:"implants,omitempty"`
	Boosters      []BoosterReq   `json:"boosters,omitempty"`
	Cargo         []CargoReq     `json:"cargo,omitempty"`
	Fleet         Fleet          `json:"fleet"`
	Projected     []Projected    `json:"projected,omitempty"`
	Environment   Environment    `json:"environment"`
	DamagePattern *Resists       `json:"damage_pattern"`
	TargetProfile *TargetProfile `json:"target_profile"`
	Overrides     []Override     `json:"overrides,omitempty"`
	Options       Options        `json:"options"`
}

func (r *FitRequest) UnmarshalJSON(b []byte) error {
	// One decoding pass into wire structs whose defaulted fields are pointers (no nested custom
	// unmarshalers, which would re-validate and re-scan every sub-object), then apply serde defaults.
	var w wireFit
	if fastDecodeWire(b, &w) {
		if w.missingShip() {
			return fmt.Errorf("missing field `ship`")
		}
		w.to(r)
		return nil
	}
	w = wireFit{}
	err := json.Unmarshal(b, &w)
	if _, syntax := err.(*json.SyntaxError); syntax {
		return err
	}
	if w.missingShip() {
		return fmt.Errorf("missing field `ship`")
	}
	if err != nil {
		return err
	}
	w.to(r)
	return nil
}

type wireDrone struct {
	TypeID   uint32    `json:"type_id"`
	Quantity *uint32   `json:"quantity"`
	Active   *uint32   `json:"active"`
	Mutation *Mutation `json:"mutation"`
}

type wireFighter struct {
	TypeID    uint32    `json:"type_id"`
	Quantity  *uint32   `json:"quantity"`
	Active    *bool     `json:"active"`
	Abilities *[]uint32 `json:"abilities"`
}

type wireCargo struct {
	TypeID   uint32  `json:"type_id"`
	Quantity *uint32 `json:"quantity"`
}

type wireProjected struct {
	Kind      string       `json:"kind"`
	Module    *ModuleReq   `json:"module"`
	Drone     *wireDrone   `json:"drone"`
	Fit       *wireFit     `json:"fit"`
	Fighter   *wireFighter `json:"fighter"`
	Amount    *uint32      `json:"amount"`
	DistanceM *float64     `json:"distance_m"`
}

type wireOptions struct {
	NosNoTargetCap    bool       `json:"nos_no_target_cap"`
	FactorReload      bool       `json:"factor_reload"`
	DefaultSpool      *Spool     `json:"default_spool"`
	Rah               *string    `json:"rah"`
	IncludeAttributes *string    `json:"include_attributes"`
	Sources           bool       `json:"sources"`
	Validate          *bool      `json:"validate"`
	CapSim            CapSimOpts `json:"cap_sim"`
}

type wireFleet struct {
	Buffs       []Buff    `json:"buffs"`
	BoosterFits []wireFit `json:"booster_fits"`
}

type wireFit struct {
	SchemaVersion *uint32         `json:"schema_version"`
	Ship          *ShipReq        `json:"ship"`
	Character     Character       `json:"character"`
	Modules       []ModuleReq     `json:"modules"`
	Drones        []wireDrone     `json:"drones"`
	Fighters      []wireFighter   `json:"fighters"`
	Implants      []uint32        `json:"implants"`
	Boosters      []BoosterReq    `json:"boosters"`
	Cargo         []wireCargo     `json:"cargo"`
	Fleet         wireFleet       `json:"fleet"`
	Projected     []wireProjected `json:"projected"`
	Environment   Environment     `json:"environment"`
	DamagePattern *Resists        `json:"damage_pattern"`
	TargetProfile *TargetProfile  `json:"target_profile"`
	Overrides     []Override      `json:"overrides"`
	Options       *wireOptions    `json:"options"`
}

// missingShip: a FitRequest (top level or nested booster/projected fit) without "ship".
func (w *wireFit) missingShip() bool {
	if w.Ship == nil {
		return true
	}
	for i := range w.Fleet.BoosterFits {
		if w.Fleet.BoosterFits[i].missingShip() {
			return true
		}
	}
	for _, p := range w.Projected {
		if p.Fit != nil && p.Fit.missingShip() {
			return true
		}
	}
	return false
}

func orU32(p *uint32, d uint32) uint32 {
	if p == nil {
		return d
	}
	return *p
}

func (x *wireFighter) to() FighterReq {
	return FighterReq{TypeID: x.TypeID, Quantity: x.Quantity, Active: x.Active == nil || *x.Active, Abilities: x.Abilities}
}

func (d *wireDrone) to() DroneReq {
	return DroneReq{TypeID: d.TypeID, Quantity: orU32(d.Quantity, 1), Active: d.Active, Mutation: d.Mutation}
}

func (w *wireFit) to(r *FitRequest) {
	*r = FitRequest{SchemaVersion: w.SchemaVersion, Character: w.Character, Modules: w.Modules, Implants: w.Implants,
		Boosters: w.Boosters, Environment: w.Environment, DamagePattern: w.DamagePattern, TargetProfile: w.TargetProfile,
		Overrides: w.Overrides, Options: Options{Validate: true}}
	if w.Ship != nil {
		r.Ship = *w.Ship
	}
	if w.Drones != nil {
		r.Drones = make([]DroneReq, len(w.Drones))
		for i := range w.Drones {
			r.Drones[i] = w.Drones[i].to()
		}
	}
	if w.Fighters != nil {
		r.Fighters = make([]FighterReq, len(w.Fighters))
		for i, x := range w.Fighters {
			r.Fighters[i] = x.to()
		}
	}
	if w.Cargo != nil {
		r.Cargo = make([]CargoReq, len(w.Cargo))
		for i, x := range w.Cargo {
			r.Cargo[i] = CargoReq{TypeID: x.TypeID, Quantity: orU32(x.Quantity, 1)}
		}
	}
	r.Fleet.Buffs = w.Fleet.Buffs
	if w.Fleet.BoosterFits != nil {
		r.Fleet.BoosterFits = make([]FitRequest, len(w.Fleet.BoosterFits))
		for i := range w.Fleet.BoosterFits {
			w.Fleet.BoosterFits[i].to(&r.Fleet.BoosterFits[i])
		}
	}
	if w.Projected != nil {
		r.Projected = make([]Projected, len(w.Projected))
		for i, x := range w.Projected {
			p := Projected{Kind: x.Kind, Module: x.Module, Amount: orU32(x.Amount, 1), DistanceM: x.DistanceM}
			if x.Drone != nil {
				d := x.Drone.to()
				p.Drone = &d
			}
			if x.Fighter != nil {
				fr := x.Fighter.to()
				p.Fighter = &fr
			}
			if x.Fit != nil {
				p.Fit = &FitRequest{}
				x.Fit.to(p.Fit)
			}
			r.Projected[i] = p
		}
	}
	if o := w.Options; o != nil {
		r.Options = Options{NosNoTargetCap: o.NosNoTargetCap, FactorReload: o.FactorReload, DefaultSpool: o.DefaultSpool,
			Rah: o.Rah, IncludeAttributes: o.IncludeAttributes, Sources: o.Sources, Validate: o.Validate == nil || *o.Validate,
			CapSim: o.CapSim}
	}
}
