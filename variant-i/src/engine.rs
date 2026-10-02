//! Incremental dogma engine on salsa.
//!
//! Inputs: one `FitIn` (canonical item order + fit context) and one `ItemIn` per item slot.
//! Derived queries (memoised, dependency tracked, backdated when unchanged):
//!   index(fit)                 -> per-item structural info used for modifier target filters
//!   outgoing(fit, item)        -> modifiers an item emits (its effects x targets)
//!   incoming(fit)              -> per-target modifier maps (layer 0: item effects + explicit fleet buffs)
//!   item_mods(fit, item)       -> one target's modifier map (backdates, so unrelated attrs stay valid)
//!   layer_mods(fit, L)         -> extra modifier layers that need evaluated values (local bursts, each RAH)
//!   attr_value(fit, item/attr/layer) -> evaluated attribute
//! Modifier semantics follow EX-CT/eve-dogma-rs (LGPL-3.0-or-later) so results match Pyfa the same way.
use crate::data::{Dataset, Domain, Func};
use crate::request::State;
use crate::spec::{ItemSpec, Kind, Loc};
use rustc_hash::FxHashMap;
use std::sync::Arc;

const EXEMPT_CATEGORIES: [u32; 6] = [6, 8, 16, 20, 32, 65];
const EFFECT_SKILL_EFFECT: u32 = 132;
const HULL_RESONANCES: [u32; 4] = [113, 111, 109, 110];
const STRUCTURE_SKILL_EFFECT_NAMES: [&str; 5] = [
    "targetingMaxTargetBonusModAddMaxLockedTargetsLocationChar",
    "skillStructureMissileDamageBonus",
    "skillStructureElectronicSystemsCapNeedBonus",
    "skillStructureEngineeringSystemsCapNeedBonus",
    "skillStructureDoomsdayDurationBonus",
];
const ARMOR_RES: [&str; 4] = ["armorEmDamageResonance", "armorThermalDamageResonance", "armorKineticDamageResonance", "armorExplosiveDamageResonance"];

/// f64 with bitwise equality, so derived values can be compared for backdating.
#[derive(Debug, Clone, Copy)]
pub struct F(pub f64);
impl PartialEq for F {
    fn eq(&self, o: &F) -> bool {
        self.0.to_bits() == o.0.to_bits()
    }
}
impl Eq for F {}
impl std::hash::Hash for F {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.0.to_bits().hash(h)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Src {
    Attr { item: u32, attr: u32 },
    Const(F),
    Prop { module: u32, ship: u32, speed: u32, thrust: u32, mass: u32 },
    Projected { item: u32, attr: u32, factor: F, target: u32, resist: u32, mul: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AMod {
    pub op: i8,
    pub penalized: bool,
    pub src: Src,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Out {
    pub target: u32,
    pub attr: u32,
    pub m: AMod,
}

/// Fit-wide context that is not per item.
#[derive(Debug, Clone, PartialEq)]
pub struct Ctx {
    pub ship: u32,
    pub char: u32,
    pub is_structure: bool,
    /// aggregated explicit fleet buffs (id, value), sorted by id
    pub buffs: Vec<(u32, F)>,
    /// (buff id, value) offered by fleet booster fits, in booster-fit order
    pub booster_offers: Vec<(u32, F)>,
    pub rah_disable: bool,
    pub pattern: [F; 4],
}

#[salsa::db]
pub trait Db: salsa::Database {
    fn ds(&self) -> &Dataset;
    fn consts(&self) -> &Consts;
}

/// Attribute/effect ids resolved once per dataset.
pub struct Consts {
    pub e_ab: u32,
    pub e_mwd: u32,
    pub e_slot: u32,
    pub e_hp: u32,
    pub e_mjd: u32,
    pub e_bastion: u32,
    pub e_rah: u32,
    pub structure_ok: Vec<u32>,
    pub mass_addition: u32,
    pub speed_factor: u32,
    pub speed_boost_factor: u32,
    pub max_velocity: u32,
    pub sig: u32,
    pub sig_bonus: u32,
    pub sig_bonus_pct: u32,
    pub slot_mods: [(u32, u32); 3],
    pub hp_mods: [(u32, u32); 2],
    pub remote_resist: u32,
    pub max_target_range: u32,
    pub max_target_range_bonus: u32,
    pub scan_res: u32,
    pub scan_res_bonus: u32,
    pub warfare: [(u32, u32); 4],
    pub armor_res: [u32; 4],
    pub shift: u32,
    pub rounded: Vec<u32>,
    /// sorted ids of attributes with min/max caps
    pub capped: Vec<u32>,
    pub v: ValidateIds,
}

pub struct ValidateIds {
    pub can_fit_groups: Vec<u32>,
    pub can_fit_types: Vec<u32>,
    pub charge_groups: Vec<u32>,
    pub req_skill: [u32; 6],
    pub req_level: [u32; 6],
}

impl Consts {
    /// attribute with a min/max cap or output rounding (always evaluated through `attr_value`)
    #[inline]
    pub fn special(&self, attr: u32) -> bool {
        self.capped.binary_search(&attr).is_ok() || self.rounded.contains(&attr)
    }

    pub fn new(ds: &Dataset) -> Consts {
        let a = |n: &str| ds.attr_id(n);
        let e = |n: &str| ds.effect_id(n);
        let w = |k: u32| (a(&format!("warfareBuff{k}ID")), a(&format!("warfareBuff{k}Value")));
        Consts {
            e_ab: e("moduleBonusAfterburner"),
            e_mwd: e("moduleBonusMicrowarpdrive"),
            e_slot: e("slotModifier"),
            e_hp: e("hardPointModifierEffect"),
            e_mjd: e("microJumpDrive"),
            e_bastion: e("moduleBonusBastionModule"),
            e_rah: e("adaptiveArmorHardener"),
            structure_ok: STRUCTURE_SKILL_EFFECT_NAMES.iter().map(|n| e(n)).collect(),
            mass_addition: a("massAddition"),
            speed_factor: a("speedFactor"),
            speed_boost_factor: a("speedBoostFactor"),
            max_velocity: a("maxVelocity"),
            sig: a("signatureRadius"),
            sig_bonus: a("signatureRadiusBonus"),
            sig_bonus_pct: a("signatureRadiusBonusPercent"),
            slot_mods: [(a("hiSlots"), a("hiSlotModifier")), (a("medSlots"), a("medSlotModifier")), (a("lowSlots"), a("lowSlotModifier"))],
            hp_mods: [(a("turretSlotsLeft"), a("turretHardPointModifier")), (a("launcherSlotsLeft"), a("launcherHardPointModifier"))],
            remote_resist: a("remoteResistanceID"),
            max_target_range: a("maxTargetRange"),
            max_target_range_bonus: a("maxTargetRangeBonus"),
            scan_res: a("scanResolution"),
            scan_res_bonus: a("scanResolutionBonus"),
            warfare: [w(1), w(2), w(3), w(4)],
            armor_res: [a(ARMOR_RES[0]), a(ARMOR_RES[1]), a(ARMOR_RES[2]), a(ARMOR_RES[3])],
            shift: a("resistanceShiftAmount"),
            rounded: ["cpu", "power", "cpuOutput", "powerOutput"].iter().map(|n| a(n)).filter(|x| *x != 0).collect(),
            capped: {
                let mut v: Vec<u32> = ds.attrs.values().filter(|i| i.min_attr.is_some() || i.max_attr.is_some()).map(|i| i.id).collect();
                v.sort();
                v
            },
            v: ValidateIds {
                can_fit_groups: (1..=20).map(|k| a(&format!("canFitShipGroup{k:02}"))).filter(|x| *x != 0).collect(),
                can_fit_types: (1..=11).map(|k| a(&format!("canFitShipType{k}"))).filter(|x| *x != 0).collect(),
                charge_groups: (1..=5).map(|k| a(&format!("chargeGroup{k}"))).collect(),
                req_skill: [a("requiredSkill1"), a("requiredSkill2"), a("requiredSkill3"), a("requiredSkill4"), a("requiredSkill5"), a("requiredSkill6")],
                req_level: [a("requiredSkill1Level"), a("requiredSkill2Level"), a("requiredSkill3Level"), a("requiredSkill4Level"), a("requiredSkill5Level"), a("requiredSkill6Level")],
            },
        }
    }
}

#[salsa::input]
pub struct ItemIn {
    #[returns(copy)]
    pub slot: u32,
    #[returns(ref)]
    pub spec: Arc<ItemSpec>,
}

#[salsa::input]
pub struct FitIn {
    /// item handle per slot (slots are never reused for a different key)
    #[returns(ref)]
    pub slots: Vec<ItemIn>,
    /// present items in canonical (registration) order
    #[returns(ref)]
    pub order: Vec<u32>,
    #[returns(ref)]
    pub ctx: Arc<Ctx>,
    #[returns(copy)]
    pub core: Core,
}

#[salsa::interned]
pub struct AKey<'db> {
    #[returns(copy)]
    pub item: u32,
    #[returns(copy)]
    pub attr: u32,
    #[returns(copy)]
    pub layer: u32,
}

#[salsa::interned]
pub struct LKey<'db> {
    #[returns(copy)]
    pub layer: u32,
}

/// Structural per-item info for target filters (changes only when items are added/removed/retyped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdxItem {
    pub slot: u32,
    pub kind: Kind,
    pub loc: Loc,
    pub group: u32,
    pub owned: bool,
    pub type_id: u32,
    pub req_skills: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    pub items: Vec<IdxItem>,
    /// target sets in canonical order: (set kind, filter) -> slots
    pub sets: FxHashMap<(u8, u32), Vec<u32>>,
}

/// set kinds for `target_set`
pub const S_SHIP_ALL: u8 = 0;
pub const S_SHIP_GROUP: u8 = 1;
pub const S_SHIP_SKILL: u8 = 2;
pub const S_OWNED_SKILL: u8 = 3;
pub const S_CHAR_ALL: u8 = 4;
pub const S_CHAR_GROUP: u8 = 5;
pub const S_CHAR_SKILL: u8 = 6;

#[salsa::interned]
pub struct TKey<'db> {
    #[returns(copy)]
    pub kind: u8,
    #[returns(copy)]
    pub extra: u32,
}

