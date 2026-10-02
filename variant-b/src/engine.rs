//! Variant B dogma engine: **compile, then evaluate**.
//!
//! 1. *Build*: items are created from the request (flat `Vec<Item>`, base attributes = sorted patch
//!    vector over the type's sorted attribute slice — no per-item hash maps).
//! 2. *Register*: every effect modifier is expanded against pre-computed target index lists
//!    (ship-location items, owner items, character items) into a flat list of raw modifiers
//!    `(target item, attr, op, penalised, source)`.
//! 3. *Compile*: raw modifiers are stable-sorted by `(target item, attr, op)`; each distinct
//!    `(item, attr)` target becomes a dense **node**. Sources are resolved to either a node index or a
//!    constant (an attribute nobody modifies is folded to its base value at compile time). The result is
//!    a CSR graph: `mod_start[node]..mod_start[node+1]` indexes a flat `Vec<CMod>`.
//! 4. *Evaluate*: iterative DFS post-order over the node graph (explicit stack, no recursion), each
//!    node evaluated exactly once into a flat `Vec<f64>`. Nodes are evaluated on demand, so modifier
//!    nodes nobody reads (e.g. hundreds of skill self-bonuses) cost nothing.
//!
//! Effects that need evaluated values while registering (command-burst buff ids, the reactive armor
//! hardener) are handled by *staged compilation*: compile → evaluate the few needed nodes → append raw
//! modifiers → recompile.
use crate::data::{Dataset, Domain, Func, TypeInfo};
use crate::request::{FitRequest, ModuleReq, Slot, State};
use std::cell::{Cell, RefCell};

/// Source categories exempt from stacking penalties: Ship, Charge, Skill, Implant, Subsystem, Structure.
const EXEMPT_CATEGORIES: [u32; 6] = [6, 8, 16, 20, 32, 65];
/// requiredSkill1..6
pub const REQ_SKILL_ATTRS: [u32; 6] = [182, 183, 184, 1285, 1289, 1290];
pub const ATTR_SKILL_LEVEL: u32 = 280;
const EFFECT_SKILL_EFFECT: u32 = 132;
/// em/explosive/kinetic/thermal DamageResonance (hull)
const HULL_RESONANCES: [u32; 4] = [113, 111, 109, 110];
const STRUCTURE_SKILL_EFFECT_NAMES: [&str; 5] = [
    "targetingMaxTargetBonusModAddMaxLockedTargetsLocationChar",
    "skillStructureMissileDamageBonus",
    "skillStructureElectronicSystemsCapNeedBonus",
    "skillStructureEngineeringSystemsCapNeedBonus",
    "skillStructureDoomsdayDurationBonus",
];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loc {
    Ship,
    Char,
    Space,
    Nowhere,
}

/// Uncompiled modifier source (item/attr references).
#[derive(Debug, Clone, Copy)]
enum Src {
    Attr { item: u32, attr: u32 },
    Const(f64),
    /// AB/MWD: 1 + speedFactor/100 * speedBoostFactor / ship mass (PostMul)
    Prop { module: u32, ship: u32, speed: u32, thrust: u32, mass: u32 },
    /// projected: value scaled by range factor and target resistance attribute
    Projected { item: u32, attr: u32, factor: f64, target: u32, resist: u32, mul: bool },
}

#[derive(Debug, Clone, Copy)]
struct RawMod {
    item: u32,
    attr: u32,
    op: i8,
    penalized: bool,
    seq: u32,
    src: Src,
}

/// Compiled value reference: node or folded constant.
#[derive(Debug, Clone, Copy)]
enum Ref {
    Node(u32),
    Const(f64),
}

#[derive(Debug, Clone, Copy)]
enum CSrc {
    Val(Ref),
    Prop { speed: Ref, thrust: Ref, mass: Ref },
    Projected { v: Ref, factor: f64, resist: Option<Ref>, mul: bool },
}

#[derive(Debug, Clone, Copy)]
struct CMod {
    op: i8,
    penalized: bool,
    src: CSrc,
}

#[derive(Debug, Clone, Copy)]
struct NodeMeta {
    base: f64,
    high_is_good: bool,
    round2: bool,
    min: Option<Ref>,
    max: Option<Ref>,
}

#[derive(Debug)]
pub struct Item<'a> {
    pub type_id: u32,
    pub group: u32,
    pub category: u32,
    pub kind: Kind,
    pub state: State,
    pub loc: Loc,
    pub owned: bool,
    pub parent: Option<usize>,
    pub charge: Option<usize>,
    pub slot: Option<Slot>,
    pub req_index: Option<usize>,
    pub quantity: u32,
    pub active_count: u32,
    pub req_skills: Vec<u32>,
    pub effects: Vec<(u32, bool)>,
    pub fighter_abilities: Option<Vec<u32>>,
    pub booster_side_effects: Vec<u32>,
    pub spool: Option<crate::request::Spool>,
    pub distance: Option<f64>,
    /// sorted (attr, value) overriding `type_attrs`
    patch: Vec<(u32, f64)>,
    /// sorted base attributes of the type (empty for mutated items, whose attrs are fully in `patch`)
    type_attrs: &'a [(u32, f64)],
    /// sorted (attr, node) for attributes that receive modifiers
    nodes: Vec<(u32, u32)>,
}

impl<'a> Item<'a> {
    #[inline]
    fn base_opt(&self, attr: u32) -> Option<f64> {
        if let Ok(p) = self.patch.binary_search_by_key(&attr, |x| x.0) {
            return Some(self.patch[p].1);
        }
        self.type_attrs.binary_search_by_key(&attr, |x| x.0).ok().map(|p| self.type_attrs[p].1)
    }
    fn set_base(&mut self, attr: u32, v: f64) {
        match self.patch.binary_search_by_key(&attr, |x| x.0) {
            Ok(p) => self.patch[p].1 = v,
            Err(p) => self.patch.insert(p, (attr, v)),
        }
    }
    #[inline]
    fn node(&self, attr: u32) -> Option<u32> {
        self.nodes.binary_search_by_key(&attr, |x| x.0).ok().map(|p| self.nodes[p].1)
    }
}

#[derive(Default)]
struct Graph {
    meta: Vec<NodeMeta>,
    mod_start: Vec<u32>,
    mods: Vec<CMod>,
    /// 0 = not evaluated, 1 = on stack, 2 = done
    state: RefCell<Vec<u8>>,
    vals: RefCell<Vec<f64>>,
}

pub struct Fit<'a> {
    pub ds: &'a Dataset,
    pub(crate) prep: &'a Prepared,
    pub items: Vec<Item<'a>>,
    pub ship: usize,
    pub char: usize,
    pub warnings: Vec<String>,
    pub is_structure: bool,
    /// incoming remote reps / cap transfers / neuts from projected items, evaluated in stats
    pub proj_special: Vec<ProjSpecial>,
    raw: Vec<RawMod>,
    g: Graph,
    /// index lists for target selection
    idx_ship_loc: Vec<u32>,
    idx_owned: Vec<u32>,
    idx_char_loc: Vec<u32>,
    idx_char_skillable: Vec<u32>,
    /// (required skill, item) pairs, sorted: O(log n) LocationRequiredSkill / OwnerRequiredSkill lookups
    by_skill_ship: Vec<(u32, u32)>,
    by_skill_owned: Vec<(u32, u32)>,
    by_skill_char: Vec<(u32, u32)>,
    pub stats_evals: Cell<u64>,
    /// trained skill levels (type id, level 0..5), sorted; skills are folded, not instantiated
    pub skills: Vec<(u32, u8)>,
    folded: bool,
}

/// A projected effect that does not modify attributes but feeds tank or capacitor stats (Pyfa: fit._armorRr, addDrain).
#[derive(Debug, Clone, Copy)]
pub enum ProjSpecial {
    /// layer 0 shield, 1 armor, 2 hull; amount attr * mult * factor every `duration`
    Rep { item: usize, layer: u8, amount: u32, mult: f64, factor: f64 },
    /// capacitor drain (sign +1) or fill (sign -1) per cycle of `duration` attr
    Drain { item: usize, amount: u32, duration: u32, factor: f64, resist: u32, sign: f64 },
    /// ECM jam strength vs the target's strongest sensor type (Pyfa addProjectedEcm / jamChance)
    Ecm { item: usize, fighter: bool, factor: f64, resist: u32 },
}

/// Pyfa default: standard attack on; other abilities (except MWD/evasive/MJD) on only if they come before the
/// standard attack in effect order.
fn default_fighter_abilities(ds: &Dataset, effects: &[(u32, bool)]) -> Vec<u32> {
    let mut ids: Vec<u32> = effects.iter().map(|(e, _)| *e).collect();
    ids.sort();
    let mut on = Vec::new();
    let mut std_seen = false;
    for e in ids {
        let Some(n) = ds.effects.get(&e).map(|x| x.name.as_str()) else { continue };
        if !n.starts_with("fighterAbility") {
            continue;
        }
        if n == "fighterAbilityAttackM" {
            on.push(e);
            std_seen = true;
        } else if !std_seen && !matches!(n, "fighterAbilityMicroWarpDrive" | "fighterAbilityEvasiveManeuvers" | "fighterAbilityMicroJumpDrive") {
            on.push(e);
        }
    }
    on
}

#[derive(Debug)]
pub struct EngineError {
    pub code: &'static str,
    pub message: String,
    pub path: String,
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

impl<'a> Fit<'a> {
    fn new_item(&mut self, type_id: u32, kind: Kind, loc: Loc, path: &str) -> Result<usize, EngineError> {
        let ds = self.ds;
        let t = ds.types.get(&type_id).ok_or_else(|| EngineError {
            code: "UNKNOWN_TYPE",
            message: format!("unknown type_id {type_id}"),
            path: path.to_string(),
        })?;
        let mut item = Item {
            type_id,
            group: t.group,
            category: t.category,
            kind,
            state: State::Online,
            loc,
            owned: matches!(kind, Kind::Module | Kind::Charge | Kind::Drone | Kind::Fighter | Kind::Ship),
            parent: None,
            charge: None,
            slot: None,
            req_index: None,
            quantity: 1,
            active_count: 0,
            req_skills: REQ_SKILL_ATTRS.iter().filter_map(|a| t.attr(*a)).map(|v| v as u32).filter(|v| *v != 0).collect(),
            effects: t.effects.clone(),
            fighter_abilities: None,
            booster_side_effects: Vec::new(),
            spool: None,
            distance: None,
            patch: Vec::new(),
            type_attrs: t.attrs.as_slice(),
            nodes: Vec::new(),
        };
        // type-level fields are authoritative (mass/capacity/volume/radius)
        for (a, v) in [(4u32, t.mass), (38, t.capacity), (161, t.volume), (162, t.radius)] {
            if v != 0.0 || item.base_opt(a).is_none() {
                item.set_base(a, v);
            }
        }
        self.items.push(item);
        Ok(self.items.len() - 1)
    }

    fn apply_mutation(&mut self, idx: usize, m: &crate::request::Mutation) {
        let ds = self.ds;
        let tid = self.items[idx].type_id;
        if let Some(base) = ds.types.get(&m.base_type_id) {
            let own = self.items[idx].effects.clone();
            let item = &mut self.items[idx];
            // materialise: current attrs, then base attrs, then mutated type's own attrs on top
            let mut full: Vec<(u32, f64)> = item.type_attrs.to_vec();
            for &(a, v) in &item.patch {
                match full.binary_search_by_key(&a, |x| x.0) {
                    Ok(p) => full[p].1 = v,
                    Err(p) => full.insert(p, (a, v)),
                }
            }
            item.type_attrs = &[];
            item.patch = full;
            for (a, v) in &base.attrs {
                item.set_base(*a, *v);
            }
            for (a, v) in ds.types[&tid].attrs.iter() {
                item.set_base(*a, *v);
            }
            for (e, d) in &base.effects {
                if !own.iter().any(|(x, _)| x == e) {
                    item.effects.push((*e, *d));
                }
            }
            if item.req_skills.is_empty() {
                item.req_skills =
                    REQ_SKILL_ATTRS.iter().filter_map(|a| base.attr(*a)).map(|v| v as u32).filter(|v| *v != 0).collect();
            }
            if item.base_opt(4).unwrap_or(0.0) == 0.0 && base.mass != 0.0 {
                item.set_base(4, base.mass);
            }
        }
        let muta = m.mutaplasmid_type_id.and_then(|id| ds.mutaplasmids.get(&id));
        let base_t = ds.types.get(&m.base_type_id);
        for (k, v) in &m.attributes {
            let Ok(aid) = k.parse::<u32>() else { continue };
            let mut val = *v;
            if let (Some(mu), Some(bt)) = (muta, base_t) {
                if let (Some((lo, hi)), Some(bv)) = (mu.attrs.get(k), bt.attr(aid)) {
                    let (a, b) = (bv * lo, bv * hi);
                    let (mn, mx) = if a < b { (a, b) } else { (b, a) };
                    if bv != 0.0 {
                        val = val.clamp(mn, mx);
                    }
                }
            }
            self.items[idx].set_base(aid, val);
        }
    }

