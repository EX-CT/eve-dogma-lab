//! Request -> canonical list of item specs (pure, no evaluation).
//! Semantics follow EX-CT/eve-dogma-rs `Fit::build` (LGPL-3.0-or-later), restructured into immutable specs
//! with stable identities (`ItemKey`) so that the salsa database can diff two requests item by item.
use crate::data::{Dataset, TypeInfo};
use crate::request::{FitRequest, ModuleReq, Slot, State};
use crate::data::Func;
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::Arc;
use smallvec::SmallVec;

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

/// Per-session cache of type templates and (skill, level) specs, so the ~500 skill items of every
/// request are shared `Arc`s (pointer-equal across requests -> no diff, no re-validation).
#[derive(Default)]
pub struct SpecCache {
    types: FxHashMap<u32, Arc<ItemSpec>>,
    skills: FxHashMap<(u32, u8), Arc<ItemSpec>>,
    modes: Option<Vec<(u32, String)>>,
    /// per skill: conditions under which one of its modifiers can reach an item (see `skill_relevant`)
    reach: FxHashMap<u32, Arc<Reach>>,
    levels: Option<(u8, Vec<(String, u8)>, Arc<Vec<(u32, u8, Arc<Reach>)>>)>,
    level_specs: Vec<Option<Arc<ItemSpec>>>,
    level_specs_for: Option<Arc<Vec<(u32, u8, Arc<Reach>)>>>,
}

/// When can a skill's modifiers reach something? `always`, or any listed group present / skill required.
#[derive(Default)]
pub struct Reach {
    always: bool,
    groups: Vec<u32>,
    skills: Vec<u32>,
}

const EFFECT_SKILL_EFFECT: u32 = 132;

fn reach_of(ds: &Dataset, s: u32) -> Reach {
    let mut r = Reach::default();
    let Some(t) = ds.types.get(&s) else { return r };
    for (eid, _) in &t.effects {
        if *eid == EFFECT_SKILL_EFFECT {
            continue;
        }
        let Some(e) = ds.effects.get(eid) else { continue };
        if e.mods.is_empty() {
            r.always = true; // hand-written / special effect
            return r;
        }
        for m in &e.mods {
            match m.func {
                Func::Item | Func::Location | Func::EffectStopper => {
                    r.always = true;
                    return r;
                }
                Func::LocationGroup => {
                    if ds.groups.get(&m.extra).map(|g| g.category == 16).unwrap_or(true) {
                        r.always = true;
                        return r;
                    }
                    r.groups.push(m.extra);
                }
                Func::LocationRequiredSkill | Func::OwnerRequiredSkill => r.skills.push(if m.extra == 0 { s } else { m.extra }),
            }
        }
    }
    r
}

/// Skills required by any item of the request and the groups present (for skill pruning, as Variant A).
fn fit_skill_context(ds: &Dataset, req: &FitRequest) -> (FxHashSet<u32>, FxHashSet<u32>) {
    let mut need = FxHashSet::default();
    let mut groups = FxHashSet::default();
    let mut add = |tid: u32| {
        if let Some(t) = ds.types.get(&tid) {
            groups.insert(t.group);
            for a in REQ_SKILL_ATTRS {
                if let Some(v) = t.attr(a) {
                    if v != 0.0 {
                        need.insert(v as u32);
                    }
                }
            }
        }
    };
    add(req.ship.type_id);
    if let Some(m) = req.ship.mode_type_id {
        add(m);
    }
    for m in &req.modules {
        add(m.type_id);
        if let Some(c) = m.charge_type_id {
            add(c);
        }
        if let Some(mu) = &m.mutation {
            add(mu.base_type_id);
        }
    }
    for d in &req.drones {
        add(d.type_id);
        if let Some(mu) = &d.mutation {
            add(mu.base_type_id);
        }
    }
    for f in &req.fighters {
        add(f.type_id);
    }
    for i in &req.implants {
        add(*i);
    }
    for b in &req.boosters {
        add(b.type_id);
    }
    for c in &req.cargo {
        add(c.type_id);
    }
    (need, groups)
}

