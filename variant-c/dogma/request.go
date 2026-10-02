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
	Attributes        map[string]float64 `json:"attributes"`
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
	SideEffects []uint32 `json:"side_effects"`
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
	Levels       map[string]uint8 `json:"levels"`
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
	Buffs       []Buff       `json:"buffs"`
	BoosterFits []FitRequest `json:"booster_fits"`
}

type Projected struct {
	Kind      string      `json:"kind"`
	Module    *ModuleReq  `json:"module"`
	Drone     *DroneReq   `json:"drone"`
	Fit       *FitRequest `json:"fit"`
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
	EffectTypeIDs  []uint32 `json:"effect_type_ids"`
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
	Modules       []ModuleReq    `json:"modules"`
	Drones        []DroneReq     `json:"drones"`
	Fighters      []FighterReq   `json:"fighters"`
	Implants      []uint32       `json:"implants"`
	Boosters      []BoosterReq   `json:"boosters"`
	Cargo         []CargoReq     `json:"cargo"`
	Fleet         Fleet          `json:"fleet"`
	Projected     []Projected    `json:"projected"`
	Environment   Environment    `json:"environment"`
	DamagePattern *Resists       `json:"damage_pattern"`
	TargetProfile *TargetProfile `json:"target_profile"`
	Overrides     []Override     `json:"overrides"`
	Options       Options        `json:"options"`
}

func (r *FitRequest) UnmarshalJSON(b []byte) error {
	type alias FitRequest
	a := alias{Options: Options{Validate: true}}
	var probe struct {
		Ship *json.RawMessage `json:"ship"`
	}
	if err := json.Unmarshal(b, &probe); err != nil {
		return err
	}
	if probe.Ship == nil {
		return fmt.Errorf("missing field `ship`")
	}
	if err := json.Unmarshal(b, &a); err != nil {
		return err
	}
	*r = FitRequest(a)
	return nil
}