    fn add_module(&mut self, i: usize, m: &ModuleReq, path: &str) -> Result<usize, EngineError> {
        let idx = self.new_item(m.type_id, Kind::Module, Loc::Ship, path)?;
        let slot = m.slot.or_else(|| infer_slot(self.ds, &self.ds.types[&m.type_id]));
        let it = &mut self.items[idx];
        it.slot = slot;
        it.req_index = Some(i);
        it.spool = m.spool;
        it.state = m.state.unwrap_or(State::Online);
        if matches!(slot, Some(Slot::Rig) | Some(Slot::Subsystem)) && it.state != State::Offline {
            it.state = State::Online;
        }
        if let Some(mu) = &m.mutation {
            self.apply_mutation(idx, mu);
        }
        if let Some(c) = m.charge_type_id {
            let cidx = self.new_item(c, Kind::Charge, Loc::Ship, &format!("{path}/charge_type_id"))?;
            self.items[cidx].parent = Some(idx);
            self.items[cidx].req_index = Some(i);
            self.items[idx].charge = Some(cidx);
        }
        Ok(idx)
    }

    pub fn build(ds: &'a Dataset, req: &FitRequest) -> Result<Fit<'a>, EngineError> {
        let mut fit = Fit {
            ds,
            prep: ds.prepared.get_or_init(|| Prepared::new(ds)),
            items: Vec::with_capacity(ds.skills.len() + 64),
            ship: 0,
            char: 0,
            warnings: Vec::new(),
            is_structure: false,
            proj_special: Vec::new(),
            raw: Vec::with_capacity(4096),
            g: Graph::default(),
            idx_ship_loc: Vec::new(),
            idx_owned: Vec::new(),
            idx_char_loc: Vec::new(),
            idx_char_skillable: Vec::new(),
            by_skill_ship: Vec::new(),
            by_skill_owned: Vec::new(),
            by_skill_char: Vec::new(),
            stats_evals: Cell::new(0),
            skills: Vec::new(),
            folded: false,
        };
        prof_start();
        let ship = fit.new_item(req.ship.type_id, Kind::Ship, Loc::Ship, "/ship/type_id")?;
        fit.ship = ship;
        fit.is_structure = fit.items[ship].category == 65;
        let ch = fit.new_item(1373, Kind::Char, Loc::Char, "/character")?;
        fit.char = ch;
        if let Some(sec) = req.character.security_status {
            let a = ds.attr_id("pilotSecurityStatus");
            if a != 0 {
                fit.items[ch].set_base(a, sec);
            }
        }
        // skills: every published skill exists (untrained = level 0); ds.skills is sorted
        let default_level = req.character.skills.default_level.unwrap_or(0);
        let mut explicit: Vec<(u32, u8)> = Vec::new();
        for (k, v) in &req.character.skills.levels {
            let id = k.parse::<u32>().ok().or_else(|| ds.type_by_name(k));
            if let Some(id) = id {
                match explicit.iter_mut().find(|x| x.0 == id) {
                    Some(x) => x.1 = *v,
                    None => explicit.push((id, *v)),
                }
            }
        }
        let mut skill_ids: Vec<(u32, u8)> = fit.prep.published_skills.iter().map(|s| (*s, default_level)).collect();
        for (id, l) in explicit {
            match skill_ids.binary_search_by_key(&id, |x| x.0) {
                Ok(p) => skill_ids[p].1 = l,
                Err(p) => skill_ids.insert(p, (id, l)),
            }
        }
        skill_ids.retain(|(s, _)| ds.types.contains_key(s));
        for x in skill_ids.iter_mut() {
            x.1 = x.1.min(5);
        }
        let prep = fit.prep;
        // folding is exact unless something can modify skill attributes from outside the skill itself
        fit.folded = prep.skills_foldable && !req.overrides.iter().any(|o| ds.types.get(&o.type_id).map(|t| t.category == 16).unwrap_or(false));
        if !fit.folded {
            for &(s, l) in &skill_ids {
                let idx = fit.new_item(s, Kind::Skill, Loc::Char, "/character/skills")?;
                fit.items[idx].set_base(ATTR_SKILL_LEVEL, l as f64);
                fit.items[idx].owned = false;
            }
        }
        fit.skills = skill_ids;
        let mode_id = req.ship.mode_type_id.or_else(|| {
            let ship_name = ds.types.get(&req.ship.type_id)?.name.to_lowercase();
            let m = fit.prep.modes.iter().filter(|(n, _)| n.starts_with(&ship_name)).map(|(_, id)| *id).min()?;
            fit.warnings.push(format!("no tactical mode given; defaulted to type {m}"));
            Some(m)
        });
        if let Some(mode) = mode_id {
            let idx = fit.new_item(mode, Kind::Mode, Loc::Nowhere, "/ship/mode_type_id")?;
            fit.items[idx].owned = false;
        }
        for (i, m) in req.modules.iter().enumerate() {
            fit.add_module(i, m, &format!("/modules/{i}"))?;
        }
        for (i, d) in req.drones.iter().enumerate() {
            let idx = fit.new_item(d.type_id, Kind::Drone, Loc::Space, &format!("/drones/{i}"))?;
            if let Some(mu) = &d.mutation {
                fit.apply_mutation(idx, mu);
            }
            let it = &mut fit.items[idx];
            it.quantity = d.quantity.max(1);
            it.active_count = d.active.unwrap_or(0).min(it.quantity);
            it.state = if it.active_count > 0 { State::Active } else { State::Offline };
            it.req_index = Some(i);
        }
        let sq = ds.attr_id("fighterSquadronMaxSize");
        for (i, f) in req.fighters.iter().enumerate() {
            let idx = fit.new_item(f.type_id, Kind::Fighter, Loc::Space, &format!("/fighters/{i}"))?;
            let maxsq = fit.items[idx].base_opt(sq).map(|a| a as u32).unwrap_or(1);
            if f.quantity.unwrap_or(0) > maxsq {
                fit.warnings.push(format!("fighters/{i}: squadron size {} capped to {maxsq}", f.quantity.unwrap_or(0)));
            }
            let it = &mut fit.items[idx];
            it.quantity = f.quantity.unwrap_or(maxsq).clamp(1, maxsq.max(1));
            it.active_count = if f.active { it.quantity } else { 0 };
            it.state = if f.active { State::Active } else { State::Offline };
            it.fighter_abilities = f.abilities.clone().or_else(|| Some(default_fighter_abilities(ds, &it.effects)));
            it.req_index = Some(i);
        }
        for (i, imp) in req.implants.iter().enumerate() {
            let idx = fit.new_item(*imp, Kind::Implant, Loc::Char, &format!("/implants/{i}"))?;
            fit.items[idx].owned = false;
            fit.items[idx].req_index = Some(i);
        }
        for (i, b) in req.boosters.iter().enumerate() {
            let idx = fit.new_item(b.type_id, Kind::Booster, Loc::Char, &format!("/boosters/{i}"))?;
            fit.items[idx].owned = false;
            fit.items[idx].booster_side_effects = b.side_effects.clone();
            fit.items[idx].req_index = Some(i);
        }
        for (i, e) in req.environment.effect_type_ids.iter().enumerate() {
            let idx = fit.new_item(*e, Kind::Beacon, Loc::Nowhere, &format!("/environment/effect_type_ids/{i}"))?;
            fit.items[idx].owned = false;
        }
        for (i, p) in req.projected.iter().enumerate() {
            match p.kind.as_str() {
                "module" => {
                    if let Some(m) = &p.module {
                        for _ in 0..p.amount.max(1) {
                            let idx = fit.new_item(m.type_id, Kind::Projected, Loc::Nowhere, &format!("/projected/{i}"))?;
                            let it = &mut fit.items[idx];
                            it.owned = false;
                            it.state = m.state.unwrap_or(State::Active);
                            it.distance = p.distance_m;
                            it.req_index = Some(i);
                            if let Some(c) = m.charge_type_id {
                                let cidx = fit.new_item(c, Kind::Charge, Loc::Nowhere, &format!("/projected/{i}/module/charge_type_id"))?;
                                fit.items[cidx].parent = Some(idx);
                                fit.items[cidx].owned = false;
                                fit.items[idx].charge = Some(cidx);
                            }
                        }
                    }
                }
                "drone" => {
                    if let Some(d) = &p.drone {
                        for _ in 0..(p.amount.max(1) * d.quantity.max(1)) {
                            let idx = fit.new_item(d.type_id, Kind::Projected, Loc::Nowhere, &format!("/projected/{i}"))?;
                            let it = &mut fit.items[idx];
                            it.owned = false;
                            it.state = State::Active;
                            it.distance = p.distance_m;
                        }
                    }
                }
                "fighter" => {
                    if let Some(f) = &p.fighter {
                        for _ in 0..p.amount.max(1) {
                            let idx = fit.new_item(f.type_id, Kind::Projected, Loc::Nowhere, &format!("/projected/{i}"))?;
                            let maxsq = fit.items[idx].base_opt(sq).map(|a| a as u32).unwrap_or(1).max(1);
                            let it = &mut fit.items[idx];
                            it.owned = false;
                            it.state = if f.active { State::Active } else { State::Offline };
                            it.quantity = f.quantity.unwrap_or(maxsq).clamp(1, maxsq);
                            it.active_count = it.quantity;
                            it.distance = p.distance_m;
                            it.req_index = Some(i);
                            it.fighter_abilities = f.abilities.clone().or_else(|| Some(default_fighter_abilities(ds, &it.effects)));
                        }
                    }
                }
                "fit" => {
                    // whole projected fit: compute the source fit on its own, then project each active module /
                    // drone as a frozen item carrying the source-modified values as base attributes.
                    if let Some(src_req) = &p.fit {
                        let mut sreq = (**src_req).clone();
                        sreq.projected.clear();
                        let src = match Fit::build(ds, &sreq) {
                            Ok(f) => f,
                            Err(e) => {
                                fit.warnings.push(format!("projected[{i}] fit: {e:?}"));
                                continue;
                            }
                        };
                        let mut frozen: Vec<(u32, u32, Vec<(u32, f64)>, Kind, u32, Option<Vec<u32>>)> = Vec::new();
                        for (si, it) in src.items.iter().enumerate() {
                            let copies = match it.kind {
                                Kind::Module if it.state >= State::Active => 1,
                                Kind::Drone => it.active_count,
                                Kind::Fighter if it.state >= State::Active => 1,
                                _ => 0,
                            };
                            if copies == 0 {
                                continue;
                            }
                            let vals: Vec<(u32, f64)> = src.attr_keys(si).into_iter().map(|a| (a, src.get(si, a))).collect();
                            frozen.push((it.type_id, copies, vals, it.kind, it.quantity, it.fighter_abilities.clone()));
                        }
                        for (type_id, copies, vals, kind, qty, abil) in frozen {
                            for _ in 0..copies * p.amount.max(1) {
                                let idx = fit.new_item(type_id, Kind::Projected, Loc::Nowhere, &format!("/projected/{i}"))?;
                                let it = &mut fit.items[idx];
                                it.owned = false;
                                it.state = State::Active;
                                it.distance = p.distance_m;
                                it.req_index = Some(i);
                                if kind == Kind::Fighter {
                                    it.quantity = qty;
                                    it.active_count = qty;
                                    it.fighter_abilities = abil.clone();
                                }
                                // vals is sorted and covers every attribute the item has
                                let mut full = vals.clone();
                                for &(a, v) in it.patch.iter() {
                                    if let Err(pos) = full.binary_search_by_key(&a, |x| x.0) {
                                        full.insert(pos, (a, v));
                                    }
                                }
                                it.patch = full;
                                it.type_attrs = &[];
                            }
                        }
                    }
                }
                other => fit.warnings.push(format!("projected kind '{other}' not supported yet (index {i})")),
            }
        }
        {
            let sec = req.environment.system_security.as_deref().unwrap_or("nullsec").to_lowercase();
            let src = match sec.as_str() {
                "hisec" | "highsec" | "high" => "hiSecModifier",
                "lowsec" | "low" => "lowSecModifier",
                "nullsec" | "null" | "wspace" | "wormhole" | "w-space" => "nullSecModifier",
                other => {
                    fit.warnings.push(format!("unknown system_security '{other}', using nullsec"));
                    "nullSecModifier"
                }
            };
            let (src_id, dst_id) = (ds.attr_id(src), ds.attr_id("securityModifier"));
            for it in fit.items.iter_mut() {
                if let Some(v) = it.base_opt(src_id) {
                    it.set_base(dst_id, v);
                }
            }
        }
        for o in &req.overrides {
            for it in fit.items.iter_mut().filter(|it| it.type_id == o.type_id) {
                it.set_base(o.attribute_id, o.value);
            }
        }
        prof(0);
        fit.build_indexes();
        fit.register_all(req);
        prof(1);
        fit.compile();
        prof(2);
        // stage 2: command bursts need evaluated buff ids
        if fit.register_local_bursts(req) {
            fit.compile();
        }
        fit.apply_rah(req);
        prof(3);
        Ok(fit)
    }

    fn build_indexes(&mut self) {
        for (i, it) in self.items.iter().enumerate() {
            let i = i as u32;
            if it.loc == Loc::Ship {
                self.idx_ship_loc.push(i);
            }
            if it.owned {
                self.idx_owned.push(i);
            }
            if it.loc == Loc::Char {
                self.idx_char_loc.push(i);
            }
            if (it.owned || it.loc == Loc::Char) && it.kind != Kind::Skill {
                self.idx_char_skillable.push(i);
            }
        }
        let pairs = |items: &Vec<Item>, list: &Vec<u32>| -> Vec<(u32, u32)> {
            let mut v: Vec<(u32, u32)> = list.iter().flat_map(|&i| items[i as usize].req_skills.iter().map(move |&s| (s, i))).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        self.by_skill_ship = pairs(&self.items, &self.idx_ship_loc);
        self.by_skill_owned = pairs(&self.items, &self.idx_owned);
        self.by_skill_char = pairs(&self.items, &self.idx_char_skillable);
    }

    // ---------------------------------------------------------------- registration
    #[inline]
    fn push_mod(&mut self, target: usize, attr: u32, op: i32, src: Src, source_cat: u32) {
        let stackable = self.prep.meta(attr).stackable;
        let penalized = !stackable && !EXEMPT_CATEGORIES.contains(&source_cat);
        let seq = self.raw.len() as u32;
        self.raw.push(RawMod { item: target as u32, attr, op: op as i8, penalized, seq, src });
    }

    /// Target selection over pre-built index lists (same semantics as eve-dogma-rs `targets`).
    fn for_targets(&self, src: usize, func: Func, domain: Domain, extra: u32, out: &mut Vec<u32>) {
        out.clear();
        let items = &self.items;
        let s = &items[src];
        match domain {
            Domain::Item => {
                if func == Func::Item {
                    out.push(src as u32)
                }
            }
            Domain::Other => {
                if let Some(c) = s.charge {
                    out.push(c as u32)
                } else if let Some(p) = s.parent {
                    out.push(p as u32)
                }
            }
            Domain::Ship | Domain::Structure => {
                if domain == Domain::Structure && !self.is_structure {
                    return;
                }
                match func {
                    Func::Item => out.push(self.ship as u32),
                    Func::Location => out.extend_from_slice(&self.idx_ship_loc),
                    Func::LocationGroup => {
                        out.extend(self.idx_ship_loc.iter().copied().filter(|&i| items[i as usize].group == extra))
                    }
                    Func::LocationRequiredSkill => skill_range(&self.by_skill_ship, extra, out),
                    Func::OwnerRequiredSkill => skill_range(&self.by_skill_owned, extra, out),
                    Func::EffectStopper => {}
                }
            }
            Domain::Char => match func {
                Func::Item => out.push(self.char as u32),
                Func::Location => out.extend_from_slice(&self.idx_char_loc),
                Func::LocationGroup => {
                    out.extend(self.idx_char_loc.iter().copied().filter(|&i| items[i as usize].group == extra))
                }
                Func::LocationRequiredSkill | Func::OwnerRequiredSkill => skill_range(&self.by_skill_char, extra, out),
                Func::EffectStopper => {}
            },
            _ => {}
        }
    }

    fn effective_state(&self, i: usize) -> State {
        let it = &self.items[i];
        match it.kind {
            Kind::Charge => it.parent.map(|p| self.items[p].state).unwrap_or(State::Online),
            Kind::Ship | Kind::Char | Kind::Skill | Kind::Implant | Kind::Booster | Kind::Mode | Kind::Beacon => State::Online,
            Kind::Drone | Kind::Fighter => {
                if it.active_count > 0 {
                    State::Active
                } else {
                    State::Offline
                }
            }
            _ => it.state,
        }
    }

    fn register_all(&mut self, req: &FitRequest) {
        let ds = self.ds;
        let n = self.items.len();
        let e_ab = ds.effect_id("moduleBonusAfterburner");
        let e_mwd = ds.effect_id("moduleBonusMicrowarpdrive");
        let e_slot = ds.effect_id("slotModifier");
        let e_hp = ds.effect_id("hardPointModifierEffect");
        let e_mjd = ds.effect_id("microJumpDrive");
        let e_bastion = ds.effect_id("moduleBonusBastionModule");
        let is_structure = self.is_structure;
        let structure_ok: Vec<u32> = STRUCTURE_SKILL_EFFECT_NAMES.iter().map(|n| ds.effect_id(n)).collect();
        let a_mass_add = ds.attr_id("massAddition");
        let a_speed_factor = ds.attr_id("speedFactor");
        let a_thrust = ds.attr_id("speedBoostFactor");
        let a_maxv = ds.attr_id("maxVelocity");
        let a_sig = ds.attr_id("signatureRadius");
        let a_sigb = ds.attr_id("signatureRadiusBonus");
        let a_sigbp = ds.attr_id("signatureRadiusBonusPercent");
        let slot_pairs: Vec<(u32, u32)> = [("hiSlots", "hiSlotModifier"), ("medSlots", "medSlotModifier"), ("lowSlots", "lowSlotModifier")]
            .iter()
            .map(|(t, s)| (ds.attr_id(t), ds.attr_id(s)))
            .collect();
        let hp_pairs: Vec<(u32, u32)> =
            [("turretSlotsLeft", "turretHardPointModifier"), ("launcherSlotsLeft", "launcherHardPointModifier")]
                .iter()
                .map(|(t, s)| (ds.attr_id(t), ds.attr_id(s)))
                .collect();
        let mut targets: Vec<u32> = Vec::with_capacity(64);
        let mut skills_done = false;
        for i in 0..n {
            if i == self.char + 1 && self.folded {
                self.register_folded_skills(&mut targets);
                skills_done = true;
            }
            let kind = self.items[i].kind;
            if kind == Kind::Projected {
                self.register_projected(i);
                continue;
            }
            if is_structure && matches!(kind, Kind::Drone | Kind::Implant | Kind::Booster) {
                continue;
            }
            let state = self.effective_state(i);
            let src_cat = self.items[i].category;
            let ne = self.items[i].effects.len();
            for k in 0..ne {
                let (eid, is_default) = self.items[i].effects[k];
                if eid == EFFECT_SKILL_EFFECT {
                    continue;
                }
                let Some(e) = ds.effects.get(&eid) else { continue };
                if is_structure && kind == Kind::Skill && !structure_ok.contains(&eid) && !e.mods.iter().all(|m| m.domain == Domain::Item) {
                    continue;
                }
                if e.fitting_usage_chance_attr.is_some() && !self.items[i].booster_side_effects.contains(&eid) {
                    continue;
                }
                if kind == Kind::Fighter && e.category != 0 {
                    let used = match &self.items[i].fighter_abilities {
                        Some(a) => a.contains(&eid),
                        None => is_default,
                    };
                    if !used {
                        continue;
                    }
                }
                // Pyfa 'active' handlers for SDE effects without modifiers (some are target-category in the SDE)
                if e.mods.is_empty() && kind == Kind::Module && state >= State::Active && self.local_special(i, e.name.as_str(), src_cat) {
                    continue;
                }
                if kind == Kind::Beacon && e.name == "OffensiveDefensiveReduction" {
                    self.incursion_effect(i);
                    continue;
                }
                if !state_ok(e.category, state) {
                    continue;
                }
                let ship = self.ship;
                let iu = i as u32;
                if kind == Kind::Fighter && e.mods.is_empty() {
                    // fighter self abilities (Pyfa hand-written handlers, eos LGPL)
                    let fm: &[(&str, &str, i32)] = match e.name.as_str() {
                        "fighterAbilityMicroWarpDrive" => &[
                            ("maxVelocity", "fighterAbilityMicroWarpDriveSpeedBonus", 6),
                            ("signatureRadius", "fighterAbilityMicroWarpDriveSignatureRadiusBonus", 6),
                        ],
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
                        for &(t, a, op) in fm {
                            self.push_mod(i, ds.attr_id(t), op, Src::Attr { item: iu, attr: ds.attr_id(a) }, src_cat);
                        }
                        continue;
                    }
                }
                if eid == e_ab || eid == e_mwd {
                    self.push_mod(ship, 4, 2, Src::Attr { item: iu, attr: a_mass_add }, src_cat);
                    let src = Src::Prop { module: iu, ship: ship as u32, speed: a_speed_factor, thrust: a_thrust, mass: 4 };
                    self.push_mod(ship, a_maxv, 4, src, src_cat);
                    if eid == e_mwd {
                        self.push_mod(ship, a_sig, 6, Src::Attr { item: iu, attr: a_sigb }, src_cat);
                    }
                    continue;
                }
                if eid == e_mjd {
                    self.push_mod(ship, a_sig, 6, Src::Attr { item: iu, attr: a_sigbp }, 6);
                    continue;
                }
                if eid == e_slot {
                    for &(t, s) in &slot_pairs {
                        self.push_mod(ship, t, 2, Src::Attr { item: iu, attr: s }, src_cat);
                    }
                    continue;
                }
                if eid == e_hp {
                    for &(t, s) in &hp_pairs {
                        self.push_mod(ship, t, 2, Src::Attr { item: iu, attr: s }, src_cat);
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
                    let extra = if m.extra == 0 && matches!(m.func, Func::LocationRequiredSkill | Func::OwnerRequiredSkill) {
                        self.items[i].type_id
                    } else {
                        m.extra
                    };
                    self.for_targets(i, m.func, m.domain, extra, &mut targets);
                    let cat = if eid == e_bastion && HULL_RESONANCES.contains(&m.modified) { 6 } else { src_cat };
                    for &t in &targets {
                        self.push_mod(t as usize, m.modified, m.op, Src::Attr { item: iu, attr: m.modifying }, cat);
                    }
                }
            }
        }
        if self.folded && !skills_done {
            self.register_folded_skills(&mut targets);
        }
        let _ = req;
    }

    /// Skills as constant modifier sources: values of the skill's own attributes at the trained level
    /// come from the dataset-level fold table (same registration order as instantiated skills).
    fn register_folded_skills(&mut self, targets: &mut Vec<u32>) {
        let ds = self.ds;
        let prep = ds.prepared.get().expect("prepared");
        let skills = std::mem::take(&mut self.skills);
        // skill ids that some fitted item requires (only these can be reached by *RequiredSkill filters)
        let mut required: Vec<u32> =
            self.by_skill_ship.iter().chain(self.by_skill_owned.iter()).chain(self.by_skill_char.iter()).map(|x| x.0).collect();
        required.sort_unstable();
        required.dedup();
        for &(s, l) in &skills {
            let Some(f) = prep.fold(ds, s) else { continue };
            if !f.unconditional && !f.extras.iter().any(|e| required.binary_search(e).is_ok()) {
                continue; // no modifier of this skill can reach anything in this fit
            }
            let vals = f.values(ds, s, l, self.is_structure);
            for (k, m) in f.outgoing.iter().enumerate() {
                if self.is_structure && !m.structure_ok {
                    continue;
                }
                self.for_targets(self.char, m.func, m.domain, m.extra, targets);
                for &t in targets.iter() {
                    self.push_mod(t as usize, m.modified, m.op, Src::Const(vals[k]), f.category);
                }
            }
        }
        self.skills = skills;
    }

    /// Local module effects that have no modifierInfo in the SDE but a hand-written Pyfa handler (eos/effects.py,
    /// LGPL; re-expressed here). Returns true when the effect was handled. Category 6 as the source category marks
    /// a boost Pyfa applies without stacking penalty.
    fn local_special(&mut self, i: usize, name: &str, src_cat: u32) -> bool {
        let ds = self.ds;
        let ship = self.ship;
        let a = |n: &str| ds.attr_id(n);
        let iu = i as u32;
        match name {
            "superWeaponAmarr" | "superWeaponCaldari" | "superWeaponGallente" | "superWeaponMinmatar" | "doomsdaySlash"
            | "doomsdayBeamDOT" | "doomsdayConeDOT" | "doomsdayHOG" | "debuffLance" => {
                self.push_mod(ship, a("maxVelocity"), 6, Src::Attr { item: iu, attr: a("speedFactor") }, src_cat);
                self.push_mod(ship, a("warpScrambleStatus"), 2, Src::Attr { item: iu, attr: a("siegeModeWarpStatus") }, src_cat);
            }
            "emergencyHullEnergizer" => {
                for t in ["Em", "Thermal", "Kinetic", "Explosive"] {
                    let tgt = a(&format!("{}DamageResonance", t.to_lowercase()));
                    self.push_mod(ship, tgt, 4, Src::Attr { item: iu, attr: a(&format!("hull{t}DamageResonance")) }, src_cat);
                }
            }
            "entosisLink" => {
                self.push_mod(ship, a("disallowAssistance"), 7, Src::Attr { item: iu, attr: a("disallowAssistance") }, 6);
                for t in ["Gravimetric", "Magnetometric", "Radar", "Ladar"] {
                    self.push_mod(ship, a(&format!("scan{t}Strength")), 6, Src::Attr { item: iu, attr: a(&format!("scan{t}StrengthPercent")) }, src_cat);
                }
            }
            "moduleBonusBreacherPodDamageControl" => {
                self.push_mod(ship, a("breacherPodDamageResistance"), 6, Src::Attr { item: iu, attr: a("breacherPodActivatedDamageReceivedPercentage") }, 6);
            }
            "microJumpPortalDrive" | "microJumpPortalDriveCapital" => {
                self.push_mod(ship, a("signatureRadius"), 6, Src::Attr { item: iu, attr: a("signatureRadiusBonusPercent") }, src_cat);
            }
            "warpDisruptSphere" => {
                self.push_mod(ship, a("disallowAssistance"), 7, Src::Const(1.0), 6);
                if self.items[i].charge.is_none() {
                    self.push_mod(ship, 4, 6, Src::Attr { item: iu, attr: a("massBonusPercentage") }, 6);
                    self.push_mod(ship, a("signatureRadius"), 6, Src::Attr { item: iu, attr: a("signatureRadiusBonus") }, 6);
                    let props: Vec<usize> = (0..self.items.len())
                        .filter(|&t| {
                            let it = &self.items[t];
                            it.kind == Kind::Module && it.loc == Loc::Ship && ds.groups.get(&it.group).map(|g| g.name == "Propulsion Module").unwrap_or(false)
                        })
                        .collect();
                    for t in props {
                        self.push_mod(t, a("speedBoostFactor"), 6, Src::Attr { item: iu, attr: a("speedBoostFactorBonus") }, 6);
                        self.push_mod(t, a("speedFactor"), 6, Src::Attr { item: iu, attr: a("speedFactorBonus") }, 6);
                    }
                }
            }
            _ => return false,
        }
        true
    }

    /// Sansha / Drifter incursion system effects (Pyfa Effect4728 OffensiveDefensiveReduction, LGPL; re-expressed):
    /// unpenalised PostPercent of missile-charge and smartbomb damage, turret and drone damageMultiplier by
    /// systemEffectDamageReduction, and of the ship's armor/shield resonances by the beacon's resistance bonuses.
    fn incursion_effect(&mut self, b: usize) {
        let ds = self.ds;
        let a = |n: &str| ds.attr_id(n);
        let ship = self.ship;
        let bu = b as u32;
        let red = a("systemEffectDamageReduction");
        let mls = ds.type_by_name("Missile Launcher Operation").unwrap_or(0);
        let gunnery = ds.type_by_name("Gunnery").unwrap_or(0);
        let smartbomb = ds.groups.iter().find(|(_, g)| g.name == "Smart Bomb").map(|(k, _)| k).unwrap_or(0);
        let n = self.items.len();
        for t in 0..n {
            let it = &self.items[t];
            if !it.owned || it.loc != Loc::Ship && it.kind != Kind::Drone {
                continue;
            }
            let mut dmg = false;
            let mut mult = false;
            match it.kind {
                Kind::Charge => dmg = it.req_skills.contains(&mls),
                Kind::Module => {
                    dmg = it.group == smartbomb;
                    mult = it.req_skills.contains(&gunnery);
                }
                Kind::Drone => mult = true,
                _ => {}
            }
            if dmg {
                for d in ["em", "thermal", "kinetic", "explosive"] {
                    self.push_mod(t, a(&format!("{d}Damage")), 6, Src::Attr { item: bu, attr: red }, 6);
                }
            }
            if mult {
                self.push_mod(t, a("damageMultiplier"), 6, Src::Attr { item: bu, attr: red }, 6);
            }
        }
        for d in ["Em", "Thermal", "Kinetic", "Explosive"] {
            for l in ["armor", "shield"] {
                self.push_mod(ship, a(&format!("{l}{d}DamageResonance")), 6, Src::Attr { item: bu, attr: a(&format!("{l}{d}DamageResistanceBonus")) }, 6);
            }
        }
    }

    /// Pyfa's 'projected' handlers for remote reps, cap transfers and neuts/nos (eos/effects.py, LGPL).
    fn proj_special_for(&self, i: usize, name: &str, resist: u32) -> Option<Vec<ProjSpecial>> {
        let ds = self.ds;
        let a = |n: &str| ds.attr_id(n);
        let it = &self.items[i];
        let base = |n: &str| it.base_opt(ds.attr_id(n)).unwrap_or(0.0);
        let dist = it.distance;
        let falloff_factor = || crate::stats::range_factor(base("maxRange"), base("falloffEffectiveness"), dist, true);
        let gate = |opt: f64| if opt < dist.unwrap_or(0.0) { 0.0 } else { 1.0 };
        let no_assist = self.items[self.ship].base_opt(a("disallowAssistance")).map(|x| x != 0.0).unwrap_or(false);
        let rep = |layer: u8, amt: &str, mult: f64, factor: f64| {
            if no_assist { vec![] } else { vec![ProjSpecial::Rep { item: i, layer, amount: a(amt), mult, factor }] }
        };
        let drain = |amt: &str, dur: &str, factor: f64, sign: f64| vec![ProjSpecial::Drain { item: i, amount: a(amt), duration: a(dur), factor, resist, sign }];
        let no_offense = self.items[self.ship].base_opt(a("disallowOffensiveModifiers")).map(|x| x != 0.0).unwrap_or(false);
        let ecm = |fighter: bool, factor: f64| if no_offense { vec![] } else { vec![ProjSpecial::Ecm { item: i, fighter, factor, resist }] };
        let paste = it.charge.map(|c| ds.types.get(&self.items[c].type_id).map(|t| t.name == "Nanite Repair Paste").unwrap_or(false)).unwrap_or(false);
        Some(match name {
            "shipModuleRemoteShieldBooster" | "shipModuleAncillaryRemoteShieldBooster" => rep(0, "shieldBonus", 1.0, falloff_factor()),
            "shipModuleRemoteArmorRepairer" | "ShipModuleRemoteArmorMutadaptiveRepairer" => rep(1, "armorDamageAmount", 1.0, falloff_factor()),
            "shipModuleAncillaryRemoteArmorRepairer" => rep(1, "armorDamageAmount", if paste { 3.0 } else { 1.0 }, falloff_factor()),
            "shipModuleRemoteHullRepairer" => rep(2, "structureDamageAmount", 1.0, falloff_factor()),
            "npcEntityRemoteShieldBooster" => rep(0, "shieldBonus", 1.0, gate(base("maxRange"))),
            "npcEntityRemoteArmorRepairer" => rep(1, "armorDamageAmount", 1.0, gate(base("maxRange"))),
            "npcEntityRemoteHullRepairer" => rep(2, "structureDamageAmount", 1.0, gate(base("maxRange"))),
            "shipModuleRemoteCapacitorTransmitter" => {
                if no_assist { vec![] } else { drain("powerTransferAmount", "duration", gate(base("maxRange")), -1.0) }
            }
            "energyNeutralizerFalloff" => drain("energyNeutralizerAmount", "duration", falloff_factor(), 1.0),
            "fighterAbilityEnergyNeutralizer" => {
                let f = crate::stats::range_factor(base("fighterAbilityEnergyNeutralizerOptimalRange"), base("fighterAbilityEnergyNeutralizerFalloffRange"), dist, true);
                drain("fighterAbilityEnergyNeutralizerAmount", "fighterAbilityEnergyNeutralizerDuration", f * it.quantity.max(1) as f64, 1.0)
            }
            "remoteECMFalloff" | "structureModuleEffectECM" => ecm(false, falloff_factor()),
            "entityECMFalloff" => ecm(false, gate(base("ECMRangeOptimal"))),
            "ECMBurstJammer" => ecm(false, gate(base("ecmBurstRange"))),
            "fighterAbilityECM" => {
                let f = crate::stats::range_factor(base("fighterAbilityECMRangeOptimal"), base("fighterAbilityECMRangeFalloff"), dist, true);
                ecm(true, f * it.quantity.max(1) as f64)
            }
            "energyNosferatuFalloff" => drain("powerTransferAmount", "duration", falloff_factor(), 1.0),
            "structureEnergyNeutralizerFalloff" => drain("energyNeutralizerAmount", "duration", 1.0, 1.0),
            "entityEnergyNeutralizerFalloff" => {
                drain("energyNeutralizerAmount", "energyNeutralizerDuration", gate(base("energyNeutralizerRangeOptimal")), 1.0)
            }
            _ => return None,
        })
    }

    fn register_projected(&mut self, i: usize) {
        const DAMAGE_EFFECTS: &[&str] = &["projectileFired", "targetAttack", "useMissiles", "barrage", "targetDisintegratorAttack",
            "missileLaunchingForEntity", "fighterAbilityAttackM", "fighterAbilityMissiles", "superWeaponAmarr", "superWeaponCaldari",
            "superWeaponGallente", "superWeaponMinmatar", "mining", "miningLaser", "miningClouds", "dotMissileLaunching", "ChainLightning", "salvageDroneEffect"];
        let ds = self.ds;
        let src_cat = self.items[i].category;
        let state = self.items[i].state;
        let effects = self.items[i].effects.clone();
        let ship = self.ship;
        let abilities = self.items[i].fighter_abilities.clone();
        let qty = self.items[i].quantity.max(1) as f64;
        for (eid, _) in effects {
            let Some(e) = ds.effects.get(&eid) else { continue };
            if e.category != 2 && e.category != 3 && e.name != "ECMBurstJammer" && !e.name.starts_with("doomsdayAOE") {
                continue;
            }
            if let Some(ab) = &abilities {
                if e.name.starts_with("fighterAbility") && !ab.contains(&eid) {
                    continue;
                }
            }
            if state < State::Active {
                continue;
            }
            let factor = {
                let it = &self.items[i];
                let opt = e.range_attr.and_then(|a| it.base_opt(a)).unwrap_or(0.0);
                let fo = e.falloff_attr.and_then(|a| it.base_opt(a)).unwrap_or(0.0);
                crate::stats::range_factor(opt, fo, it.distance, true)
            };
            let resist = e.resistance_attr.unwrap_or_else(|| {
                let it = &self.items[i];
                let look = |n: &str| it.base_opt(ds.attr_id(n)).map(|a| a as u32).unwrap_or(0);
                if e.name.starts_with("fighterAbility") {
                    let r = look(&format!("{}ResistanceID", e.name));
                    if r != 0 { r } else { look(&format!("{}RemoteResistanceID", e.name)) }
                } else {
                    look("remoteResistanceID")
                }
            });
            let target_offense_ok = self.items[ship].base_opt(ds.attr_id("disallowOffensiveModifiers")).map(|a| a == 0.0).unwrap_or(true);
            let push = |fit: &mut Fit<'a>, target_attr: u32, src_attr: u32, op: i32| {
                let mul = op == 4 || op == 0;
                fit.push_mod(
                    ship,
                    target_attr,
                    op,
                    Src::Projected { item: i as u32, attr: src_attr, factor, target: ship as u32, resist, mul },
                    src_cat,
                );
            };
            // burst projectors and the Standup weapon disruptor stay engine-side even if a dataset revision gives
            // them modifiers: the generic path has no AoE full-strength rule
            let engine_side = e.name.starts_with("doomsdayAOE") || e.name == "structureModuleEffectWeaponDisruption";
            if !e.mods.is_empty() && !engine_side {
                for m in &e.mods {
                    if matches!(m.domain, Domain::TargetId | Domain::Target | Domain::Ship) && m.func == Func::Item {
                        push(self, m.modified, m.modifying, m.op);
                    }
                }
                continue;
            }
            let name = e.name.as_str();
            let pbase = |n: &str| self.items[i].base_opt(ds.attr_id(n)).unwrap_or(0.0);
            if name == "fighterAbilityStasisWebifier" {
                if target_offense_ok {
                    let f = crate::stats::range_factor(pbase("fighterAbilityStasisWebifierOptimalRange"), pbase("fighterAbilityStasisWebifierFalloffRange"), self.items[i].distance, true) * qty;
                    let src = Src::Projected { item: i as u32, attr: ds.attr_id("fighterAbilityStasisWebifierSpeedPenalty"), factor: f, target: ship as u32, resist, mul: false };
                    self.push_mod(ship, ds.attr_id("maxVelocity"), 6, src, src_cat);
                }
                continue;
            }
            if name == "fighterAbilityWarpDisruption" {
                if target_offense_ok && pbase("fighterAbilityWarpDisruptionRange") >= self.items[i].distance.unwrap_or(0.0) {
                    let src = Src::Projected { item: i as u32, attr: ds.attr_id("fighterAbilityWarpDisruptionPointStrength"), factor: qty, target: ship as u32, resist, mul: false };
                    self.push_mod(ship, ds.attr_id("warpScrambleStatus"), 2, src, src_cat);
                }
                continue;
            }
            // burst projectors (Pyfa Effect6476-6482/6513): full strength on every ship in the AoE (no range factor)
            let full = |fit: &mut Fit<'a>, t: usize, tgt: u32, sa: u32| {
                fit.push_mod(t, tgt, 6, Src::Projected { item: i as u32, attr: sa, factor: 1.0, target: ship as u32, resist, mul: false }, src_cat);
            };
            match name {
                "doomsdayAOEWeb" | "doomsdayAOEPaint" | "doomsdayAOEDamp" => {
                    if target_offense_ok {
                        let pairs: &[(&str, &str)] = match name {
                            "doomsdayAOEWeb" => &[("maxVelocity", "speedFactor")],
                            "doomsdayAOEPaint" => &[("signatureRadius", "signatureRadiusBonus")],
                            _ => &[("maxTargetRange", "maxTargetRangeBonus"), ("scanResolution", "scanResolutionBonus")],
                        };
                        for (t, sa) in pairs {
                            full(self, ship, ds.attr_id(t), ds.attr_id(sa));
                        }
                    }
                    continue;
                }
                "doomsdayAOENeut" => {
                    self.proj_special.push(ProjSpecial::Drain { item: i, amount: ds.attr_id("energyNeutralizerAmount"), duration: ds.attr_id("duration"), factor: 1.0, resist, sign: 1.0 });
                    continue;
                }
                "doomsdayAOEECM" => {
                    if target_offense_ok {
                        self.proj_special.push(ProjSpecial::Ecm { item: i, fighter: false, factor: 1.0, resist });
                    }
                    continue;
                }
                "doomsdayAOEBubble" | "doomsdayAOEGuide" => continue,
                _ => {}
            }
            let weapon_disruption = name == "doomsdayAOETrack" || name == "structureModuleEffectWeaponDisruption";
            if name.starts_with("remoteWebifier") || name == "structureModuleEffectStasisWebifier" {
                push(self, ds.attr_id("maxVelocity"), ds.attr_id("speedFactor"), 6);
            } else if name.starts_with("remoteTargetPaint") || name == "structureModuleEffectTargetPainter" {
                push(self, ds.attr_id("signatureRadius"), ds.attr_id("signatureRadiusBonus"), 6);
            } else if name.starts_with("remoteSensorDamp")
                || name == "structureModuleEffectRemoteSensorDampener"
                || name.starts_with("remoteSensorBoost")
            {
                push(self, ds.attr_id("maxTargetRange"), ds.attr_id("maxTargetRangeBonus"), 6);
                push(self, ds.attr_id("scanResolution"), ds.attr_id("scanResolutionBonus"), 6);
                if name.starts_with("remoteSensorBoost") {
                    for t in ["Gravimetric", "Ladar", "Magnetometric", "Radar"] {
                        push(self, ds.attr_id(&format!("scan{t}Strength")), ds.attr_id(&format!("scan{t}StrengthPercent")), 6);
                    }
                }
            } else if weapon_disruption {
                // AoE weapon disruption burst (full strength) / Standup Weapon Disruptor (range factor): turrets and missiles
                if target_offense_ok {
                    let tf = if name == "doomsdayAOETrack" {
                        1.0
                    } else {
                        let it = &self.items[i];
                        crate::stats::range_factor(it.base_opt(ds.attr_id("maxRange")).unwrap_or(0.0), it.base_opt(ds.attr_id("falloffEffectiveness")).unwrap_or(0.0), it.distance, true)
                    };
                    let (gun, mls) = (ds.type_by_name("Gunnery").unwrap_or(0), ds.type_by_name("Missile Launcher Operation").unwrap_or(0));
                    let n = self.items.len();
                    for t in 0..n {
                        let it = &self.items[t];
                        if it.loc != Loc::Ship || !it.owned {
                            continue;
                        }
                        let pairs: &[(&str, &str)] = if it.kind == Kind::Module && it.req_skills.contains(&gun) {
                            &[("trackingSpeedBonus", "trackingSpeed"), ("maxRangeBonus", "maxRange"), ("falloffBonus", "falloff")]
                        } else if it.kind == Kind::Charge && it.req_skills.contains(&mls) {
                            &[("aoeCloudSizeBonus", "aoeCloudSize"), ("aoeVelocityBonus", "aoeVelocity"), ("missileVelocityBonus", "maxVelocity"), ("explosionDelayBonus", "explosionDelay")]
                        } else {
                            continue;
                        };
                        for (sa, ta) in pairs {
                            self.push_mod(t, ds.attr_id(ta), 6, Src::Projected { item: i as u32, attr: ds.attr_id(sa), factor: tf, target: ship as u32, resist, mul: false }, src_cat);
                        }
                    }
                }
            } else if name == "shipModuleTrackingDisruptor" || name == "shipModuleGuidanceDisruptor" || name == "shipModuleRemoteTrackingComputer" || name == "npcEntityWeaponDisruptor" {
                // Pyfa Effect6424 / Effect6423 / shipModuleRemoteTrackingComputer: boost the target's gunnery modules
                // (TD, remote tracking computer) / missile charges (GD)
                let allowed = if name == "shipModuleRemoteTrackingComputer" {
                    self.items[ship].base_opt(ds.attr_id("disallowAssistance")).map(|a| a == 0.0).unwrap_or(true)
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
                        let it = &self.items[i];
                        if it.base_opt(ds.attr_id("maxRange")).unwrap_or(0.0) < it.distance.unwrap_or(0.0) { 0.0 } else { 1.0 }
                    } else {
                        let it = &self.items[i];
                        crate::stats::range_factor(it.base_opt(ds.attr_id("maxRange")).unwrap_or(0.0), it.base_opt(ds.attr_id("falloffEffectiveness")).unwrap_or(0.0), it.distance, true)
                    };
                    let targets: Vec<usize> = (0..self.items.len())
                        .filter(|&t| {
                            let it = &self.items[t];
                            it.loc == Loc::Ship && it.owned && if charges { it.kind == Kind::Charge } else { it.kind == Kind::Module } && it.req_skills.contains(&sk)
                        })
                        .collect();
                    for t in targets {
                        for (src_a, tgt_a) in pairs {
                            let src = Src::Projected { item: i as u32, attr: ds.attr_id(src_a), factor: tf, target: ship as u32, resist, mul: false };
                            self.push_mod(t, ds.attr_id(tgt_a), 6, src, src_cat);
                        }
                    }
                }
            } else if let Some(ps) = self.proj_special_for(i, name, resist) {
                self.proj_special.extend(ps);
            } else if DAMAGE_EFFECTS.contains(&name) {
                // weapon damage onto the target: not part of the target's own stats
            } else {
                self.warnings.push(format!("projected effect '{name}' not modelled yet"));
            }
        }
    }