/// One modifier target set; backdates when unchanged, so an item's `outgoing` only re-runs when a set it
/// actually targets changes (e.g. a skill bonus to "modules requiring Gunnery" when a turret is added).
#[salsa::tracked(returns(ref))]
pub fn target_set<'db>(db: &'db dyn Db, fit: FitIn, k: TKey<'db>) -> Arc<Vec<u32>> {
    Arc::new(index(db, fit).sets.get(&(k.kind(db), k.extra(db))).cloned().unwrap_or_default())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Core {
    pub ship: u32,
    pub char: u32,
    pub is_structure: bool,
}

#[salsa::tracked(returns(ref))]
pub fn index(db: &dyn Db, fit: FitIn) -> Arc<Index> {
    let slots = fit.slots(db);
    let mut items = Vec::with_capacity(fit.order(db).len());
    let mut sets: FxHashMap<(u8, u32), Vec<u32>> = FxHashMap::default();
    for &s in fit.order(db) {
        let sp = slots[s as usize].spec(db);
        let mut add = |k: u8, e: u32| sets.entry((k, e)).or_default().push(s);
        if sp.loc == Loc::Ship {
            add(S_SHIP_ALL, 0);
            add(S_SHIP_GROUP, sp.group);
            for &r in &sp.req_skills {
                add(S_SHIP_SKILL, r);
            }
        }
        if sp.owned {
            for &r in &sp.req_skills {
                add(S_OWNED_SKILL, r);
            }
        }
        if sp.loc == Loc::Char {
            add(S_CHAR_ALL, 0);
            add(S_CHAR_GROUP, sp.group);
        }
        if (sp.owned || sp.loc == Loc::Char) && sp.kind != Kind::Skill {
            for &r in &sp.req_skills {
                add(S_CHAR_SKILL, r);
            }
        }
        items.push(IdxItem { slot: s, kind: sp.kind, loc: sp.loc, group: sp.group, owned: sp.owned, type_id: sp.type_id, req_skills: sp.req_skills.clone() });
    }
    // req_skills may repeat a skill: keep each slot once per set (the reference filters items, it doesn't duplicate)
    for v in sets.values_mut() {
        v.dedup();
    }
    Arc::new(Index { items, sets })
}

/// charge/parent links (slot ids) - separate query so link changes don't touch everything else
#[salsa::tracked(returns(copy))]
pub fn links(db: &dyn Db, item: ItemIn) -> (Option<u32>, Option<u32>) {
    let sp = item.spec(db);
    (sp.charge.map(|c| c as u32), sp.parent.map(|p| p as u32))
}

