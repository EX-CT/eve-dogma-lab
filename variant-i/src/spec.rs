//! Request -> canonical list of item specs (pure, no evaluation).
//! Semantics follow EX-CT/eve-dogma-rs `Fit::build` (LGPL-3.0-or-later), restructured into immutable specs
//! with stable identities (`ItemKey`) so that the salsa database can diff two requests item by item.
use crate::data::{Dataset, TypeInfo};
use crate::request::{FitRequest, ModuleReq, Slot, State};
use rustc_hash::FxHashMap;

/// requiredSkill1..6
pub const REQ_SKILL_ATTRS: [u32; 6] = [182, 183, 184, 1285, 1289, 1290];
pub const ATTR_SKILL_LEVEL: u32 = 280;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Loc {
    Ship,
    Char,
    Space,
    Nowhere,
}

/// Stable identity of an item across requests (used to reuse salsa inputs and memos).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ItemKey {
    Ship,
    Char,
    Skill(u32),
    Mode,
    Module(u32),
    Charge(u32),
    Drone(u32),
    Fighter(u32),
    Implant(u32),
    Booster(u32),
    Beacon(u32),
    Projected(u32, u32),
    ProjCharge(u32, u32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemSpec {
    pub type_id: u32,
    pub group: u32,
    pub category: u32,
    pub kind: Kind,
    pub state: State,
    pub loc: Loc,
    pub owned: bool,
    /// canonical index (into the spec list) of the parent / charge; remapped to slots by the session
    pub parent: Option<usize>,
    pub charge: Option<usize>,
    pub slot: Option<Slot>,
    pub req_index: Option<usize>,
    pub quantity: u32,
    pub active_count: u32,
    /// base attribute values (sorted by attribute id)
    pub attrs: Vec<(u32, f64)>,
    pub req_skills: Vec<u32>,
    pub effects: Vec<(u32, bool)>,
    pub fighter_abilities: Option<Vec<u32>>,
    pub booster_side_effects: Vec<u32>,
    pub spool: Option<crate::request::Spool>,
    pub distance: Option<f64>,
}

impl ItemSpec {
    #[inline]
    pub fn base(&self, a: u32) -> Option<f64> {
        self.attrs.binary_search_by_key(&a, |x| x.0).ok().map(|i| self.attrs[i].1)
    }
}

#[derive(Debug)]
pub struct EngineError {
    pub code: &'static str,
    pub message: String,
    pub path: String,
}

struct B<'a> {
    ds: &'a Dataset,
    keys: Vec<ItemKey>,
    items: Vec<ItemSpec>,
    attrs: Vec<FxHashMap<u32, f64>>,
    warnings: Vec<String>,
}

pub struct Built {
    pub keys: Vec<ItemKey>,
    pub items: Vec<ItemSpec>,
    pub warnings: Vec<String>,
    pub is_structure: bool,
}

impl<'a> B<'a> {
    fn new_item(&mut self, key: ItemKey, type_id: u32, kind: Kind, loc: Loc, path: &str) -> Result<usize, EngineError> {
        let t = self.ds.types.get(&type_id).ok_or_else(|| EngineError {
            code: "UNKNOWN_TYPE",
            message: format!("unknown type_id {type_id}"),
            path: path.to_string(),
        })?;
        let mut attrs: FxHashMap<u32, f64> = FxHashMap::default();
        set_type_attrs(&mut attrs, t);
        let req_skills = REQ_SKILL_ATTRS.iter().filter_map(|a| t.attr(*a)).map(|v| v as u32).filter(|v| *v != 0).collect();
        self.items.push(ItemSpec {
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
            attrs: Vec::new(),
            req_skills,
            effects: t.effects.clone(),
            fighter_abilities: None,
            booster_side_effects: Vec::new(),
            spool: None,
            distance: None,
        });
        self.attrs.push(attrs);
        self.keys.push(key);
        Ok(self.items.len() - 1)
    }

    fn apply_mutation(&mut self, idx: usize, m: &crate::request::Mutation) {
        let ds = self.ds;
        if let Some(base) = ds.types.get(&m.base_type_id) {
            let own = self.items[idx].effects.clone();
            let item = &mut self.items[idx];
            let attrs = &mut self.attrs[idx];
            for (a, v) in &base.attrs {
                attrs.insert(*a, *v);
            }
            for (a, v) in &ds.types[&item.type_id].attrs {
                attrs.insert(*a, *v);
            }
            for (e, d) in &base.effects {
                if !own.iter().any(|(x, _)| x == e) {
                    item.effects.push((*e, *d));
                }
            }
            if item.req_skills.is_empty() {
                item.req_skills = REQ_SKILL_ATTRS.iter().filter_map(|a| base.attr(*a)).map(|v| v as u32).filter(|v| *v != 0).collect();
            }
            if attrs.get(&4).copied().unwrap_or(0.0) == 0.0 && base.mass != 0.0 {
                attrs.insert(4, base.mass);
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
            self.attrs[idx].insert(aid, val);
        }
    }

    fn add_module(&mut self, i: usize, m: &ModuleReq, path: &str) -> Result<usize, EngineError> {
        let idx = self.new_item(ItemKey::Module(i as u32), m.type_id, Kind::Module, Loc::Ship, path)?;
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
            let cidx = self.new_item(ItemKey::Charge(i as u32), c, Kind::Charge, Loc::Ship, &format!("{path}/charge_type_id"))?;
            self.items[cidx].parent = Some(idx);
            self.items[cidx].req_index = Some(i);
            self.items[idx].charge = Some(cidx);
        }
        Ok(idx)
    }
}