    /// Warfare buffs: explicit `fleet.buffs`, the fit's own active command bursts and `fleet.booster_fits`.
    /// Per buff id the strongest |value| wins (Pyfa); explicit buffs override. Burst ids are PostAssigned by
    /// charges, so this runs on the compiled graph (stage 2). Returns true when modifiers were added.
    fn register_local_bursts(&mut self, req: &FitRequest) -> bool {
        let ds = self.ds;
        let mut agg: Vec<(u32, f64)> = Vec::new();
        for b in &req.fleet.buffs {
            let Some(info) = ds.dbuffs.get(&b.buff_id) else {
                self.warnings.push(format!("unknown warfare buff {}", b.buff_id));
                continue;
            };
            match agg.iter_mut().find(|x| x.0 == b.buff_id) {
                None => agg.push((b.buff_id, b.value)),
                Some(e) => {
                    e.1 = match info.aggregate.as_deref() {
                        Some("Minimum") => e.1.min(b.value),
                        _ => e.1.max(b.value),
                    }
                }
            }
        }
        let pairs: Vec<(u32, u32)> =
            (1..=4).map(|k| (ds.attr_id(&format!("warfareBuff{k}ID")), ds.attr_id(&format!("warfareBuff{k}Value")))).collect();
        let mut best: Vec<(u32, f64, Src)> = Vec::new();
        let collect = |f: &Fit, own: bool, best: &mut Vec<(u32, f64, Src)>| {
            for i in 0..f.items.len() {
                if f.items[i].kind != Kind::Module || f.items[i].state < State::Active {
                    continue;
                }
                for &(ida, vala) in &pairs {
                    let id = if f.has(i, ida) { f.get(i, ida) as u32 } else { 0 };
                    if id == 0 || agg.iter().any(|x| x.0 == id) {
                        continue;
                    }
                    let v = f.get(i, vala);
                    let src = if own { Src::Attr { item: i as u32, attr: vala } } else { Src::Const(v) };
                    match best.iter_mut().find(|x| x.0 == id) {
                        Some(x) => {
                            if x.1.abs() < v.abs() {
                                *x = (id, v, src)
                            }
                        }
                        None => best.push((id, v, src)),
                    }
                }
            }
        };
        collect(self, true, &mut best);
        // abyssal weather / AoE cloud beacons (Pyfa weather_* / aoe_beacon_* effects): warfareBuff1/2 of the
        // environment item join the same command-bonus pool (strongest |value| per buff id)
        for i in 0..self.items.len() {
            if self.items[i].kind != Kind::Beacon {
                continue;
            }
            let weather = self.items[i].effects.iter().any(|(e, _)| {
                ds.effects.get(e).map_or(false, |ei| ei.name.starts_with("weather_") || ei.name.starts_with("aoe_beacon_"))
            });
            if !weather {
                continue;
            }
            for &(ida, vala) in &pairs[..2] {
                let id = if self.has(i, ida) { self.get(i, ida) as u32 } else { 0 };
                if id == 0 || agg.iter().any(|x| x.0 == id) {
                    continue;
                }
                let v = self.get(i, vala);
                let src = Src::Const(v);
                match best.iter_mut().find(|x| x.0 == id) {
                    Some(x) => {
                        if x.1.abs() < v.abs() {
                            *x = (id, v, src)
                        }
                    }
                    None => best.push((id, v, src)),
                }
            }
        }
        for (k, bf) in req.fleet.booster_fits.iter().enumerate() {
            let mut breq = bf.clone();
            breq.fleet.booster_fits.clear();
            match Fit::build(ds, &breq) {
                Ok(b) => collect(&b, false, &mut best),
                Err(e) => self.warnings.push(format!("fleet.booster_fits[{k}]: {e:?}")),
            }
        }
        for &(id, value) in &agg {
            match best.iter_mut().find(|x| x.0 == id) {
                Some(x) => *x = (id, value, Src::Const(value)),
                None => best.push((id, value, Src::Const(value))),
            }
        }
        best.sort_by_key(|x| x.0);
        let before = self.raw.len();
        for (id, _, src) in best {
            self.apply_buff(id, src);
        }
        self.raw.len() != before
    }