fn state_ok(category: u8, state: State) -> bool {
    match category {
        0 | 4 => state >= State::Online,
        1 => state >= State::Active,
        5 => state >= State::Overheated,
        7 => true,
        _ => false,
    }
}

fn penalized(ds: &Dataset, attr: u32, source_cat: u32) -> bool {
    let stackable = ds.attrs.get(&attr).map(|a| a.stackable).unwrap_or(true);
    !stackable && !EXEMPT_CATEGORIES.contains(&source_cat)
}

/// A modifier's target: nothing, one slot, or an indexed target set (kind, filter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sel {
    None,
    One(u32),
    Set(u8, u32),
}

fn selector(core: Core, src: u32, links: (Option<u32>, Option<u32>), func: Func, domain: Domain, extra: u32) -> Sel {
    match domain {
        Domain::Item => {
            if func == Func::Item {
                Sel::One(src)
            } else {
                Sel::None
            }
        }
        Domain::Other => match (links.0, links.1) {
            (Some(c), _) => Sel::One(c),
            (None, Some(p)) => Sel::One(p),
            _ => Sel::None,
        },
        Domain::Ship | Domain::Structure => {
            if domain == Domain::Structure && !core.is_structure {
                return Sel::None;
            }
            match func {
                Func::Item => Sel::One(core.ship),
                Func::Location => Sel::Set(S_SHIP_ALL, 0),
                Func::LocationGroup => Sel::Set(S_SHIP_GROUP, extra),
                Func::LocationRequiredSkill => Sel::Set(S_SHIP_SKILL, extra),
                Func::OwnerRequiredSkill => Sel::Set(S_OWNED_SKILL, extra),
                Func::EffectStopper => Sel::None,
            }
        }
        Domain::Char => match func {
            Func::Item => Sel::One(core.char),
            Func::Location => Sel::Set(S_CHAR_ALL, 0),
            Func::LocationGroup => Sel::Set(S_CHAR_GROUP, extra),
            Func::LocationRequiredSkill | Func::OwnerRequiredSkill => Sel::Set(S_CHAR_SKILL, extra),
            Func::EffectStopper => Sel::None,
        },
        _ => Sel::None,
    }
}

/// `Out.target` with this bit set is a deferred target set: bits 24..31 = set kind, 0..24 = filter.
/// `outgoing` then depends only on the item (not on what else is fitted); `incoming` expands it.
pub const SET_BIT: u32 = 0x8000_0000;

fn targets(db: &dyn Db, fit: FitIn, core: Core, src: u32, links: (Option<u32>, Option<u32>), func: Func, domain: Domain, extra: u32, out: &mut Vec<u32>) {
    match selector(core, src, links, func, domain, extra) {
        Sel::None => {}
        Sel::One(t) => out.push(t),
        Sel::Set(k, e) => out.extend_from_slice(target_set(db, fit, TKey::new(db, k, e))),
    }
}

fn effective_state(db: &dyn Db, fit: FitIn, sp: &ItemSpec) -> State {
    match sp.kind {
        Kind::Charge => sp.parent.map(|p| fit.slots(db)[p].spec(db).state).unwrap_or(State::Online),
        Kind::Ship | Kind::Char | Kind::Skill | Kind::Implant | Kind::Booster | Kind::Mode | Kind::Beacon => State::Online,
        Kind::Drone | Kind::Fighter => {
            if sp.active_count > 0 {
                State::Active
            } else {
                State::Offline
            }
        }
        _ => sp.state,
    }
}

/// Modifiers emitted by one item (its effects applied to their targets), in registration order.
#[salsa::tracked(returns(ref))]
pub fn outgoing(db: &dyn Db, fit: FitIn, item: ItemIn) -> Arc<Vec<Out>> {
    Arc::new(outgoing_impl(db, fit, item))
}