/// Effective rolled values of a mutation (CONTRACT-MUTATED §2.4, Pyfa `Mutator`): every attribute the mutaplasmid
/// lists and the base type has, starting from the base type's value; a given value is kept when value/base lies within
/// the 3-decimal-rounded [min, max] multipliers and clamped to that range otherwise; base 0 gives 0; attributes the
/// mutaplasmid does not list are ignored. Without a known mutaplasmid the given values are used as they are.
pub fn mutated_values(ds: &Dataset, m: &crate::request::Mutation) -> Vec<(u32, f64)> {
    let muta = m.mutaplasmid_type_id.and_then(|id| ds.mutaplasmids.get(&id));
    let base_t = ds.types.get(&m.base_type_id);
    let (Some(mu), Some(bt)) = (muta, base_t) else {
        let mut v: Vec<(u32, f64)> = m.attributes.iter().filter_map(|(k, v)| k.parse::<u32>().ok().map(|a| (a, *v))).collect();
        v.sort_unstable_by_key(|x| x.0);
        return v;
    };
    let mut out: Vec<(u32, f64)> = Vec::with_capacity(mu.attrs.len());
    for (k, (lo, hi)) in &mu.attrs {
        let Ok(aid) = k.parse::<u32>() else { continue };
        let bv = if aid == 4 { Some(bt.mass) } else { bt.attr(aid) };
        let Some(bv) = bv else { continue };
        let v = m.attributes.get(k).copied().unwrap_or(bv);
        let (lo, hi) = (round3(*lo), round3(*hi));
        let val = if bv == 0.0 {
            0.0
        } else {
            let r = v / bv;
            if lo <= r && r <= hi {
                v
            } else {
                let (a, b) = (lo * bv, hi * bv);
                let (mn, mx) = if a < b { (a, b) } else { (b, a) };
                v.clamp(mn, mx)
            }
        };
        out.push((aid, val));
    }
    out.sort_unstable_by_key(|x| x.0);
    out
}

/// Python round(x, 3) for the mutaplasmid multipliers (they have at most a few decimals, so no tie issues)
fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

struct B<'a> {
    ds: &'a Dataset,
    cache: &'a mut SpecCache,
    keys: Vec<ItemKey>,
    items: Vec<Arc<ItemSpec>>,
    warnings: Vec<String>,
}

pub struct Built {
    pub keys: Vec<ItemKey>,
    pub items: Vec<Arc<ItemSpec>>,
    pub warnings: Vec<String>,
    pub is_structure: bool,
}

fn template(ds: &Dataset, t: &TypeInfo) -> ItemSpec {
    let mut attrs: FxHashMap<u32, f64> = FxHashMap::default();
    set_type_attrs(&mut attrs, t);
    let mut v: Vec<(u32, f64)> = attrs.into_iter().collect();
    v.sort_unstable_by_key(|x| x.0);
    let _ = ds;
    ItemSpec {
        type_id: t.id,
        group: t.group,
        category: t.category,
        kind: Kind::Module,
        state: State::Online,
        loc: Loc::Nowhere,
        owned: false,
        parent: None,
        charge: None,
        slot: None,
        req_index: None,
        quantity: 1,
        active_count: 0,
        attrs: v,
        req_skills: REQ_SKILL_ATTRS.iter().filter_map(|a| t.attr(*a)).map(|v| v as u32).filter(|v| *v != 0).collect(),
        effects: t.effects.clone(),
        fighter_abilities: None,
        booster_side_effects: Vec::new(),
        spool: None,
        distance: None,
    }
}

impl ItemSpec {
    pub fn set(&mut self, a: u32, v: f64) {
        match self.attrs.binary_search_by_key(&a, |x| x.0) {
            Ok(i) => self.attrs[i].1 = v,
            Err(i) => self.attrs.insert(i, (a, v)),
        }
    }
}

impl<'a> B<'a> {
    fn new_item(&mut self, key: ItemKey, type_id: u32, kind: Kind, loc: Loc, path: &str) -> Result<usize, EngineError> {
        let Some(t) = self.ds.types.get(&type_id) else {
            return Err(EngineError { code: "UNKNOWN_TYPE", message: format!("unknown type_id {type_id}"), path: path.to_string() });
        };
        let ds = self.ds;
        let tm = self.cache.types.entry(type_id).or_insert_with(|| Arc::new(template(ds, t)));
        let mut sp: ItemSpec = (**tm).clone();
        sp.kind = kind;
        sp.loc = loc;
        sp.owned = matches!(kind, Kind::Module | Kind::Charge | Kind::Drone | Kind::Fighter | Kind::Ship);
        self.items.push(Arc::new(sp));
        self.keys.push(key);
        Ok(self.items.len() - 1)
    }

    #[inline]
    fn m(&mut self, idx: usize) -> &mut ItemSpec {
        Arc::make_mut(&mut self.items[idx])
    }