    fn apply_buff(&mut self, id: u32, src: Src) {
        let ds = self.ds;
        let Some(info) = ds.dbuffs.get(&id) else { return };
        let op = info.op;
        let ship = self.ship;
        let mut targets = Vec::new();
        // Pyfa applies most buffs stacking-penalised; the abyssal weather resistance/HP/velocity buffs are not
        let cat = if matches!(id, 90 | 93 | 94 | 95 | 96 | 98 | 99) { 6 } else { 0 };
        for &a in &info.item {
            self.push_mod(ship, a, op, src, cat);
        }
        // AoE cloud / weather buffs also hit drones that require the Drones skill (Pyfa fit.py commandBonus)
        let drone_attrs: &[&str] = match id {
            79 => &["signatureRadius"],
            90 => &["shieldEmDamageResonance", "armorEmDamageResonance", "emDamageResonance"],
            93 => &["shieldExplosiveDamageResonance", "armorExplosiveDamageResonance", "explosiveDamageResonance"],
            95 => &["shieldThermalDamageResonance", "armorThermalDamageResonance", "thermalDamageResonance"],
            99 => &["shieldKineticDamageResonance", "armorKineticDamageResonance", "kineticDamageResonance"],
            94 => &["shieldCapacity"],
            96 => &["armorHP"],
            97 => &["maxRange", "falloff"],
            98 => &["maxVelocity"],
            _ => &[],
        };
        if !drone_attrs.is_empty() {
            let drones_skill = 3436;
            let drones: Vec<usize> = (0..self.items.len())
                .filter(|&d| self.items[d].kind == Kind::Drone && self.items[d].req_skills.contains(&drones_skill))
                .collect();
            for d in drones {
                for n in drone_attrs {
                    let a = ds.attr_id(n);
                    if a != 0 {
                        self.push_mod(d, a, op, src, cat);
                    }
                }
            }
        }
        for &a in &info.location {
            self.for_targets(ship, Func::Location, Domain::Ship, 0, &mut targets);
            for &t in &targets {
                self.push_mod(t as usize, a, op, src, cat);
            }
        }
        for &(a, g) in &info.location_group {
            self.for_targets(ship, Func::LocationGroup, Domain::Ship, g, &mut targets);
            for &t in &targets {
                self.push_mod(t as usize, a, op, src, cat);
            }
        }
        for &(a, s) in &info.location_skill {
            self.for_targets(ship, Func::LocationRequiredSkill, Domain::Ship, s, &mut targets);
            for &t in &targets {
                self.push_mod(t as usize, a, op, src, cat);
            }
        }
    }