fn outgoing_impl(db: &dyn Db, fit: FitIn, item: ItemIn) -> Vec<Out> {
    let ds = db.ds();
    let c = db.consts();
    let sp = item.spec(db).clone();
    let i = item.slot(db);
    let ctx = fit.core(db);
    let mut out: Vec<Out> = Vec::new();
    let push = |out: &mut Vec<Out>, target: u32, attr: u32, op: i32, src: Src, cat: u32| {
        out.push(Out { target, attr, m: AMod { op: op as i8, penalized: penalized(ds, attr, cat), src } });
    };
    let kind = sp.kind;
    if kind == Kind::Projected {
        projected(db, fit, &sp, i, &ctx, &mut out);
        return out;
    }
    if ctx.is_structure && matches!(kind, Kind::Drone | Kind::Implant | Kind::Booster) {
        return out;
    }
    let state = effective_state(db, fit, &sp);
    let src_cat = sp.category;
    let lk = links(db, item);
    let ship = ctx.ship;
    let mut tg: Vec<u32> = Vec::new();
    for &(eid, is_default) in &sp.effects {
        if eid == EFFECT_SKILL_EFFECT {
            continue;
        }
        let Some(e) = ds.effects.get(&eid) else { continue };
        if ctx.is_structure && kind == Kind::Skill && !c.structure_ok.contains(&eid) && !e.mods.iter().all(|m| m.domain == Domain::Item) {
            continue;
        }
        if e.fitting_usage_chance_attr.is_some() && !sp.booster_side_effects.contains(&eid) {
            continue;
        }
        if kind == Kind::Fighter && e.category != 0 {
            let used = match &sp.fighter_abilities {
                Some(a) => a.contains(&eid),
                None => is_default,
            };
            if !used {
                continue;
            }
        }
        // Pyfa 'active' handlers for SDE effects without modifiers (some are target-category in the SDE)
        if e.mods.is_empty() && kind == Kind::Module && state >= State::Active && local_special(ds, &sp, i, ship, e.name.as_str(), src_cat, &mut out) {
            continue;
        }
        if !state_ok(e.category, state) {
            continue;
        }
        if kind == Kind::Fighter && e.mods.is_empty() {
            // fighter self abilities (Pyfa hand-written handlers, eos LGPL; as in eve-dogma-rs)
            let fm: &[(&str, &str, i32)] = match e.name.as_str() {
                "fighterAbilityMicroWarpDrive" => &[("maxVelocity", "fighterAbilityMicroWarpDriveSpeedBonus", 6), ("signatureRadius", "fighterAbilityMicroWarpDriveSignatureRadiusBonus", 6)],
                "fighterAbilityAfterburner" => &[("maxVelocity", "fighterAbilityAfterburnerSpeedBonus", 6)],
                "fighterAbilityEvasiveManeuvers" => &[
                    ("maxVelocity", "fighterAbilityEvasiveManeuversSpeedBonus", 6),
                    ("signatureRadius", "fighterAbilityEvasiveManeuversSignatureRadiusBonus", 6),
                    ("shieldEmDamageResonance", "fighterAbilityEvasiveManeuversEmResonance", 4),
                    ("shieldThermalDamageResonance", "fighterAbilityEvasiveManeuversThermResonance", 4),
                    ("shieldKineticDamageResonance", "fighterAbilityEvasiveManeuversKinResonance", 4),
                    ("shieldExplosiveDamageResonance", "fighterAbilityEvasiveManeuversExpResonance", 4),
                ],
                _ => &[],
            };
            if !fm.is_empty() {
                for (t, a, op) in fm {
                    push(&mut out, i, ds.attr_id(t), *op, Src::Attr { item: i, attr: ds.attr_id(a) }, src_cat);
                }
                continue;
            }
        }
        if eid == c.e_ab || eid == c.e_mwd {
            push(&mut out, ship, 4, 2, Src::Attr { item: i, attr: c.mass_addition }, src_cat);
            let src = Src::Prop { module: i, ship, speed: c.speed_factor, thrust: c.speed_boost_factor, mass: 4 };
            push(&mut out, ship, c.max_velocity, 4, src, src_cat);
            if eid == c.e_mwd {
                push(&mut out, ship, c.sig, 6, Src::Attr { item: i, attr: c.sig_bonus }, src_cat);
            }
            continue;
        }
        if eid == c.e_mjd {
            push(&mut out, ship, c.sig, 6, Src::Attr { item: i, attr: c.sig_bonus_pct }, 6);
            continue;
        }
        if eid == c.e_slot {
            for (t, s) in c.slot_mods {
                push(&mut out, ship, t, 2, Src::Attr { item: i, attr: s }, src_cat);
            }
            continue;
        }
        if eid == c.e_hp {
            for (t, s) in c.hp_mods {
                push(&mut out, ship, t, 2, Src::Attr { item: i, attr: s }, src_cat);
            }
            continue;
        }
        for m in &e.mods {
            if m.func == Func::EffectStopper || m.op == 9 {
                continue;
            }
            if matches!(m.domain, Domain::TargetId | Domain::Target) {
                continue;
            }
            let extra = if m.extra == 0 && matches!(m.func, Func::LocationRequiredSkill | Func::OwnerRequiredSkill) { sp.type_id } else { m.extra };
            let cat = if eid == c.e_bastion && HULL_RESONANCES.contains(&m.modified) { 6 } else { src_cat };
            let src = Src::Attr { item: i, attr: m.modifying };
            match selector(ctx, i, lk, m.func, m.domain, extra) {
                Sel::None => {}
                Sel::One(t) => push(&mut out, t, m.modified, m.op, src, cat),
                Sel::Set(k, e) if e < (1 << 24) => push(&mut out, SET_BIT | (k as u32) << 24 | e, m.modified, m.op, src, cat),
                Sel::Set(k, e) => {
                    tg.clear();
                    tg.extend_from_slice(target_set(db, fit, TKey::new(db, k, e)));
                    for &t in &tg {
                        push(&mut out, t, m.modified, m.op, src, cat);
                    }
                }
            }
        }
    }
    out
}

/// Local module effects that have no modifierInfo in the SDE but a hand-written Pyfa handler (eos/effects.py,
/// LGPL; re-expressed as in eve-dogma-rs). Returns true when handled. Source category 6 = no stacking penalty.
fn local_special(ds: &Dataset, sp: &ItemSpec, i: u32, ship: u32, name: &str, src_cat: u32, out: &mut Vec<Out>) -> bool {
    let a = |n: &str| ds.attr_id(n);
    let mut push = |target: u32, attr: u32, op: i32, src: Src, cat: u32| {
        out.push(Out { target, attr, m: AMod { op: op as i8, penalized: penalized(ds, attr, cat), src } });
    };
    let at = |n: &str| Src::Attr { item: i, attr: ds.attr_id(n) };
    match name {
        "superWeaponAmarr" | "superWeaponCaldari" | "superWeaponGallente" | "superWeaponMinmatar" | "doomsdaySlash" | "doomsdayBeamDOT"
        | "doomsdayConeDOT" | "doomsdayHOG" | "debuffLance" => {
            push(ship, a("maxVelocity"), 6, at("speedFactor"), src_cat);
            push(ship, a("warpScrambleStatus"), 2, at("siegeModeWarpStatus"), src_cat);
        }
        "emergencyHullEnergizer" => {
            for t in ["Em", "Thermal", "Kinetic", "Explosive"] {
                push(ship, a(&format!("{}DamageResonance", t.to_lowercase())), 4, at(&format!("hull{t}DamageResonance")), src_cat);
            }
        }
        "entosisLink" => {
            push(ship, a("disallowAssistance"), 7, at("disallowAssistance"), 6);
            for t in ["Gravimetric", "Magnetometric", "Radar", "Ladar"] {
                push(ship, a(&format!("scan{t}Strength")), 6, at(&format!("scan{t}StrengthPercent")), src_cat);
            }
        }
        "microJumpPortalDrive" | "microJumpPortalDriveCapital" => {
            push(ship, a("signatureRadius"), 6, at("signatureRadiusBonusPercent"), src_cat);
        }
        "warpDisruptSphere" => {
            push(ship, a("disallowAssistance"), 7, Src::Const(F(1.0)), 6);
            if sp.charge.is_none() {
                push(ship, 4, 6, at("massBonusPercentage"), 6);
                push(ship, a("signatureRadius"), 6, at("signatureRadiusBonus"), 6);
                // every fitted propulsion module (deferred ship-group target set; groups hold ship-located items only)
                let mut groups: Vec<u32> = ds.groups.iter().filter(|(_, g)| g.name == "Propulsion Module").map(|(k, _)| *k).collect();
                groups.sort_unstable();
                for g in groups {
                    let set = SET_BIT | (S_SHIP_GROUP as u32) << 24 | g;
                    push(set, a("speedBoostFactor"), 6, at("speedBoostFactorBonus"), 6);
                    push(set, a("speedFactor"), 6, at("speedFactorBonus"), 6);
                }
            }
        }
        _ => return false,
    }
    true
}

