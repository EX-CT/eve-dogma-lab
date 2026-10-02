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
    pub items: Vec<Item<'a>>,
    pub ship: usize,
    pub char: usize,
    pub warnings: Vec<String>,
    pub is_structure: bool,
    raw: Vec<RawMod>,
    g: Graph,
    /// index lists for target selection
    idx_ship_loc: Vec<u32>,
    idx_owned: Vec<u32>,
    idx_char_loc: Vec<u32>,
    idx_char_skillable: Vec<u32>,
    pub stats_evals: Cell<u64>,
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
            items: Vec::with_capacity(ds.skills.len() + 64),
            ship: 0,
            char: 0,
            warnings: Vec::new(),
            is_structure: false,
            raw: Vec::with_capacity(4096),
            g: Graph::default(),
            idx_ship_loc: Vec::new(),
            idx_owned: Vec::new(),
            idx_char_loc: Vec::new(),
            idx_char_skillable: Vec::new(),
            stats_evals: Cell::new(0),
        };
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
        let mut skill_ids: Vec<(u32, u8)> =
            ds.skills.iter().filter(|s| ds.types[s].published).map(|s| (*s, default_level)).collect();
        for (id, l) in explicit {
            match skill_ids.binary_search_by_key(&id, |x| x.0) {
                Ok(p) => skill_ids[p].1 = l,
                Err(p) => skill_ids.insert(p, (id, l)),
            }
        }
        for (s, l) in skill_ids {
            if !ds.types.contains_key(&s) {
                continue;
            }
            let idx = fit.new_item(s, Kind::Skill, Loc::Char, "/character/skills")?;
            fit.items[idx].set_base(ATTR_SKILL_LEVEL, l.min(5) as f64);
            fit.items[idx].owned = false;
        }
        let mode_id = req.ship.mode_type_id.or_else(|| {
            let ship_name = ds.types.get(&req.ship.type_id)?.name.to_lowercase();
            let m = ds
                .types
                .iter()
                .filter(|(_, t)| t.group == 1306 && t.name.to_lowercase().starts_with(&ship_name))
                .map(|(id, _)| *id)
                .min()?;
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
            it.fighter_abilities = f.abilities.clone().or_else(|| {
                let mut ids: Vec<u32> = it.effects.iter().map(|(e, _)| *e).collect();
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
                    } else if !std_seen
                        && !matches!(n, "fighterAbilityMicroWarpDrive" | "fighterAbilityEvasiveManeuvers" | "fighterAbilityMicroJumpDrive")
                    {
                        on.push(e);
                    }
                }
                Some(on)
            });
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
        fit.build_indexes();
        fit.register_all(req);
        fit.compile();
        // stage 2: command bursts need evaluated buff ids
        if fit.register_local_bursts(req) {
            fit.compile();
        }
        fit.apply_rah(req);
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
    }

    // ---------------------------------------------------------------- registration
    #[inline]
    fn push_mod(&mut self, target: usize, attr: u32, op: i32, src: Src, source_cat: u32) {
        let stackable = self.ds.attrs.get(&attr).map(|a| a.stackable).unwrap_or(true);
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
                    Func::LocationRequiredSkill => out.extend(
                        self.idx_ship_loc.iter().copied().filter(|&i| items[i as usize].req_skills.contains(&extra)),
                    ),
                    Func::OwnerRequiredSkill => out
                        .extend(self.idx_owned.iter().copied().filter(|&i| items[i as usize].req_skills.contains(&extra))),
                    Func::EffectStopper => {}
                }
            }
            Domain::Char => match func {
                Func::Item => out.push(self.char as u32),
                Func::Location => out.extend_from_slice(&self.idx_char_loc),
                Func::LocationGroup => {
                    out.extend(self.idx_char_loc.iter().copied().filter(|&i| items[i as usize].group == extra))
                }
                Func::LocationRequiredSkill | Func::OwnerRequiredSkill => out.extend(
                    self.idx_char_skillable.iter().copied().filter(|&i| items[i as usize].req_skills.contains(&extra)),
                ),
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
        for i in 0..n {
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
                if !state_ok(e.category, state) {
                    continue;
                }
                let ship = self.ship;
                let iu = i as u32;
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
        self.register_explicit_buffs(req);
    }

    fn register_projected(&mut self, i: usize) {
        let ds = self.ds;
        let src_cat = self.items[i].category;
        let state = self.items[i].state;
        let effects = self.items[i].effects.clone();
        let ship = self.ship;
        for (eid, _) in effects {
            let Some(e) = ds.effects.get(&eid) else { continue };
            if e.category != 2 && e.category != 3 {
                continue;
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
                self.items[i].base_opt(ds.attr_id("remoteResistanceID")).map(|a| a as u32).unwrap_or(0)
            });
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
            if !e.mods.is_empty() {
                for m in &e.mods {
                    if matches!(m.domain, Domain::TargetId | Domain::Target | Domain::Ship) && m.func == Func::Item {
                        push(self, m.modified, m.modifying, m.op);
                    }
                }
                continue;
            }
            let name = e.name.as_str();
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
            } else {
                self.warnings.push(format!("projected effect '{name}' not modelled yet"));
            }
        }
    }

    fn register_explicit_buffs(&mut self, req: &FitRequest) {
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
        agg.sort_by_key(|x| x.0);
        for (id, value) in agg {
            self.apply_buff(id, Src::Const(value));
        }
    }

    /// Local command bursts: buff ids are PostAssigned by charges, so they need an evaluated graph.
    /// Returns true when modifiers were added (graph must be recompiled).
    fn register_local_bursts(&mut self, req: &FitRequest) -> bool {
        let ds = self.ds;
        let pairs: Vec<(u32, u32)> =
            (1..=4).map(|k| (ds.attr_id(&format!("warfareBuff{k}ID")), ds.attr_id(&format!("warfareBuff{k}Value")))).collect();
        let explicit: Vec<u32> = req.fleet.buffs.iter().map(|b| b.buff_id).collect();
        let before = self.raw.len();
        for i in 0..self.items.len() {
            if self.items[i].kind != Kind::Module || self.items[i].state < State::Active {
                continue;
            }
            for &(ida, vala) in &pairs {
                let id = if self.has(i, ida) { self.get(i, ida) as u32 } else { 0 };
                if id == 0 || explicit.contains(&id) {
                    continue;
                }
                self.apply_buff(id, Src::Attr { item: i as u32, attr: vala });
            }
        }
        self.raw.len() != before
    }

    fn apply_buff(&mut self, id: u32, src: Src) {
        let ds = self.ds;
        let Some(info) = ds.dbuffs.get(&id) else { return };
        let op = info.op;
        let ship = self.ship;
        let mut targets = Vec::new();
        for &a in &info.item {
            self.push_mod(ship, a, op, src, 0);
        }
        for &a in &info.location {
            self.for_targets(ship, Func::Location, Domain::Ship, 0, &mut targets);
            for &t in &targets {
                self.push_mod(t as usize, a, op, src, 0);
            }
        }
        for &(a, g) in &info.location_group {
            self.for_targets(ship, Func::LocationGroup, Domain::Ship, g, &mut targets);
            for &t in &targets {
                self.push_mod(t as usize, a, op, src, 0);
            }
        }
        for &(a, s) in &info.location_skill {
            self.for_targets(ship, Func::LocationRequiredSkill, Domain::Ship, s, &mut targets);
            for &t in &targets {
                self.push_mod(t as usize, a, op, src, 0);
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
        let ds = self.ds;
        // 1. order raw modifiers by (target item, attr, op, registration order)
        let mut order: Vec<u32> = (0..self.raw.len() as u32).collect();
        {
            let raw = &self.raw;
            order.sort_unstable_by_key(|&k| {
                let r = &raw[k as usize];
                (r.item, r.attr, r.op, r.seq)
            });
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
                let info = ds.attrs.get(&r.attr);
                meta.push(NodeMeta {
                    base: it.base_opt(r.attr).unwrap_or_else(|| ds.attr_default(r.attr)),
                    high_is_good: info.map(|i| i.high_is_good).unwrap_or(true),
                    round2: info.map(|i| matches!(i.name.as_str(), "cpu" | "power" | "cpuOutput" | "powerOutput")).unwrap_or(false),
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
                None => Ref::Const(it.base_opt(attr).unwrap_or_else(|| ds.attr_default(attr))),
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
            if let Some(info) = ds.attrs.get(&attr) {
                m.min = info.min_attr.map(|a| resolve(&self.items, item, a));
                m.max = info.max_attr.map(|a| resolve(&self.items, item, a));
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
            val = (val * 100.0).round() / 100.0;
        }
        val
    }

    // ---------------------------------------------------------------- query API (used by stats)
    pub fn get(&self, item: usize, attr: u32) -> f64 {
        let it = &self.items[item];
        match it.node(attr) {
            Some(n) => self.eval_node(n),
            None => it.base_opt(attr).unwrap_or_else(|| self.ds.attr_default(attr)),
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
        self.items[item].base_opt(attr).unwrap_or_else(|| self.ds.attr_default(attr))
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