    /// Reactive Armor Hardener adaptation (algorithm of Pyfa/eos, LGPL), staged: evaluate, then append
    /// the adapted resonances as modifiers and recompile.
    fn apply_rah(&mut self, req: &FitRequest) {
        let ds = self.ds;
        let eid = ds.effect_id("adaptiveArmorHardener");
        if eid == 0 {
            return;
        }
        let names = ["armorEmDamageResonance", "armorThermalDamageResonance", "armorKineticDamageResonance", "armorExplosiveDamageResonance"];
        let attrs: Vec<u32> = names.iter().map(|n| ds.attr_id(n)).collect();
        let shift_attr = ds.attr_id("resistanceShiftAmount");
        let rahs: Vec<usize> = (0..self.items.len())
            .filter(|&i| self.items[i].kind == Kind::Module && self.items[i].state >= State::Active && self.items[i].effects.iter().any(|(e, _)| *e == eid))
            .collect();
        let disable = req.options.rah.as_deref() == Some("disable");
        let dp = req.damage_pattern.unwrap_or(crate::request::Resists { em: 25.0, thermal: 25.0, kinetic: 25.0, explosive: 25.0 });
        let pattern = [dp.em, dp.thermal, dp.kinetic, dp.explosive];
        let ship = self.ship;
        for m in rahs {
            let mut res: Vec<f64> = attrs.iter().map(|&a| self.get(m, a)).collect();
            if !disable {
                let base: Vec<f64> = (0..4).map(|k| pattern[k] * self.get(ship, attrs[k])).collect();
                let shift = self.get(m, shift_attr) / 100.0;
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
            let cat = self.items[m].category;
            for k in 0..4 {
                if !disable {
                    self.push_mod(m, attrs[k], 7, Src::Const(res[k]), cat);
                }
                self.push_mod(ship, attrs[k], 0, Src::Const(res[k]), cat);
            }
            self.compile();
        }
    }

    // ---------------------------------------------------------------- compilation
    fn compile(&mut self) {
        let prep = self.prep;
        // 1. order raw modifiers by (target item, attr, op, registration order)
        let mut order: Vec<u32> = (0..self.raw.len() as u32).collect();
        {
            let raw = &self.raw;
            // packed key: item(16) | attr(20) | op+1(4) | seq(24) — one u64 compare per step
            let packable = raw.len() < (1 << 24) && self.items.len() < (1 << 16) && raw.iter().all(|r| r.attr < (1 << 20));
            if !packable {
                order.sort_unstable_by_key(|&k| {
                    let r = &raw[k as usize];
                    (r.item, r.attr, r.op, r.seq)
                });
            }
            let mut keyed: Vec<u64> = if !packable { Vec::new() } else { raw
                .iter()
                .map(|r| ((r.item as u64) << 48) | ((r.attr as u64 & 0xFFFFF) << 28) | (((r.op as i64 + 1) as u64 & 0xF) << 24) | r.seq as u64)
                .collect() };
            if packable {
                keyed.sort_unstable();
                for (o, k) in order.iter_mut().zip(keyed) {
                    *o = (k & 0xFF_FFFF) as u32;
                }
            }
        }
        // 2. nodes
        for it in self.items.iter_mut() {
            it.nodes.clear();
        }
        let mut meta: Vec<NodeMeta> = Vec::new();
        let mut node_of_mod: Vec<u32> = Vec::with_capacity(order.len());
        let mut last: Option<(u32, u32)> = None;
        for &k in &order {
            let r = self.raw[k as usize];
            if last != Some((r.item, r.attr)) {
                last = Some((r.item, r.attr));
                let nid = meta.len() as u32;
                let it = &mut self.items[r.item as usize];
                it.nodes.push((r.attr, nid)); // sorted because order is sorted by attr within item
                let am = prep.meta(r.attr);
                meta.push(NodeMeta {
                    base: it.base_opt(r.attr).unwrap_or(am.default),
                    high_is_good: am.high_is_good,
                    round2: am.round2,
                    min: None,
                    max: None,
                });
            }
            node_of_mod.push(meta.len() as u32 - 1);
        }
        // 3. resolve references
        let resolve = |items: &Vec<Item<'a>>, item: u32, attr: u32| -> Ref {
            let it = &items[item as usize];
            match it.node(attr) {
                Some(n) => Ref::Node(n),
                None => Ref::Const(it.base_opt(attr).unwrap_or(prep.meta(attr).default)),
            }
        };
        let mut node_item: Vec<(u32, u32)> = vec![(0, 0); meta.len()];
        for it_idx in 0..self.items.len() {
            for &(a, n) in &self.items[it_idx].nodes {
                node_item[n as usize] = (it_idx as u32, a);
            }
        }
        for (n, m) in meta.iter_mut().enumerate() {
            let (item, attr) = node_item[n];
            let am = prep.meta(attr);
            if am.min != 0 {
                m.min = Some(resolve(&self.items, item, am.min));
            }
            if am.max != 0 {
                m.max = Some(resolve(&self.items, item, am.max));
            }
        }
        let mut mods: Vec<CMod> = Vec::with_capacity(order.len());
        let mut mod_start: Vec<u32> = vec![0; meta.len() + 1];
        for (pos, &k) in order.iter().enumerate() {
            let r = self.raw[k as usize];
            let src = match r.src {
                Src::Attr { item, attr } => CSrc::Val(resolve(&self.items, item, attr)),
                Src::Const(v) => CSrc::Val(Ref::Const(v)),
                Src::Prop { module, ship, speed, thrust, mass } => CSrc::Prop {
                    speed: resolve(&self.items, module, speed),
                    thrust: resolve(&self.items, module, thrust),
                    mass: resolve(&self.items, ship, mass),
                },
                Src::Projected { item, attr, factor, target, resist, mul } => CSrc::Projected {
                    v: resolve(&self.items, item, attr),
                    factor,
                    resist: if resist != 0 { Some(resolve(&self.items, target, resist)) } else { None },
                    mul,
                },
            };
            mods.push(CMod { op: r.op, penalized: r.penalized, src });
            mod_start[node_of_mod[pos] as usize + 1] += 1;
        }
        for n in 0..meta.len() {
            mod_start[n + 1] += mod_start[n];
        }
        let nn = meta.len();
        self.g = Graph { meta, mod_start, mods, state: RefCell::new(vec![0; nn]), vals: RefCell::new(vec![0.0; nn]) };
    }