/// Effects of a projected item that take part in projection (category / state / fighter-ability filters).
pub fn proj_effects<'a>(ds: &'a Dataset, sp: &ItemSpec) -> Vec<&'a crate::data::EffectInfo> {
    let mut v = Vec::new();
    for &(eid, _) in &sp.effects {
        let Some(e) = ds.effects.get(&eid) else { continue };
        if e.category != 2 && e.category != 3 && e.name != "ECMBurstJammer" {
            continue;
        }
        if let Some(ab) = &sp.fighter_abilities {
            if e.name.starts_with("fighterAbility") && !ab.contains(&eid) {
                continue;
            }
        }
        if sp.state < State::Active {
            continue;
        }
        v.push(e);
    }
    v
}

pub fn proj_resist(ds: &Dataset, sp: &ItemSpec, e: &crate::data::EffectInfo) -> u32 {
    e.resistance_attr.unwrap_or_else(|| {
        let look = |n: &str| sp.base(ds.attr_id(n)).map(|a| a as u32).unwrap_or(0);
        if e.name.starts_with("fighterAbility") {
            let r = look(&format!("{}ResistanceID", e.name));
            if r != 0 { r } else { look(&format!("{}RemoteResistanceID", e.name)) }
        } else {
            look("remoteResistanceID")
        }
    })
}

fn is_basic_projection(name: &str) -> bool {
    name.starts_with("remoteWebifier")
        || name == "structureModuleEffectStasisWebifier"
        || name.starts_with("remoteTargetPaint")
        || name == "structureModuleEffectTargetPainter"
        || name.starts_with("remoteSensorDamp")
        || name == "structureModuleEffectRemoteSensorDampener"
        || name.starts_with("remoteSensorBoost")
        || name == "shipModuleTrackingDisruptor"
        || name == "shipModuleGuidanceDisruptor"
        || name == "shipModuleRemoteTrackingComputer"
        || name == "npcEntityWeaponDisruptor"
}

fn projected(db: &dyn Db, fit: FitIn, sp: &ItemSpec, i: u32, ctx: &Core, out: &mut Vec<Out>) {
    let ds = db.ds();
    let c = db.consts();
    let src_cat = sp.category;
    let ship = ctx.ship;
    let ship_sp = fit.slots(db)[ship as usize].spec(db);
    let target_offense_ok = ship_sp.base(ds.attr_id("disallowOffensiveModifiers")).map(|a| a == 0.0).unwrap_or(true);
    let qty = sp.quantity.max(1) as f64;
    for e in proj_effects(ds, sp) {
        let opt = e.range_attr.and_then(|a| sp.base(a)).unwrap_or(0.0);
        let fo = e.falloff_attr.and_then(|a| sp.base(a)).unwrap_or(0.0);
        let factor = crate::stats::range_factor(opt, fo, sp.distance, true);
        let resist = proj_resist(ds, sp, e);
        let push = |out: &mut Vec<Out>, target_attr: u32, op: i32, src: Src| {
            out.push(Out { target: ship, attr: target_attr, m: AMod { op: op as i8, penalized: penalized(ds, target_attr, src_cat), src } });
        };
        let pj = |src_attr: u32, op: i32| Src::Projected { item: i, attr: src_attr, factor: F(factor), target: ship, resist, mul: op == 4 || op == 0 };
        if !e.mods.is_empty() {
            for m in &e.mods {
                if matches!(m.domain, Domain::TargetId | Domain::Target | Domain::Ship) && m.func == Func::Item {
                    push(out, m.modified, m.op, pj(m.modifying, m.op));
                }
            }
            continue;
        }
        let name = e.name.as_str();
        let pbase = |n: &str| sp.base(ds.attr_id(n)).unwrap_or(0.0);
        if name == "fighterAbilityStasisWebifier" {
            if target_offense_ok {
                let f = crate::stats::range_factor(pbase("fighterAbilityStasisWebifierOptimalRange"), pbase("fighterAbilityStasisWebifierFalloffRange"), sp.distance, true) * qty;
                let src = Src::Projected { item: i, attr: ds.attr_id("fighterAbilityStasisWebifierSpeedPenalty"), factor: F(f), target: ship, resist, mul: false };
                push(out, c.max_velocity, 6, src);
            }
            continue;
        }
        if name == "fighterAbilityWarpDisruption" {
            if target_offense_ok && pbase("fighterAbilityWarpDisruptionRange") >= sp.distance.unwrap_or(0.0) {
                let src = Src::Projected { item: i, attr: ds.attr_id("fighterAbilityWarpDisruptionPointStrength"), factor: F(qty), target: ship, resist, mul: false };
                push(out, ds.attr_id("warpScrambleStatus"), 2, src);
            }
            continue;
        }
        if name.starts_with("remoteWebifier") || name == "structureModuleEffectStasisWebifier" {
            push(out, c.max_velocity, 6, pj(c.speed_factor, 6));
        } else if name.starts_with("remoteTargetPaint") || name == "structureModuleEffectTargetPainter" {
            push(out, c.sig, 6, pj(c.sig_bonus, 6));
        } else if name.starts_with("remoteSensorDamp") || name == "structureModuleEffectRemoteSensorDampener" {
            push(out, c.max_target_range, 6, pj(c.max_target_range_bonus, 6));
            push(out, c.scan_res, 6, pj(c.scan_res_bonus, 6));
        } else if name == "shipModuleTrackingDisruptor" || name == "shipModuleGuidanceDisruptor" || name == "shipModuleRemoteTrackingComputer" || name == "npcEntityWeaponDisruptor" {
            // Pyfa Effect6424 / Effect6423 / shipModuleRemoteTrackingComputer / Effect6694: modify the target's gunnery
            // modules (TD, remote tracking computer, TD drones) / missile charges (GD)
            let allowed = if name == "shipModuleRemoteTrackingComputer" {
                ship_sp.base(ds.attr_id("disallowAssistance")).map(|a| a == 0.0).unwrap_or(true)
            } else {
                target_offense_ok
            };
            if allowed {
                let (skill, charges, pairs): (&str, bool, &[(&str, &str)]) = if name != "shipModuleGuidanceDisruptor" {
                    ("Gunnery", false, &[("trackingSpeedBonus", "trackingSpeed"), ("maxRangeBonus", "maxRange"), ("falloffBonus", "falloff")])
                } else {
                    ("Missile Launcher Operation", true, &[("aoeCloudSizeBonus", "aoeCloudSize"), ("aoeVelocityBonus", "aoeVelocity"), ("missileVelocityBonus", "maxVelocity"), ("explosionDelayBonus", "explosionDelay")])
                };
                let sk = ds.type_by_name(skill).unwrap_or(0);
                let tf = if name == "npcEntityWeaponDisruptor" {
                    // TD drones (Pyfa Effect6694): full strength inside maxRange, nothing beyond
                    if pbase("maxRange") < sp.distance.unwrap_or(0.0) { 0.0 } else { 1.0 }
                } else {
                    crate::stats::range_factor(pbase("maxRange"), pbase("falloffEffectiveness"), sp.distance, true)
                };
                for it in index(db, fit).items.iter() {
                    let kind_ok = if charges { it.kind == Kind::Charge } else { it.kind == Kind::Module };
                    if it.loc == Loc::Ship && it.owned && kind_ok && it.req_skills.contains(&sk) {
                        for (src_a, tgt_a) in pairs {
                            let ta = ds.attr_id(tgt_a);
                            let src = Src::Projected { item: i, attr: ds.attr_id(src_a), factor: F(tf), target: ship, resist, mul: false };
                            out.push(Out { target: it.slot, attr: ta, m: AMod { op: 6, penalized: penalized(ds, ta, src_cat), src } });
                        }
                    }
                }
            }
        } else if name.starts_with("remoteSensorBoost") {
            push(out, c.max_target_range, 6, pj(c.max_target_range_bonus, 6));
            push(out, c.scan_res, 6, pj(c.scan_res_bonus, 6));
            for t in ["Gravimetric", "Ladar", "Magnetometric", "Radar"] {
                push(out, ds.attr_id(&format!("scan{t}Strength")), 6, pj(ds.attr_id(&format!("scan{t}StrengthPercent")), 6));
            }
        }
    }
}