    fn apply_mutation(&mut self, idx: usize, m: &crate::request::Mutation) {
        let ds = self.ds;
        if let Some(base) = ds.types.get(&m.base_type_id) {
            let item = self.m(idx);
            let own = item.effects.clone();
            let mut attrs: FxHashMap<u32, f64> = item.attrs.iter().copied().collect();
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
            let mut v: Vec<(u32, f64)> = attrs.into_iter().collect();
            v.sort_unstable_by_key(|x| x.0);
            item.attrs = v;
        }
        for (aid, val) in mutated_values(ds, m) {
            self.m(idx).set(aid, val);
        }
    }

    fn add_module(&mut self, i: usize, m: &ModuleReq, path: &str) -> Result<usize, EngineError> {
        let idx = self.new_item(ItemKey::Module(i as u32), m.type_id, Kind::Module, Loc::Ship, path)?;
        let slot = m.slot.or_else(|| infer_slot(self.ds, &self.ds.types[&m.type_id]));
        let it = Arc::make_mut(&mut self.items[idx]);
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
            self.m(cidx).parent = Some(idx);
            self.m(cidx).req_index = Some(i);
            self.m(idx).charge = Some(cidx);
        }
        Ok(idx)
    }
}

/// Build the canonical item list for a request (same order as the reference engine registers modifiers).
/// Frozen projected-fit item: (type id, copies, evaluated attribute values, kind, quantity, fighter abilities).
pub type Frozen = Vec<(u32, u32, Vec<(u32, f64)>, Kind, u32, Option<Vec<u32>>)>;