    // ---------------------------------------------------------------- evaluation
    #[inline]
    fn deps(&self, n: u32, out: &mut Vec<u32>) {
        let g = &self.g;
        let push = |r: &Ref, out: &mut Vec<u32>| {
            if let Ref::Node(x) = r {
                out.push(*x)
            }
        };
        let m = &g.meta[n as usize];
        for c in &g.mods[g.mod_start[n as usize] as usize..g.mod_start[n as usize + 1] as usize] {
            match &c.src {
                CSrc::Val(r) => push(r, out),
                CSrc::Prop { speed, thrust, mass } => {
                    push(mass, out);
                    push(speed, out);
                    push(thrust, out);
                }
                CSrc::Projected { v, resist, .. } => {
                    if let Some(r) = resist {
                        push(r, out);
                    }
                    push(v, out);
                }
            }
        }
        if let Some(r) = &m.min {
            push(r, out);
        }
        if let Some(r) = &m.max {
            push(r, out);
        }
    }

    /// Iterative post-order evaluation of `root` and everything it depends on.
    fn eval_node(&self, root: u32) -> f64 {
        {
            let st = self.g.state.borrow();
            if st[root as usize] == 2 {
                return self.g.vals.borrow()[root as usize];
            }
        }
        // stack of (node, deps pushed?)
        let mut stack: Vec<(u32, bool)> = vec![(root, false)];
        let mut deps = Vec::with_capacity(16);
        while let Some(&(n, expanded)) = stack.last() {
            if expanded {
                stack.pop();
                let v = self.compute(n);
                self.g.vals.borrow_mut()[n as usize] = v;
                self.g.state.borrow_mut()[n as usize] = 2;
                continue;
            }
            {
                let mut st = self.g.state.borrow_mut();
                if st[n as usize] != 0 {
                    stack.pop();
                    continue;
                }
                st[n as usize] = 1;
            }
            stack.last_mut().unwrap().1 = true;
            deps.clear();
            self.deps(n, &mut deps);
            let st = self.g.state.borrow();
            // reverse so the first dependency is evaluated first (same order as lazy recursion)
            for &d in deps.iter().rev() {
                if st[d as usize] == 0 {
                    stack.push((d, false));
                }
            }
        }
        self.g.vals.borrow()[root as usize]
    }