pub const DAMAGE_EFFECTS: &[&str] = &["projectileFired", "targetAttack", "useMissiles", "barrage", "targetDisintegratorAttack",
    "missileLaunchingForEntity", "fighterAbilityAttackM", "fighterAbilityMissiles", "superWeaponAmarr", "superWeaponCaldari",
    "superWeaponGallente", "superWeaponMinmatar", "mining", "miningLaser", "miningClouds", "dotMissileLaunching", "ChainLightning", "salvageDroneEffect"];
pub const PROJ_SPECIAL_EFFECTS: &[&str] = &["shipModuleRemoteShieldBooster", "shipModuleAncillaryRemoteShieldBooster", "shipModuleRemoteArmorRepairer",
    "ShipModuleRemoteArmorMutadaptiveRepairer", "shipModuleAncillaryRemoteArmorRepairer", "shipModuleRemoteHullRepairer",
    "npcEntityRemoteShieldBooster", "npcEntityRemoteArmorRepairer", "npcEntityRemoteHullRepairer", "shipModuleRemoteCapacitorTransmitter",
    "energyNeutralizerFalloff", "energyNosferatuFalloff", "structureEnergyNeutralizerFalloff", "entityEnergyNeutralizerFalloff",
    "fighterAbilityEnergyNeutralizer", "remoteECMFalloff", "structureModuleEffectECM", "entityECMFalloff", "ECMBurstJammer", "fighterAbilityECM"];

/// warnings that the reference emits during registration (projected effects not modelled)
pub fn projected_warnings(ds: &Dataset, sp: &ItemSpec) -> Vec<String> {
    let mut w = Vec::new();
    for e in proj_effects(ds, sp) {
        if !e.mods.is_empty() {
            continue;
        }
        let name = e.name.as_str();
        if !(name == "fighterAbilityStasisWebifier"
            || name == "fighterAbilityWarpDisruption"
            || is_basic_projection(name)
            || PROJ_SPECIAL_EFFECTS.contains(&name)
            || DAMAGE_EFFECTS.contains(&name))
        {
            w.push(format!("projected effect '{name}' not modelled yet"));
        }
    }
    w
}

fn buff_mods(db: &dyn Db, fit: FitIn, ctx: Core, id: u32, src: Src, out: &mut Vec<Out>) {
    let ds = db.ds();
    let Some(info) = ds.dbuffs.get(&id) else { return };
    let op = info.op as i8;
    let mut push = |t: u32, a: u32| out.push(Out { target: t, attr: a, m: AMod { op, penalized: penalized(ds, a, 0), src } });
    for &a in &info.item {
        push(ctx.ship, a);
    }
    let mut tg = Vec::new();
    for &a in &info.location {
        tg.clear();
        targets(db, fit, ctx, ctx.ship, (None, None), Func::Location, Domain::Ship, 0, &mut tg);
        for &t in &tg {
            push(t, a);
        }
    }
    for &(a, g) in &info.location_group {
        tg.clear();
        targets(db, fit, ctx, ctx.ship, (None, None), Func::LocationGroup, Domain::Ship, g, &mut tg);
        for &t in &tg {
            push(t, a);
        }
    }
    for &(a, s) in &info.location_skill {
        tg.clear();
        targets(db, fit, ctx, ctx.ship, (None, None), Func::LocationRequiredSkill, Domain::Ship, s, &mut tg);
        for &t in &tg {
            push(t, a);
        }
    }
}

pub type ModMap = FxHashMap<u32, Vec<AMod>>;

/// Layer-0 modifiers per target slot.
#[salsa::tracked(returns(ref))]
pub fn incoming(db: &dyn Db, fit: FitIn) -> Arc<Vec<Arc<ModMap>>> {
    let slots = fit.slots(db);
    let mut maps: Vec<ModMap> = vec![ModMap::default(); slots.len()];
    let idx = index(db, fit);
    for &s in fit.order(db) {
        let outs: &Arc<Vec<Out>> = outgoing(db, fit, slots[s as usize]);
        for o in outs.iter() {
            if o.target & SET_BIT != 0 {
                let k = ((o.target >> 24) & 0x7f) as u8;
                if let Some(v) = idx.sets.get(&(k, o.target & 0x00ff_ffff)) {
                    for &t in v {
                        maps[t as usize].entry(o.attr).or_default().push(o.m);
                    }
                }
            } else {
                maps[o.target as usize].entry(o.attr).or_default().push(o.m);
            }
        }
    }
    let empty = Arc::new(ModMap::default());
    Arc::new(maps.into_iter().map(|m| if m.is_empty() { empty.clone() } else { Arc::new(m) }).collect())
}