/// Build the canonical item list for a request (same order as the reference engine registers modifiers).
/// Frozen projected-fit item: (type id, copies, evaluated attribute values).
pub type Frozen = Vec<(u32, u32, Vec<(u32, f64)>)>;

pub fn build(ds: &Dataset, req: &FitRequest, proj_fit: &mut dyn FnMut(usize, &FitRequest) -> Result<Frozen, EngineError>) -> Result<Built, EngineError> {
    let mut b = B { ds, keys: Vec::with_capacity(600), items: Vec::with_capacity(600), attrs: Vec::with_capacity(600), warnings: Vec::new() };
    let ship = b.new_item(ItemKey::Ship, req.ship.type_id, Kind::Ship, Loc::Ship, "/ship/type_id")?;
    let is_structure = b.items[ship].category == 65;
    let ch = b.new_item(ItemKey::Char, 1373, Kind::Char, Loc::Char, "/character")?;
    if let Some(sec) = req.character.security_status {
        let a = ds.attr_id("pilotSecurityStatus");
        if a != 0 {
            b.attrs[ch].insert(a, sec);
        }
    }
    let default_level = req.character.skills.default_level.unwrap_or(0);
    let mut levels: FxHashMap<u32, u8> = FxHashMap::default();
    for s in &ds.skills {
        if ds.types[s].published {
            levels.insert(*s, default_level);
        }
    }
    for (k, v) in &req.character.skills.levels {
        if let Ok(id) = k.parse::<u32>() {
            levels.insert(id, *v);
        } else if let Some(id) = ds.type_by_name(k) {
            levels.insert(id, *v);
        }
    }
    let mut lv: Vec<(u32, u8)> = levels.into_iter().collect();
    lv.sort();
    for (s, l) in lv {
        if !ds.types.contains_key(&s) {
            continue;
        }
        let idx = b.new_item(ItemKey::Skill(s), s, Kind::Skill, Loc::Char, "/character/skills")?;
        b.attrs[idx].insert(ATTR_SKILL_LEVEL, l.min(5) as f64);
        b.items[idx].owned = false;
    }
    let mode_id = req.ship.mode_type_id.or_else(|| {
        let ship_name = ds.types.get(&req.ship.type_id)?.name.to_lowercase();
        let m = ds
            .types
            .iter()
            .filter(|(_, t)| t.group == 1306 && t.name.to_lowercase().starts_with(&ship_name))
            .map(|(id, _)| *id)
            .min()?;
        b.warnings.push(format!("no tactical mode given; defaulted to type {m}"));
        Some(m)
    });
    if let Some(mode) = mode_id {
        let idx = b.new_item(ItemKey::Mode, mode, Kind::Mode, Loc::Nowhere, "/ship/mode_type_id")?;
        b.items[idx].owned = false;
    }
    for (i, m) in req.modules.iter().enumerate() {
        b.add_module(i, m, &format!("/modules/{i}"))?;
    }
    for (i, d) in req.drones.iter().enumerate() {
        let idx = b.new_item(ItemKey::Drone(i as u32), d.type_id, Kind::Drone, Loc::Space, &format!("/drones/{i}"))?;
        if let Some(mu) = &d.mutation {
            b.apply_mutation(idx, mu);
        }
        let it = &mut b.items[idx];
        it.quantity = d.quantity.max(1);
        it.active_count = d.active.unwrap_or(0).min(it.quantity);
        it.state = if it.active_count > 0 { State::Active } else { State::Offline };
        it.req_index = Some(i);
    }
    for (i, f) in req.fighters.iter().enumerate() {
        let idx = b.new_item(ItemKey::Fighter(i as u32), f.type_id, Kind::Fighter, Loc::Space, &format!("/fighters/{i}"))?;
        let sq = ds.attr_id("fighterSquadronMaxSize");
        let maxsq = b.attrs[idx].get(&sq).map(|a| *a as u32).unwrap_or(1);
        let it = &mut b.items[idx];
        it.quantity = f.quantity.unwrap_or(maxsq).clamp(1, maxsq.max(1));
        if f.quantity.unwrap_or(0) > maxsq {
            b.warnings.push(format!("fighters/{i}: squadron size {} capped to {maxsq}", f.quantity.unwrap_or(0)));
        }
        let it = &mut b.items[idx];
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
        let idx = b.new_item(ItemKey::Implant(i as u32), *imp, Kind::Implant, Loc::Char, &format!("/implants/{i}"))?;
        b.items[idx].owned = false;
        b.items[idx].req_index = Some(i);
    }
    for (i, bo) in req.boosters.iter().enumerate() {
        let idx = b.new_item(ItemKey::Booster(i as u32), bo.type_id, Kind::Booster, Loc::Char, &format!("/boosters/{i}"))?;
        b.items[idx].owned = false;
        b.items[idx].booster_side_effects = bo.side_effects.clone();
        b.items[idx].req_index = Some(i);
    }
    for (i, e) in req.environment.effect_type_ids.iter().enumerate() {
        let idx = b.new_item(ItemKey::Beacon(i as u32), *e, Kind::Beacon, Loc::Nowhere, &format!("/environment/effect_type_ids/{i}"))?;
        b.items[idx].owned = false;
    }
    for (i, p) in req.projected.iter().enumerate() {
        match p.kind.as_str() {
            "module" => {
                if let Some(m) = &p.module {
                    for k in 0..p.amount.max(1) {
                        let idx = b.new_item(ItemKey::Projected(i as u32, k as u32), m.type_id, Kind::Projected, Loc::Nowhere, &format!("/projected/{i}"))?;
                        let it = &mut b.items[idx];
                        it.owned = false;
                        it.state = m.state.unwrap_or(State::Active);
                        it.distance = p.distance_m;
                        it.req_index = Some(i);
                        if let Some(c) = m.charge_type_id {
                            let cidx = b.new_item(ItemKey::ProjCharge(i as u32, k as u32), c, Kind::Charge, Loc::Nowhere, &format!("/projected/{i}/module/charge_type_id"))?;
                            b.items[cidx].parent = Some(idx);
                            b.items[cidx].owned = false;
                            b.items[idx].charge = Some(cidx);
                        }
                    }
                }
            }
            "drone" => {
                if let Some(d) = &p.drone {
                    for k in 0..(p.amount.max(1) * d.quantity.max(1)) {
                        let idx = b.new_item(ItemKey::Projected(i as u32, k as u32), d.type_id, Kind::Projected, Loc::Nowhere, &format!("/projected/{i}"))?;
                        let it = &mut b.items[idx];
                        it.owned = false;
                        it.state = State::Active;
                        it.distance = p.distance_m;
                    }
                }
            }
            "fit" => {
                if let Some(src_req) = &p.fit {
                    let mut sreq = (**src_req).clone();
                    sreq.projected.clear();
                    let frozen = match proj_fit(i, &sreq) {
                        Ok(f) => f,
                        Err(e) => {
                            b.warnings.push(format!("projected[{i}] fit: {e:?}"));
                            continue;
                        }
                    };
                    let mut k = 0u32;
                    for (type_id, copies, vals) in frozen {
                        for _ in 0..copies * p.amount.max(1) {
                            let idx = b.new_item(ItemKey::Projected(i as u32, k), type_id, Kind::Projected, Loc::Nowhere, &format!("/projected/{i}"))?;
                            k += 1;
                            let it = &mut b.items[idx];
                            it.owned = false;
                            it.state = State::Active;
                            it.distance = p.distance_m;
                            it.req_index = Some(i);
                            for (a, v) in &vals {
                                b.attrs[idx].insert(*a, *v);
                            }
                        }
                    }
                }
            }
            other => b.warnings.push(format!("projected kind '{other}' not supported yet (index {i})")),
        }
    }
    {
        let sec = req.environment.system_security.as_deref().unwrap_or("nullsec").to_lowercase();
        let src = match sec.as_str() {
            "hisec" | "highsec" | "high" => "hiSecModifier",
            "lowsec" | "low" => "lowSecModifier",
            "nullsec" | "null" | "wspace" | "wormhole" | "w-space" => "nullSecModifier",
            other => {
                b.warnings.push(format!("unknown system_security '{other}', using nullsec"));
                "nullSecModifier"
            }
        };
        let (src_id, dst_id) = (ds.attr_id(src), ds.attr_id("securityModifier"));
        for a in b.attrs.iter_mut() {
            if let Some(v) = a.get(&src_id).copied() {
                a.insert(dst_id, v);
            }
        }
    }
    for o in &req.overrides {
        for (it, a) in b.items.iter().zip(b.attrs.iter_mut()) {
            if it.type_id == o.type_id {
                a.insert(o.attribute_id, o.value);
            }
        }
    }
    let B { keys, mut items, attrs, warnings, .. } = b;
    for (it, a) in items.iter_mut().zip(attrs) {
        let mut v: Vec<(u32, f64)> = a.into_iter().collect();
        v.sort_unstable_by_key(|x| x.0);
        it.attrs = v;
    }
    Ok(Built { keys, items, warnings, is_structure })
}

fn set_type_attrs(attrs: &mut FxHashMap<u32, f64>, t: &TypeInfo) {
    for (a, v) in &t.attrs {
        attrs.insert(*a, *v);
    }
    for (a, v) in [(4u32, t.mass), (38, t.capacity), (161, t.volume), (162, t.radius)] {
        if v != 0.0 || !attrs.contains_key(&a) {
            attrs.insert(a, v);
        }
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