    #[inline]
    fn rv(&self, r: &Ref, st: &[u8], vals: &[f64]) -> f64 {
        match *r {
            Ref::Const(v) => v,
            // on-stack (cycle) dependency reads the base value, like the lazy engine's cycle guard
            Ref::Node(n) => {
                if st[n as usize] == 2 {
                    vals[n as usize]
                } else {
                    self.g.meta[n as usize].base
                }
            }
        }
    }

    fn compute(&self, n: u32) -> f64 {
        self.stats_evals.set(self.stats_evals.get() + 1);
        let g = &self.g;
        let st = g.state.borrow();
        let vals = g.vals.borrow();
        let m = &g.meta[n as usize];
        let mut val = m.base;
        let mods = &g.mods[g.mod_start[n as usize] as usize..g.mod_start[n as usize + 1] as usize];
        let mut i = 0;
        let mut pos: [f64; 32] = [0.0; 32];
        let mut neg: [f64; 32] = [0.0; 32];
        let mut posv: Vec<f64> = Vec::new();
        let mut negv: Vec<f64> = Vec::new();
        while i < mods.len() {
            let op = mods[i].op;
            let mut assign: Option<f64> = None;
            let (mut np, mut nn) = (0usize, 0usize);
            posv.clear();
            negv.clear();
            while i < mods.len() && mods[i].op == op {
                let c = &mods[i];
                i += 1;
                let v = match &c.src {
                    CSrc::Val(r) => self.rv(r, &st, &vals),
                    CSrc::Prop { speed, thrust, mass } => {
                        let ms = self.rv(mass, &st, &vals);
                        if ms == 0.0 { 1.0 } else { 1.0 + self.rv(speed, &st, &vals) / 100.0 * self.rv(thrust, &st, &vals) / ms }
                    }
                    CSrc::Projected { v, factor, resist, mul } => {
                        let mut f = *factor;
                        if let Some(r) = resist {
                            f *= self.rv(r, &st, &vals);
                        }
                        let x = self.rv(v, &st, &vals);
                        if *mul { (x - 1.0) * f + 1.0 } else { x * f }
                    }
                };
                match op {
                    -1 | 7 => {
                        assign = Some(match assign {
                            None => v,
                            Some(c) => {
                                if m.high_is_good {
                                    c.max(v)
                                } else {
                                    c.min(v)
                                }
                            }
                        })
                    }
                    2 => val += v,
                    3 => val -= v,
                    _ => {
                        let k = match op {
                            0 | 4 => v,
                            1 | 5 => {
                                if v == 0.0 {
                                    1.0
                                } else {
                                    1.0 / v
                                }
                            }
                            6 => 1.0 + v / 100.0,
                            _ => 1.0,
                        };
                        if c.penalized {
                            if k > 1.0 {
                                if np < 32 {
                                    pos[np] = k;
                                    np += 1
                                } else {
                                    if posv.is_empty() {
                                        posv.extend_from_slice(&pos);
                                    }
                                    posv.push(k)
                                }
                            } else if k < 1.0 {
                                if nn < 32 {
                                    neg[nn] = k;
                                    nn += 1
                                } else {
                                    if negv.is_empty() {
                                        negv.extend_from_slice(&neg);
                                    }
                                    negv.push(k)
                                }
                            }
                        } else {
                            val *= k;
                        }
                    }
                }
            }
            if let Some(v) = assign {
                val = v;
            }
            // >32 penalised modifiers of one op spill to the heap (never seen in practice)
            if posv.is_empty() { penalize(&mut val, &mut pos[..np]) } else { penalize(&mut val, &mut posv) }
            if negv.is_empty() { penalize(&mut val, &mut neg[..nn]) } else { penalize(&mut val, &mut negv) }
        }
        if let Some(r) = &m.min {
            val = val.max(self.rv(r, &st, &vals));
        }
        if let Some(r) = &m.max {
            val = val.min(self.rv(r, &st, &vals));
        }
        if m.round2 {
            val = crate::stats::py_round2(val);
        }
        val
    }

    // ---------------------------------------------------------------- query API (used by stats)
    pub fn get(&self, item: usize, attr: u32) -> f64 {
        let it = &self.items[item];
        match it.node(attr) {
            Some(n) => self.eval_node(n),
            None => it.base_opt(attr).unwrap_or(self.prep.meta(attr).default),
        }
    }

    pub fn get_opt(&self, item: usize, attr: u32) -> Option<f64> {
        if self.has(item, attr) { Some(self.get(item, attr)) } else { None }
    }

    pub fn has(&self, item: usize, attr: u32) -> bool {
        let it = &self.items[item];
        it.node(attr).is_some() || it.base_opt(attr).is_some()
    }

    pub fn base(&self, item: usize, attr: u32) -> f64 {
        self.items[item].base_opt(attr).unwrap_or(self.prep.meta(attr).default)
    }

    /// All attribute ids present on an item (base ∪ modified), sorted.
    pub fn attr_keys(&self, item: usize) -> Vec<u32> {
        let it = &self.items[item];
        let mut k: Vec<u32> = it.type_attrs.iter().map(|x| x.0).chain(it.patch.iter().map(|x| x.0)).chain(it.nodes.iter().map(|x| x.0)).collect();
        k.sort_unstable();
        k.dedup();
        k
    }

    /// Graph size (nodes, modifiers) and how many nodes were actually evaluated.
    pub fn graph_stats(&self) -> (usize, usize, u64) {
        (self.g.meta.len(), self.g.mods.len(), self.stats_evals.get())
    }
}

#[inline]
fn skill_range(v: &[(u32, u32)], skill: u32, out: &mut Vec<u32>) {
    let lo = v.partition_point(|x| x.0 < skill);
    out.extend(v[lo..].iter().take_while(|x| x.0 == skill).map(|x| x.1));
}

/// Stacking penalty: strongest first, factor exp(-(i/2.67)^2).
#[inline]
fn penalize(val: &mut f64, list: &mut [f64]) {
    if list.is_empty() {
        return;
    }
    list.sort_by(|x, y| (y - 1.0).abs().partial_cmp(&(x - 1.0).abs()).unwrap_or(std::cmp::Ordering::Equal));
    for (i, m) in list.iter().enumerate() {
        *val *= 1.0 + (m - 1.0) * (-((i * i) as f64) / 7.1289).exp();
    }
}

/// Slot from the type's slot effect (hiPower 12, medPower 13, loPower 11, rigSlot 2663, subSystem 3772, serviceSlot 6306).
pub fn infer_slot(_ds: &Dataset, t: &TypeInfo) -> Option<Slot> {
    for (e, _) in &t.effects {
        match *e {
            12 => return Some(Slot::High),
            13 => return Some(Slot::Mid),
            11 => return Some(Slot::Low),
            2663 => return Some(Slot::Rig),
            3772 => return Some(Slot::Subsystem),
            6306 => return Some(Slot::Service),
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------- phase profiler (bench-phases)
thread_local! {
    static PROF_ON: Cell<bool> = const { Cell::new(false) };
    static PROF_T: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
    pub static PROF_ACC: RefCell<[f64; 6]> = const { RefCell::new([0.0; 6]) };
}
pub fn prof_enable(on: bool) {
    PROF_ON.with(|p| p.set(on));
}
#[inline]
pub(crate) fn prof_start() {
    if PROF_ON.with(|p| p.get()) {
        PROF_T.with(|t| t.set(Some(std::time::Instant::now())));
    }
}
#[inline]
pub(crate) fn prof(k: usize) {
    if PROF_ON.with(|p| p.get()) {
        let now = std::time::Instant::now();
        PROF_T.with(|t| {
            if let Some(t0) = t.get() {
                PROF_ACC.with(|a| a.borrow_mut()[k] += (now - t0).as_secs_f64());
            }
            t.set(Some(now));
        });
    }
}

// ---------------------------------------------------------------- skill folding (dataset-level)
/// An outgoing (non-self) modifier of a skill effect.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OutMod {
    func: Func,
    domain: Domain,
    modified: u32,
    op: i32,
    extra: u32,
    structure_ok: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillFold {
    category: u32,
    outgoing: Vec<OutMod>,
    modifying: Vec<u32>,
    /// any outgoing modifier with an unconditional target (Item / Location / LocationGroup)
    unconditional: bool,
    /// skill filters of *RequiredSkill outgoing modifiers
    extras: Vec<u32>,
    /// precomputed values (snapshot): all 12 slots of `vals` flattened, slot-major; empty = probe lazily
    pre: Vec<f64>,
    /// lazily probed: [structure*6 + level][k] = value of outgoing[k]'s modifying attribute
    #[serde(skip)]
    vals: [std::sync::OnceLock<Vec<f64>>; 12],
}

impl SkillFold {
    fn values(&self, ds: &Dataset, s: u32, level: u8, structure: bool) -> &[f64] {
        let slot = structure as usize * 6 + level.min(5) as usize;
        if !self.pre.is_empty() {
            let k = self.outgoing.len();
            return &self.pre[slot * k..(slot + 1) * k];
        }
        self.vals[slot].get_or_init(|| {
            if self.outgoing.is_empty() {
                return Vec::new();
            }
            let probe = Fit::probe(ds, s, level, structure);
            self.modifying.iter().map(|&a| probe.get(0, a)).collect()
        })
    }
}

/// Dense per-attribute metadata (indexed by attribute id) — no hash lookups in compile/eval.
/// `repr(C)` so the snapshot can hold the dense table as 24-byte records read in place.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct AttrMeta {
    pub default: f64,
    pub min: u32,
    pub max: u32,
    pub stackable: bool,
    pub high_is_good: bool,
    pub round2: bool,
}
const _: () = assert!(std::mem::size_of::<AttrMeta>() == 24 && std::mem::align_of::<AttrMeta>() == 8);

/// Dense AttrMeta table: built in memory or borrowed from the snapshot bytes.
pub struct MetaTable {
    ptr: *const AttrMeta,
    len: usize,
    _own: MetaOwn,
}
#[allow(dead_code)] // owners only keep the bytes alive
enum MetaOwn {
    Vec(Vec<AttrMeta>),
    Blob(std::sync::Arc<crate::data::Blob>),
}
// SAFETY: ptr points into the owned, immutable Vec / blob
unsafe impl Send for MetaTable {}
unsafe impl Sync for MetaTable {}

impl MetaTable {
    fn from_vec(v: Vec<AttrMeta>) -> MetaTable {
        MetaTable { ptr: v.as_ptr(), len: v.len(), _own: MetaOwn::Vec(v) }
    }
    #[inline]
    pub fn as_slice(&self) -> &[AttrMeta] {
        // SAFETY: ptr/len describe valid, aligned, initialised AttrMeta records owned by `_own`
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.len * 24);
        for m in self.as_slice() {
            out.extend_from_slice(&m.default.to_le_bytes());
            out.extend_from_slice(&m.min.to_le_bytes());
            out.extend_from_slice(&m.max.to_le_bytes());
            out.extend_from_slice(&[m.stackable as u8, m.high_is_good as u8, m.round2 as u8, 0, 0, 0, 0, 0]);
        }
        out
    }
    fn decode(blob: &std::sync::Arc<crate::data::Blob>, s: usize, e: usize) -> Option<MetaTable> {
        let b: &[u8] = blob;
        let bytes = b.get(s..e)?;
        if bytes.len() % 24 != 0 || (bytes.as_ptr() as usize) % 8 != 0 {
            return None;
        }
        let len = bytes.len() / 24;
        // bool fields must hold 0 or 1 to be read as `bool`
        for r in 0..len {
            if bytes[r * 24 + 16..r * 24 + 19].iter().any(|&x| x > 1) {
                return None;
            }
        }
        Some(MetaTable { ptr: bytes.as_ptr() as *const AttrMeta, len, _own: MetaOwn::Blob(blob.clone()) })
    }
}
const ATTR_META_UNKNOWN: AttrMeta = AttrMeta { default: 0.0, stackable: true, high_is_good: true, round2: false, min: 0, max: 0 };

/// Dataset-derived, request-independent precomputation. Built once per dataset (OnceLock), immutable.
pub struct Prepared {
    pub skills_foldable: bool,
    pub published_skills: Vec<u32>,
    /// tactical destroyer modes (group 1306): (lowercase name, type id)
    pub modes: Vec<(String, u32)>,
    pub attr_meta: MetaTable,
    /// attribute ids used by fit validation (resolved once per dataset)
    pub vids: crate::stats::ValidateIds,
    folds: std::sync::Mutex<Vec<(u32, Option<std::sync::Arc<SkillFold>>)>>,
    table: FoldTable,
}

enum FoldTable {
    Built(Vec<(u32, Option<std::sync::Arc<SkillFold>>)>),
    /// from the dataset snapshot: per-skill folds with precomputed values, decoded on first use
    Snap(crate::data::LazyTable<Option<SkillFold>>),
}

/// A skill fold borrowed from the dataset table or shared from the on-demand list.
pub(crate) enum FoldRef<'p> {
    R(&'p SkillFold),
    A(std::sync::Arc<SkillFold>),
}

impl std::ops::Deref for FoldRef<'_> {
    type Target = SkillFold;
    fn deref(&self) -> &SkillFold {
        match self {
            FoldRef::R(r) => r,
            FoldRef::A(a) => a,
        }
    }
}

/// Request-independent part of `Prepared` stored in the dataset snapshot.
#[derive(serde::Serialize, serde::Deserialize)]
struct PreparedCore {
    skills_foldable: bool,
    published_skills: Vec<u32>,
    modes: Vec<(String, u32)>,
    vids: crate::stats::ValidateIds,
}

impl Prepared {
    #[inline]
    pub fn meta(&self, attr: u32) -> &AttrMeta {
        self.attr_meta.as_slice().get(attr as usize).unwrap_or(&ATTR_META_UNKNOWN)
    }