#[salsa::tracked(returns(ref))]
pub fn item_mods(db: &dyn Db, fit: FitIn, item: ItemIn) -> Arc<ModMap> {
    incoming(db, fit)[item.slot(db) as usize].clone()
}

/// Local command bursts (need evaluated warfareBuffNID at layer 0).
#[salsa::tracked(returns(ref))]
pub fn burst_mods(db: &dyn Db, fit: FitIn) -> Arc<Vec<Out>> {
    let c = db.consts();
    let ctx = fit.ctx(db);
    let core = fit.core(db);
    let slots = fit.slots(db);
    let mut out = Vec::new();
    // Pyfa keeps, per buff id, the strongest (|value|) source among own bursts and booster fits;
    // explicit fleet.buffs override both.
    let explicit = |id: u32| ctx.buffs.iter().any(|b| b.0 == id);
    let mut best: Vec<(u32, f64, Src)> = Vec::new();
    let offer = |best: &mut Vec<(u32, f64, Src)>, id: u32, v: f64, src: Src| match best.iter_mut().find(|b| b.0 == id) {
        Some(b) if b.1.abs() >= v.abs() => {}
        Some(b) => {
            b.1 = v;
            b.2 = src
        }
        None => best.push((id, v, src)),
    };
    for &s in fit.order(db) {
        let sp = slots[s as usize].spec(db);
        if sp.kind != Kind::Module || sp.state < State::Active {
            continue;
        }
        for (ida, vala) in c.warfare {
            let id = if has(db, fit, s, ida) { value(db, fit, s, ida, 0) as u32 } else { 0 };
            if id == 0 || explicit(id) {
                continue;
            }
            let v = value(db, fit, s, vala, 0);
            offer(&mut best, id, v, Src::Attr { item: s, attr: vala });
        }
    }
    for &(id, v) in &ctx.booster_offers {
        if id == 0 || explicit(id) {
            continue;
        }
        offer(&mut best, id, v.0, Src::Const(v));
    }
    for &(id, v) in &ctx.buffs {
        match best.iter_mut().find(|b| b.0 == id) {
            Some(b) => {
                b.1 = v.0;
                b.2 = Src::Const(v)
            }
            None => best.push((id, v.0, Src::Const(v))),
        }
    }
    best.sort_by_key(|b| b.0);
    for (id, _, src) in best {
        buff_mods(db, fit, core, id, src, &mut out);
    }
    Arc::new(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub burst: bool,
    pub rah: Vec<u32>,
    pub final_layer: u32,
}

#[salsa::tracked(returns(ref))]
pub fn plan(db: &dyn Db, fit: FitIn) -> Plan {
    let c = db.consts();
    let slots = fit.slots(db);
    let rah: Vec<u32> = if c.e_rah == 0 {
        vec![]
    } else {
        fit.order(db)
            .iter()
            .copied()
            .filter(|&s| {
                let sp = slots[s as usize].spec(db);
                sp.kind == Kind::Module && sp.state >= State::Active && sp.effects.iter().any(|(e, _)| *e == c.e_rah)
            })
            .collect()
    };
    let burst = !burst_mods(db, fit).is_empty();
    let final_layer = burst as u32 + rah.len() as u32;
    Plan { burst, rah, final_layer }
}

pub type LayerMap = FxHashMap<(u32, u32), Vec<AMod>>;

/// Modifiers introduced at layer L (>0): local bursts, then one layer per RAH.
#[salsa::tracked(returns(ref))]
pub fn layer_mods<'db>(db: &'db dyn Db, fit: FitIn, l: LKey<'db>) -> Arc<LayerMap> {
    let layer = l.layer(db);
    let p = plan(db, fit);
    let mut m = LayerMap::default();
    if p.burst && layer == 1 {
        for o in burst_mods(db, fit).iter() {
            m.entry((o.target, o.attr)).or_default().push(o.m);
        }
        return Arc::new(m);
    }
    let k = (layer - 1 - p.burst as u32) as usize;
    let Some(&mo) = p.rah.get(k) else { return Arc::new(m) };
    let ds = db.ds();
    let c = db.consts();
    let ctx = fit.ctx(db);
    let core = fit.core(db);
    let prev = layer - 1;
    let attrs = c.armor_res;
    let ship = core.ship;
    let mut res: Vec<f64> = attrs.iter().map(|&a| value(db, fit, mo, a, prev)).collect();
    if !ctx.rah_disable {
        let pattern = [ctx.pattern[0].0, ctx.pattern[1].0, ctx.pattern[2].0, ctx.pattern[3].0];
        let base: Vec<f64> = (0..4).map(|k| pattern[k] * value(db, fit, ship, attrs[k], prev)).collect();
        let shift = value(db, fit, mo, c.shift, prev) / 100.0;
        let mut cycles: Vec<[f64; 4]> = Vec::new();
        let mut loop_start: isize = -20;
        for _ in 0..50 {
            let mut t: Vec<(usize, f64, f64)> = [0usize, 3, 2, 1].iter().map(|&k| (k, base[k] * res[k], res[k])).collect();
            t.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            let (c0, c1, c2, c3);
            if t[2].1 == 0.0 {
                c0 = 1.0 - t[0].2;
                c1 = 1.0 - t[1].2;
                c2 = 1.0 - t[2].2;
                c3 = -(c0 + c1 + c2);
            } else if t[1].1 == 0.0 {
                c0 = 1.0 - t[0].2;
                c1 = 1.0 - t[1].2;
                c2 = -(c0 + c1) / 2.0;
                c3 = c2;
            } else {
                c0 = shift.min(1.0 - t[0].2);
                c1 = shift.min(1.0 - t[1].2);
                c2 = -(c0 + c1) / 2.0;
                c3 = c2;
            }
            res[t[0].0] = t[0].2 + c0;
            res[t[1].0] = t[1].2 + c1;
            res[t[2].0] = t[2].2 + c2;
            res[t[3].0] = t[3].2 + c3;
            if let Some(i) = cycles.iter().position(|v| (0..4).all(|k| (res[k] - v[k]).abs() <= 1e-6)) {
                loop_start = i as isize;
                break;
            }
            cycles.push([res[0], res[1], res[2], res[3]]);
        }
        let start = if loop_start >= 0 { loop_start as usize } else { cycles.len().saturating_sub(20) };
        let lp = &cycles[start..];
        if !lp.is_empty() {
            for k in 0..4 {
                res[k] = ((lp.iter().map(|v| v[k]).sum::<f64>() / lp.len() as f64) * 1000.0).round() / 1000.0;
            }
        }
    }
    let cat = fit.slots(db)[mo as usize].spec(db).category;
    for k in 0..4 {
        if !ctx.rah_disable {
            m.entry((mo, attrs[k])).or_default().push(AMod { op: 7, penalized: penalized(ds, attrs[k], cat), src: Src::Const(F(res[k])) });
        }
        m.entry((ship, attrs[k])).or_default().push(AMod { op: 0, penalized: penalized(ds, attrs[k], cat), src: Src::Const(F(res[k])) });
    }
    Arc::new(m)
}

