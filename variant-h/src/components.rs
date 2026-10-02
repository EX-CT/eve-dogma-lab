//! ECS components. Every dogma item (ship, character, skill, module, charge, drone, fighter, implant, booster,
//! T3D mode, environment beacon, projected source) is one `hecs` entity.
use crate::request::{Slot, Spool, State};
use hecs::Entity;
use rustc_hash::FxHashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ship,
    Char,
    Skill,
    Module,
    Charge,
    Drone,
    Fighter,
    Implant,
    Booster,
    Mode,
    Beacon,
    Projected,
}

/// Where the item lives, for location-filtered modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loc {
    Ship,
    Char,
    Space,
    Nowhere,
}

/// Identity of an item (always present).
#[derive(Debug, Clone, Copy)]
pub struct Item {
    pub type_id: u32,
    pub group: u32,
    pub category: u32,
    pub kind: Kind,
    pub loc: Loc,
    /// owner-required-skill modifiers reach owned items (modules, charges, drones, fighters, ship)
    pub owned: bool,
}

/// Effective activation state used to decide which effects run (always present).
#[derive(Debug, Clone, Copy)]
pub struct Power(pub State);

/// Fitted module data.
#[derive(Debug, Clone, Copy)]
pub struct Fitted {
    pub slot: Option<Slot>,
    pub req_index: usize,
    pub charge: Option<Entity>,
    pub spool: Option<Spool>,
}

/// Charge loaded into a module.
#[derive(Debug, Clone, Copy)]
pub struct LoadedIn(pub Entity);

/// Drone stack / fighter squadron.
#[derive(Debug, Clone, Copy)]
pub struct Squad {
    pub quantity: u32,
    pub active: u32,
    pub req_index: usize,
}

#[derive(Debug, Clone)]
pub struct FighterAbilities(pub Vec<u32>);

#[derive(Debug, Clone)]
pub struct SideEffects(pub Vec<u32>);

/// Projected source distance (m), None = in optimal.
#[derive(Debug, Clone, Copy)]
pub struct Distance(pub Option<f64>);

/// Effect list / required skills when they differ from the type's (mutated items).
#[derive(Debug, Clone)]
pub struct Mutated {
    pub effects: Vec<(u32, bool)>,
    pub req_skills: Vec<u32>,
}

/// Where a modifier's value comes from.
#[derive(Debug, Clone, Copy)]
pub enum Src {
    Attr { e: Entity, attr: u32 },
    Const(f64),
    /// AB/MWD speed bonus: 1 + speedFactor/100 * speedBoostFactor / ship mass (PostMul)
    Prop { module: Entity, ship: Entity },
    /// projected effect value scaled by range factor and (lazily) by the target's resistance attribute
    Projected { e: Entity, attr: u32, factor: f64, target: Entity, resist: u32, mul: bool },
}

#[derive(Debug, Clone, Copy)]
pub struct Mod {
    pub op: i8,
    pub penalized: bool,
    pub src: Src,
}

/// One attribute that differs from the type's base value (own base and/or modifiers).
/// Pure data: memoised results live in the calculation system (`Calc`), not in the component.
#[derive(Debug)]
pub struct AttrSlot {
    pub base: f64,
    pub mods: Vec<Mod>,
}

impl AttrSlot {
    pub fn new(base: f64) -> Self {
        AttrSlot { base, mods: Vec::new() }
    }
}

/// Attribute component: base values come from the static type (no copy); only touched attributes get a slot.
#[derive(Debug)]
pub struct Attrs {
    pub type_id: u32,
    pub slots: FxHashMap<u32, AttrSlot>,
}

/// Incoming remote repair from a projected entity (feeds `defense.tank`, Pyfa applied-RR formula).
#[derive(Debug, Clone, Copy)]
pub struct IncomingRep {
    /// 0 shield, 1 armor, 2 hull
    pub layer: u8,
    pub amount_attr: u32,
    pub mult: f64,
    pub factor: f64,
}

/// Incoming capacitor drain (sign +1: neut/nos) or fill (sign -1: cap transfer) from a projected entity.
#[derive(Debug, Clone, Copy)]
pub struct IncomingDrain {
    pub amount_attr: u32,
    pub duration_attr: u32,
    pub factor: f64,
    pub resist: u32,
    pub sign: f64,
}