    pub fn new(ds: &Dataset) -> Prepared {
        let t0 = std::time::Instant::now();
        let p = Self::new_inner(ds);
        if std::env::var_os("VB_LOAD_TIMING").is_some() {
            eprintln!("prepared {:?}", t0.elapsed());
        }
        p
    }

    fn new_inner(ds: &Dataset) -> Prepared {
        // can any modifier reach a skill from outside? (char-location / char-location-group on a skill group)
        let foldable = ds.skills_foldable;
        let mut table: Vec<(u32, Option<std::sync::Arc<SkillFold>>)> = Vec::with_capacity(ds.skills.len());
        if foldable {
            for &s in &ds.skills {
                table.push((s, build_fold(ds, s).map(std::sync::Arc::new)));
            }
        }
        let max_attr = ds.attrs.iter().map(|(k, _)| k).max().unwrap_or(0) as usize;
        let mut attr_meta = vec![ATTR_META_UNKNOWN; max_attr + 1];
        for (id, a) in ds.attrs.iter() {
            attr_meta[id as usize] = AttrMeta {
                default: a.default,
                stackable: a.stackable,
                high_is_good: a.high_is_good,
                round2: matches!(a.name.as_str(), "cpu" | "power" | "cpuOutput" | "powerOutput"),
                min: a.min_attr.unwrap_or(0),
                max: a.max_attr.unwrap_or(0),
            };
        }
        let published_skills: Vec<u32> = ds.skills.iter().copied().filter(|s| ds.types[s].published).collect();
        let mut modes: Vec<(String, u32)> =
            ds.types.ids_in_group(1306).map(|id| (ds.types[&id].name.to_lowercase(), id)).collect();
        modes.sort_by_key(|x| x.1);
        let vids = crate::stats::ValidateIds::new(ds);
        Prepared { skills_foldable: foldable, published_skills, modes, attr_meta: MetaTable::from_vec(attr_meta), vids, folds: std::sync::Mutex::new(Vec::new()), table: FoldTable::Built(table) }
    }

    /// Snapshot sections: bincode(PreparedCore) and the fold table with every (structure, level) value probed.
    pub(crate) fn snapshot_sections(&self, ds: &Dataset) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
        let core = PreparedCore {
            skills_foldable: self.skills_foldable,
            published_skills: self.published_skills.clone(),
            modes: self.modes.clone(),
            vids: self.vids.clone(),
        };
        let mut folds: Vec<(u32, Option<SkillFold>)> = Vec::new();
        if let FoldTable::Built(t) = &self.table {
            for (s, f) in t {
                let f = f.as_ref().map(|f| {
                    let mut g: SkillFold = (**f).clone();
                    g.vals = Default::default();
                    let mut pre = Vec::with_capacity(12 * g.outgoing.len());
                    for slot in 0..12 {
                        pre.extend_from_slice(f.values(ds, *s, (slot % 6) as u8, slot >= 6));
                    }
                    g.pre = pre;
                    g
                });
                folds.push((*s, f));
            }
        }
        Ok((bincode::serialize(&core).map_err(|e| e.to_string())?, crate::data::LazyTable::encode_pairs(&folds)?, self.attr_meta.encode()))
    }

    pub(crate) fn from_snapshot(
        core: &[u8],
        folds: crate::data::LazyTable<Option<SkillFold>>,
        blob: &std::sync::Arc<crate::data::Blob>,
        meta: (usize, usize),
    ) -> Option<Prepared> {
        let c: PreparedCore = bincode::deserialize(core).ok()?;
        let attr_meta = MetaTable::decode(blob, meta.0, meta.1)?;
        Some(Prepared {
            skills_foldable: c.skills_foldable,
            published_skills: c.published_skills,
            modes: c.modes,
            attr_meta,
            vids: c.vids,
            folds: std::sync::Mutex::new(Vec::new()),
            table: FoldTable::Snap(folds),
        })
    }

    fn fold(&self, ds: &Dataset, s: u32) -> Option<FoldRef<'_>> {
        match &self.table {
            FoldTable::Built(t) => {
                if let Ok(p) = t.binary_search_by_key(&s, |x| x.0) {
                    return t[p].1.clone().map(FoldRef::A);
                }
            }
            FoldTable::Snap(t) => {
                if let Some(f) = t.get(&s) {
                    return f.as_ref().map(FoldRef::R);
                }
            }
        }
        // explicit non-category-16 "skill" ids: compute on demand (rare)
        let mut g = self.folds.lock().unwrap();
        if let Some(x) = g.iter().find(|x| x.0 == s) {
            return x.1.clone().map(FoldRef::A);
        }
        let f = build_fold(ds, s).map(std::sync::Arc::new);
        g.push((s, f.clone()));
        f.map(FoldRef::A)
    }
}

fn build_fold(ds: &Dataset, s: u32) -> Option<SkillFold> {
    let t = ds.types.get(&s)?;
    let structure_ok_ids: Vec<u32> = STRUCTURE_SKILL_EFFECT_NAMES.iter().map(|n| ds.effect_id(n)).collect();
    let mut outgoing = Vec::new();
    let mut modifying = Vec::new();
    for &(eid, _) in &t.effects {
        if eid == EFFECT_SKILL_EFFECT {
            continue;
        }
        let Some(e) = ds.effects.get(&eid) else { continue };
        if e.fitting_usage_chance_attr.is_some() || !state_ok(e.category, State::Online) {
            continue;
        }
        let structure_ok = structure_ok_ids.contains(&eid) || e.mods.iter().all(|m| m.domain == Domain::Item);
        for m in &e.mods {
            if m.func == Func::EffectStopper || m.op == 9 || matches!(m.domain, Domain::TargetId | Domain::Target) {
                continue;
            }
            if m.domain == Domain::Item || m.domain == Domain::Other {
                continue; // self modifiers are folded into the values; skills have no charge/parent
            }
            let extra = if m.extra == 0 && matches!(m.func, Func::LocationRequiredSkill | Func::OwnerRequiredSkill) { s } else { m.extra };
            outgoing.push(OutMod { func: m.func, domain: m.domain, modified: m.modified, op: m.op, extra, structure_ok });
            modifying.push(m.modifying);
        }
    }
    let unconditional = outgoing.iter().any(|m| matches!(m.func, Func::Item | Func::Location | Func::LocationGroup));
    let mut extras: Vec<u32> = outgoing
        .iter()
        .filter(|m| matches!(m.func, Func::LocationRequiredSkill | Func::OwnerRequiredSkill))
        .map(|m| m.extra)
        .collect();
    extras.sort_unstable();
    extras.dedup();
    Some(SkillFold { category: t.category, outgoing, modifying, unconditional, extras, pre: Vec::new(), vals: Default::default() })
}

impl<'a> Fit<'a> {
    /// A one-item fit holding only skill `s` at `level`, with the skill's self modifiers registered.
    fn probe(ds: &'a Dataset, s: u32, level: u8, structure: bool) -> Fit<'a> {
        let mut fit = Fit {
            ds,
            prep: ds.prepared.get_or_init(|| Prepared::new(ds)),
            items: Vec::with_capacity(1),
            ship: 0,
            char: 0,
            warnings: Vec::new(),
            is_structure: structure,
            proj_special: Vec::new(),
            raw: Vec::new(),
            g: Graph::default(),
            idx_ship_loc: Vec::new(),
            idx_owned: Vec::new(),
            idx_char_loc: Vec::new(),
            idx_char_skillable: Vec::new(),
            by_skill_ship: Vec::new(),
            by_skill_owned: Vec::new(),
            by_skill_char: Vec::new(),
            stats_evals: Cell::new(0),
            skills: Vec::new(),
            folded: false,
        };
        let idx = fit.new_item(s, Kind::Skill, Loc::Char, "").expect("skill type");
        fit.items[idx].set_base(ATTR_SKILL_LEVEL, level as f64);
        fit.items[idx].owned = false;
        let structure_ok_ids: Vec<u32> = STRUCTURE_SKILL_EFFECT_NAMES.iter().map(|n| ds.effect_id(n)).collect();
        let effects = fit.items[idx].effects.clone();
        for (eid, _) in effects {
            if eid == EFFECT_SKILL_EFFECT {
                continue;
            }
            let Some(e) = ds.effects.get(&eid) else { continue };
            if structure && !structure_ok_ids.contains(&eid) && !e.mods.iter().all(|m| m.domain == Domain::Item) {
                continue;
            }
            if e.fitting_usage_chance_attr.is_some() || !state_ok(e.category, State::Online) {
                continue;
            }
            for m in &e.mods {
                if m.func == Func::EffectStopper || m.op == 9 || m.domain != Domain::Item || m.func != Func::Item {
                    continue;
                }
                fit.push_mod(0, m.modified, m.op, Src::Attr { item: 0, attr: m.modifying }, t_cat(ds, s));
            }
        }
        fit.compile();
        fit
    }
}

fn t_cat(ds: &Dataset, s: u32) -> u32 {
    ds.types.get(&s).map(|t| t.category).unwrap_or(16)
}