pub fn has(db: &dyn Db, fit: FitIn, item: u32, attr: u32) -> bool {
    let it = fit.slots(db)[item as usize];
    it.spec(db).base(attr).is_some() || item_mods(db, fit, it).contains_key(&attr)
}

/// Evaluated attribute. Fast path: an attribute without modifiers, caps or rounding is its base value, so no
/// memoised query (and no interned key) is created for it; the caller then depends on `item_mods` + the spec.
#[inline]
pub fn value(db: &dyn Db, fit: FitIn, item: u32, attr: u32, layer: u32) -> f64 {
    let it = fit.slots(db)[item as usize];
    if !item_mods(db, fit, it).contains_key(&attr) && !db.consts().special(attr) {
        let mut plain = true;
        for l in 1..=layer {
            if layer_mods(db, fit, LKey::new(db, l)).contains_key(&(item, attr)) {
                plain = false;
                break;
            }
        }
        if plain {
            return it.spec(db).base(attr).unwrap_or_else(|| db.ds().attr_default(attr));
        }
    }
    attr_value(db, fit, AKey::new(db, item, attr, layer)).0
}

fn attr_cycle<'db>(db: &'db dyn Db, _id: salsa::Id, fit: FitIn, k: AKey<'db>) -> F {
    let it = fit.slots(db)[k.item(db) as usize];
    F(it.spec(db).base(k.attr(db)).unwrap_or_else(|| db.ds().attr_default(k.attr(db))))
}

fn src_value(db: &dyn Db, fit: FitIn, s: &Src, layer: u32) -> f64 {
    match *s {
        Src::Attr { item, attr } => value(db, fit, item, attr, layer),
        Src::Const(v) => v.0,
        Src::Prop { module, ship, speed, thrust, mass } => {
            let m = value(db, fit, ship, mass, layer);
            if m == 0.0 { 1.0 } else { 1.0 + value(db, fit, module, speed, layer) / 100.0 * value(db, fit, module, thrust, layer) / m }
        }
        Src::Projected { item, attr, factor, target, resist, mul } => {
            let mut f = factor.0;
            if resist != 0 {
                f *= value(db, fit, target, resist, layer);
            }
            let v = value(db, fit, item, attr, layer);
            if mul { (v - 1.0) * f + 1.0 } else { v * f }
        }
    }
}

#[salsa::tracked(returns(copy), cycle_result = attr_cycle)]
pub fn attr_value<'db>(db: &'db dyn Db, fit: FitIn, k: AKey<'db>) -> F {
    let (item, attr_id, layer) = (k.item(db), k.attr(db), k.layer(db));
    let ds = db.ds();
    let it = fit.slots(db)[item as usize];
    let base = it.spec(db).base(attr_id).unwrap_or_else(|| ds.attr_default(attr_id));
    let info = ds.attrs.get(&attr_id);
    let mut val = base;
    let mut vals: Vec<(i8, bool, f64)> = Vec::new();
    if let Some(ms) = item_mods(db, fit, it).get(&attr_id) {
        for m in ms {
            vals.push((m.op, m.penalized, src_value(db, fit, &m.src, layer)));
        }
    }
    for l in 1..=layer {
        if let Some(ms) = layer_mods(db, fit, LKey::new(db, l)).get(&(item, attr_id)) {
            for m in ms {
                vals.push((m.op, m.penalized, src_value(db, fit, &m.src, layer)));
            }
        }
    }
    if !vals.is_empty() {
        let mut pos: Vec<f64> = Vec::new();
        let mut neg: Vec<f64> = Vec::new();
        for op in [-1i8, 0, 1, 2, 3, 4, 5, 6, 7] {
            let mut any = false;
            pos.clear();
            neg.clear();
            let mut assign: Option<f64> = None;
            for &(o, pen, v) in &vals {
                if o != op {
                    continue;
                }
                any = true;
                match op {
                    -1 | 7 => {
                        let hig = info.map(|i| i.high_is_good).unwrap_or(true);
                        assign = Some(match assign {
                            None => v,
                            Some(c) => {
                                if hig { c.max(v) } else { c.min(v) }
                            }
                        });
                    }
                    2 => val += v,
                    3 => val -= v,
                    _ => {
                        let m = match op {
                            0 | 4 => v,
                            1 | 5 => {
                                if v == 0.0 { 1.0 } else { 1.0 / v }
                            }
                            6 => 1.0 + v / 100.0,
                            _ => 1.0,
                        };
                        if pen {
                            if m > 1.0 {
                                pos.push(m)
                            } else if m < 1.0 {
                                neg.push(m)
                            }
                        } else {
                            val *= m;
                        }
                    }
                }
            }
            if !any {
                continue;
            }
            if let Some(v) = assign {
                val = v;
            }
            for list in [&mut pos, &mut neg] {
                list.sort_by(|x, y| (y - 1.0).abs().partial_cmp(&(x - 1.0).abs()).unwrap_or(std::cmp::Ordering::Equal));
                for (i, m) in list.iter().enumerate() {
                    val *= 1.0 + (m - 1.0) * (-((i * i) as f64) / 7.1289).exp();
                }
            }
        }
    }
    if let Some(info) = info {
        if let Some(mn) = info.min_attr {
            val = val.max(value(db, fit, item, mn, layer));
        }
        if let Some(mx) = info.max_attr {
            val = val.min(value(db, fit, item, mx, layer));
        }
        if db.consts().rounded.contains(&attr_id) {
            val = crate::stats::py_round2(val);
        }
    }
    F(val)
}