/// Pyfa default: standard attack on; other abilities (except MWD/evasive/MJD) on only if they come before the
/// standard attack in effect order.
pub fn default_fighter_abilities(ds: &Dataset, effects: &[(u32, bool)]) -> Vec<u32> {
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

pub fn build(ds: &Dataset, cache: &mut SpecCache, req: &FitRequest, proj_fit: &mut dyn FnMut(usize, &FitRequest) -> Result<Frozen, EngineError>) -> Result<Built, EngineError> {
    let mut b = B { ds, cache, keys: Vec::with_capacity(600), items: Vec::with_capacity(600), warnings: Vec::new() };
    let ship = b.new_item(ItemKey::Ship, req.ship.type_id, Kind::Ship, Loc::Ship, "/ship/type_id")?;
    let is_structure = b.items[ship].category == 65;
    let ch = b.new_item(ItemKey::Char, 1373, Kind::Char, Loc::Char, "/character")?;
    if let Some(sec) = req.character.security_status {
        let a = ds.attr_id("pilotSecurityStatus");
        if a != 0 {
            b.m(ch).set(a, sec);
        }
    }
    let default_level = req.character.skills.default_level.unwrap_or(0);
    // the (skill, level) list depends only on the character's skills: memoised (one entry; most requests share it)
    let mut lkey: Vec<(String, u8)> = req.character.skills.levels.iter().map(|(k, v)| (k.clone(), *v)).collect();
    lkey.sort_unstable();
    let hit = match &b.cache.levels {
        Some((d, k, lv)) if *d == default_level && *k == lkey => Some(lv.clone()),
        _ => None,
    };
    let lv: Arc<Vec<(u32, u8, Arc<Reach>)>> = match hit {
        Some(lv) => lv,
        None => {
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
            let mut lv: Vec<(u32, u8)> = levels.into_iter().filter(|(s, _)| ds.types.contains_key(s)).collect();
            lv.sort();
            let lv: Vec<(u32, u8, Arc<Reach>)> = lv
                .into_iter()
                .map(|(s, l)| (s, l, b.cache.reach.entry(s).or_insert_with(|| Arc::new(reach_of(ds, s))).clone()))
                .collect();
            let lv = Arc::new(lv);
            b.cache.levels = Some((default_level, lkey, lv.clone()));
            lv
        }
    };
    let (need, groups) = fit_skill_context(ds, req);
    // resolved skill specs for this level list (parallel to `lv`), memoised with it
    if b.cache.level_specs.len() != lv.len() || !b.cache.level_specs_for.as_ref().is_some_and(|p| Arc::ptr_eq(p, &lv)) {
        b.cache.level_specs = vec![None; lv.len()];
        b.cache.level_specs_for = Some(lv.clone());
    }
    for (li, (s, l, r)) in lv.iter().enumerate() {
        let (s, l) = (*s, *l);
        // perf (as Variant A): a skill whose modifiers can reach nothing in this fit is not instantiated
        if !need.contains(&s) {
            if !(r.always || r.groups.iter().any(|g| groups.contains(g)) || r.skills.iter().any(|k| need.contains(k))) {
                continue;
            }
        }
        let lvl = l.min(5);
        if let Some(sp) = &b.cache.level_specs[li] {
            b.items.push(sp.clone());
            b.keys.push(ItemKey::Skill(s));
            continue;
        }
        let sp = match b.cache.skills.get(&(s, lvl)) {
            Some(sp) => sp.clone(),
            None => {
                let idx = b.new_item(ItemKey::Skill(s), s, Kind::Skill, Loc::Char, "/character/skills")?;
                b.m(idx).set(ATTR_SKILL_LEVEL, lvl as f64);
                b.m(idx).owned = false;
                let sp = b.items.pop().unwrap();
                b.keys.pop();
                b.cache.skills.insert((s, lvl), sp.clone());
                sp
            }
        };
        b.cache.level_specs[li] = Some(sp.clone());
        b.items.push(sp);
        b.keys.push(ItemKey::Skill(s));
    }
    let mode_id = req.ship.mode_type_id.or_else(|| {
        let ship_name = ds.types.get(&req.ship.type_id)?.name.to_lowercase();
        let modes = b.cache.modes.get_or_insert_with(|| {
            let mut m: Vec<(u32, String)> = ds.types.iter().filter(|(_, t)| t.group == 1306).map(|(id, t)| (*id, t.name.to_lowercase())).collect();
            m.sort();
            m
        });
        // sorted by id: the first match is the lowest type id
        let m = modes.iter().find(|(_, n)| n.starts_with(&ship_name)).map(|(id, _)| *id)?;
        b.warnings.push(format!("no tactical mode given; defaulted to type {m}"));
        Some(m)
    });
    if let Some(mode) = mode_id {
        let idx = b.new_item(ItemKey::Mode, mode, Kind::Mode, Loc::Nowhere, "/ship/mode_type_id")?;
        b.m(idx).owned = false;
    }
    for (i, m) in req.modules.iter().enumerate() {
        b.add_module(i, m, &format!("/modules/{i}"))?;
    }
    for (i, d) in req.drones.iter().enumerate() {
        let idx = b.new_item(ItemKey::Drone(i as u32), d.type_id, Kind::Drone, Loc::Space, &format!("/drones/{i}"))?;
        if let Some(mu) = &d.mutation {
            b.apply_mutation(idx, mu);
        }
        let it = Arc::make_mut(&mut b.items[idx]);
        it.quantity = d.quantity.max(1);
        it.active_count = d.active.unwrap_or(0).min(it.quantity);
        it.state = if it.active_count > 0 { State::Active } else { State::Offline };
        it.req_index = Some(i);
    }
    for (i, f) in req.fighters.iter().enumerate() {
        let idx = b.new_item(ItemKey::Fighter(i as u32), f.type_id, Kind::Fighter, Loc::Space, &format!("/fighters/{i}"))?;
        let sq = ds.attr_id("fighterSquadronMaxSize");
        let maxsq = b.items[idx].base(sq).map(|a| a as u32).unwrap_or(1);
        let it = Arc::make_mut(&mut b.items[idx]);
        it.quantity = f.quantity.unwrap_or(maxsq).clamp(1, maxsq.max(1));
        if f.quantity.unwrap_or(0) > maxsq {
            b.warnings.push(format!("fighters/{i}: squadron size {} capped to {maxsq}", f.quantity.unwrap_or(0)));
        }
        let it = Arc::make_mut(&mut b.items[idx]);
        it.active_count = if f.active { it.quantity } else { 0 };
        it.state = if f.active { State::Active } else { State::Offline };
        it.fighter_abilities = f.abilities.clone().or_else(|| Some(default_fighter_abilities(ds, &it.effects)));
        it.req_index = Some(i);
    }
    // Pyfa HandledImplantList/HandledBoosterList.append: an entry whose slot (implantness 331 / boosterness 1087)
    // is already taken by an earlier entry is ignored (CONTRACT-MUTATED §3.1)
    let slot_of = |t: u32, a: u32| ds.types.get(&t).and_then(|ti| ti.attr(a)).map(|v| v.to_bits());
    let mut taken: SmallVec<[u64; 8]> = SmallVec::new();
    for (i, imp) in req.implants.iter().enumerate() {
        if let Some(s) = slot_of(*imp, 331) {
            if taken.contains(&s) {
                continue;
            }
            taken.push(s);
        }
        let idx = b.new_item(ItemKey::Implant(i as u32), *imp, Kind::Implant, Loc::Char, &format!("/implants/{i}"))?;
        b.m(idx).owned = false;
        b.m(idx).req_index = Some(i);
    }
    taken.clear();
    for (i, bo) in req.boosters.iter().enumerate() {
        if let Some(s) = slot_of(bo.type_id, 1087) {
            if taken.contains(&s) {
                continue;
            }
            taken.push(s);
        }
        let idx = b.new_item(ItemKey::Booster(i as u32), bo.type_id, Kind::Booster, Loc::Char, &format!("/boosters/{i}"))?;
        b.m(idx).owned = false;
        b.m(idx).booster_side_effects = bo.side_effects.clone();
        b.m(idx).req_index = Some(i);
    }
    for (i, e) in req.environment.effect_type_ids.iter().enumerate() {
        let idx = b.new_item(ItemKey::Beacon(i as u32), *e, Kind::Beacon, Loc::Nowhere, &format!("/environment/effect_type_ids/{i}"))?;
        b.m(idx).owned = false;
    }
    for (i, p) in req.projected.iter().enumerate() {
        match p.kind.as_str() {
            "module" => {
                if let Some(m) = &p.module {
                    for k in 0..p.amount.max(1) {
                        let idx = b.new_item(ItemKey::Projected(i as u32, k as u32), m.type_id, Kind::Projected, Loc::Nowhere, &format!("/projected/{i}"))?;
                        let it = Arc::make_mut(&mut b.items[idx]);
                        it.owned = false;
                        it.state = m.state.unwrap_or(State::Active);
                        it.distance = p.distance_m;
                        it.req_index = Some(i);
                        if let Some(c) = m.charge_type_id {
                            let cidx = b.new_item(ItemKey::ProjCharge(i as u32, k as u32), c, Kind::Charge, Loc::Nowhere, &format!("/projected/{i}/module/charge_type_id"))?;
                            b.m(cidx).parent = Some(idx);
                            b.m(cidx).owned = false;
                            b.m(idx).charge = Some(cidx);
                        }
                    }
                }
            }
            "drone" => {
                if let Some(d) = &p.drone {
                    for k in 0..(p.amount.max(1) * d.quantity.max(1)) {
                        let idx = b.new_item(ItemKey::Projected(i as u32, k as u32), d.type_id, Kind::Projected, Loc::Nowhere, &format!("/projected/{i}"))?;
                        let it = Arc::make_mut(&mut b.items[idx]);
                        it.owned = false;
                        it.state = State::Active;
                        it.distance = p.distance_m;
                    }
                }
            }
            "fighter" => {
                if let Some(f) = &p.fighter {
                    for k in 0..p.amount.max(1) {
                        let idx = b.new_item(ItemKey::Projected(i as u32, k as u32), f.type_id, Kind::Projected, Loc::Nowhere, &format!("/projected/{i}"))?;
                        let sq = ds.attr_id("fighterSquadronMaxSize");
                        let maxsq = b.items[idx].base(sq).map(|a| a as u32).unwrap_or(1).max(1);
                        let it = Arc::make_mut(&mut b.items[idx]);
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
                    for (type_id, copies, vals, kind, qty, abil) in frozen {
                        for _ in 0..copies * p.amount.max(1) {
                            let idx = b.new_item(ItemKey::Projected(i as u32, k), type_id, Kind::Projected, Loc::Nowhere, &format!("/projected/{i}"))?;
                            k += 1;
                            let it = Arc::make_mut(&mut b.items[idx]);
                            it.owned = false;
                            it.state = State::Active;
                            it.distance = p.distance_m;
                            it.req_index = Some(i);
                            if kind == Kind::Fighter {
                                it.quantity = qty;
                                it.active_count = qty;
                                it.fighter_abilities = abil.clone();
                            }
                            for (a, v) in &vals {
                                b.m(idx).set(*a, *v);
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
        for i in 0..b.items.len() {
            if let Some(v) = b.items[i].base(src_id) {
                if b.items[i].base(dst_id) != Some(v) {
                    b.m(i).set(dst_id, v);
                }
            }
        }
    }
    for o in &req.overrides {
        for i in 0..b.items.len() {
            if b.items[i].type_id == o.type_id {
                b.m(i).set(o.attribute_id, o.value);
            }
        }
    }
    let B { keys, items, warnings, .. } = b;
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
