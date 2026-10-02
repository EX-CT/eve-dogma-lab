//! World construction (spawn system) and the modifier systems, run in a fixed schedule:
//!
//! 1. `spawn`            request -> entities (+ mutation, security, overrides)
//! 2. `build_index`      location / group / required-skill indices (the "spatial" lookup for modifier filters)
//! 3. `local_effects`    active effects of every item -> modifiers on target entities (incl. special handlers)
//! 4. `projected_effects` effects projected onto the ship (webs, paints, damps...) with range factor
//! 5. `fleet_buffs`      explicit warfare buffs + local command bursts (needs evaluated buff ids)
//! 6. `rah_adapt`        reactive armor hardener simulation (needs evaluated resists)
//!
//! Systems 3-5 are split in a read phase (queries over the world, producing `PendingMod`s - a command buffer)
//! and a write phase that appends to the targets' `Attrs` component.
use crate::calc::Calc;
use crate::components::*;
use crate::views::Views;
use crate::data::{Dataset, Domain, Func, TypeInfo};
use crate::request::{FitRequest, ModuleReq, Mutation, Resists, Slot, State};
use hecs::{Entity, World};
use rustc_hash::FxHashMap;

/// Source categories exempt from stacking penalties: Ship, Charge, Skill, Implant, Subsystem, Structure.
const EXEMPT_CATEGORIES: [u32; 6] = [6, 8, 16, 20, 32, 65];
/// em/explosive/kinetic/thermal DamageResonance (hull)
const HULL_RESONANCES: [u32; 4] = [113, 111, 109, 110];
const CHARACTER_TYPE: u32 = 1373;

#[derive(Debug)]
pub struct EngineError {
    pub code: &'static str,
    pub message: String,
    pub path: String,
}

/// Per-request registry ("resources" in ECS terms): well-known entities and ordered entity lists.
pub struct Fit<'a> {
    pub ds: &'a Dataset,
    pub world: World,
    pub ship: Entity,
    pub char: Entity,
    /// every entity in spawn order (deterministic iteration)
    pub order: Vec<Entity>,
    pub skills: Vec<(Entity, u32, u8)>,
    pub modules: Vec<Entity>,
    pub drones: Vec<Entity>,
    pub fighters: Vec<Entity>,
    pub is_structure: bool,
    pub warnings: Vec<String>,
    index: Index,
}

#[derive(Default)]
struct Index {
    ship_loc: Vec<Entity>,
    ship_loc_group: FxHashMap<u32, Vec<Entity>>,
    char_loc: Vec<Entity>,
    char_loc_group: FxHashMap<u32, Vec<Entity>>,
    /// required skill -> (entity, loc, owned, is_skill)
    by_skill: FxHashMap<u32, Vec<(Entity, Loc, bool, bool)>>,
}

enum Special {
    Rep(IncomingRep),
    Drain(IncomingDrain),
}

/// weapon / mining effects of projected items: they damage the target, not part of its own stats
const DAMAGE_EFFECTS: &[&str] = &["projectileFired", "targetAttack", "useMissiles", "barrage", "targetDisintegratorAttack",
    "missileLaunchingForEntity", "fighterAbilityAttackM", "fighterAbilityMissiles", "superWeaponAmarr", "superWeaponCaldari",
    "superWeaponGallente", "superWeaponMinmatar", "mining", "miningLaser", "miningClouds", "dotMissileLaunching"];

struct PendingMod {
    target: Entity,
    attr: u32,
    m: Mod,
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

/// Slot from the type's slot effect (hiPower 12, medPower 13, loPower 11, rigSlot 2663, subSystem 3772, serviceSlot 6306).
pub fn infer_slot(t: &TypeInfo) -> Option<Slot> {
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

impl<'a> Fit<'a> {
    // ------------------------------------------------------------------ helpers
    pub fn item(&self, e: Entity) -> Item {
        *self.world.get::<&Item>(e).unwrap()
    }
    pub fn state(&self, e: Entity) -> State {
        self.world.get::<&Power>(e).unwrap().0
    }
    pub fn calc(&self) -> Calc<'_> {
        Calc::new(self.ds, &self.world)
    }
    pub fn views(&self) -> Views<'_> {
        Views::new(self.ds, &self.world)
    }

    // ------------------------------------------------------------------ system 1: spawn
    fn spawn_item(&mut self, type_id: u32, kind: Kind, loc: Loc, state: State, path: &str) -> Result<Entity, EngineError> {
        let t = self.ds.types.get(&type_id).ok_or_else(|| EngineError {
            code: "UNKNOWN_TYPE",
            message: format!("unknown type_id {type_id}"),
            path: path.to_string(),
        })?;
        let owned = matches!(kind, Kind::Module | Kind::Charge | Kind::Drone | Kind::Fighter | Kind::Ship);
        let e = self.world.spawn((
            Item { type_id, group: t.group, category: t.category, kind, loc, owned },
            Power(state),
            Attrs { type_id, slots: FxHashMap::default() },
        ));
        self.order.push(e);
        Ok(e)
    }

    fn set_base(&mut self, e: Entity, attr: u32, v: f64) {
        let mut a = self.world.get::<&mut Attrs>(e).unwrap();
        a.slots.insert(attr, AttrSlot::new(v));
    }

    fn apply_mutation(&mut self, e: Entity, m: &Mutation) {
        let ds = self.ds;
        let type_id = self.item(e).type_id;
        let own = &ds.types[&type_id];
        if let Some(base) = ds.types.get(&m.base_type_id) {
            let mut effects = own.effects.clone();
            for (x, d) in &base.effects {
                if !own.effects.iter().any(|(y, _)| y == x) {
                    effects.push((*x, *d));
                }
            }
            let req_skills = if own.req_skills.is_empty() { base.req_skills.clone() } else { own.req_skills.clone() };
            {
                let mut a = self.world.get::<&mut Attrs>(e).unwrap();
                for (k, v) in &base.attrs {
                    if own.attr(*k).is_none() {
                        a.slots.insert(*k, AttrSlot::new(*v));
                    }
                }
                if own.attr(4).unwrap_or(0.0) == 0.0 && base.mass != 0.0 {
                    a.slots.insert(4, AttrSlot::new(base.mass));
                }
            }
            self.world.insert_one(e, Mutated { effects, req_skills }).unwrap();
        }
        let muta = m.mutaplasmid_type_id.and_then(|id| ds.mutaplasmids.get(&id));
        let base_t = ds.types.get(&m.base_type_id);
        for (k, v) in &m.attributes {
            let Ok(aid) = k.parse::<u32>() else { continue };
            let mut val = *v;
            if let (Some(mu), Some(bt)) = (muta, base_t) {
                if let (Some((lo, hi)), Some(bv)) = (mu.attrs.get(k), bt.attr(aid)) {
                    let (x, y) = (bv * lo, bv * hi);
                    let (mn, mx) = if x < y { (x, y) } else { (y, x) };
                    if bv != 0.0 {
                        val = val.clamp(mn, mx);
                    }
                }
            }
            self.set_base(e, aid, val);
        }
    }

    fn add_module(&mut self, i: usize, m: &ModuleReq) -> Result<Entity, EngineError> {
        let path = format!("/modules/{i}");
        let t = self.ds.types.get(&m.type_id);
        let slot = m.slot.or_else(|| t.and_then(infer_slot));
        let mut state = m.state.unwrap_or(State::Online);
        if matches!(slot, Some(Slot::Rig) | Some(Slot::Subsystem)) && state != State::Offline {
            state = State::Online;
        }
        let e = self.spawn_item(m.type_id, Kind::Module, Loc::Ship, state, &path)?;
        if let Some(mu) = &m.mutation {
            self.apply_mutation(e, mu);
        }
        let mut charge = None;
        if let Some(c) = m.charge_type_id {
            let ce = self.spawn_item(c, Kind::Charge, Loc::Ship, state, &format!("{path}/charge_type_id"))?;
            self.world.insert_one(ce, LoadedIn(e)).unwrap();
            charge = Some(ce);
        }
        self.world.insert_one(e, Fitted { slot, req_index: i, charge, spool: m.spool }).unwrap();
        self.modules.push(e);
        Ok(e)
    }

    pub fn spawn(ds: &'a Dataset, req: &FitRequest) -> Result<Fit<'a>, EngineError> {
        let world = World::new();
        let placeholder = world.reserve_entity();
        let mut fit = Fit {
            ds,
            world,
            ship: placeholder,
            char: placeholder,
            order: Vec::with_capacity(600),
            skills: Vec::with_capacity(520),
            modules: Vec::new(),
            drones: Vec::new(),
            fighters: Vec::new(),
            is_structure: false,
            warnings: Vec::new(),
            index: Index::default(),
        };
        fit.ship = fit.spawn_item(req.ship.type_id, Kind::Ship, Loc::Ship, State::Online, "/ship/type_id")?;
        fit.is_structure = fit.item(fit.ship).category == 65;
        fit.char = fit.spawn_item(CHARACTER_TYPE, Kind::Char, Loc::Char, State::Online, "/character")?;
        if let Some(sec) = req.character.security_status {
            if ds.a.pilot_sec != 0 {
                fit.set_base(fit.char, ds.a.pilot_sec, sec);
            }
        }
        // skills: every published skill at default_level (untrained = 0), then explicit levels
        let default_level = req.character.skills.default_level.unwrap_or(0);
        let mut levels: FxHashMap<u32, u8> = ds.published_skills.iter().map(|s| (*s, default_level)).collect();
        for (k, v) in &req.character.skills.levels {
            if let Ok(id) = k.parse::<u32>() {
                levels.insert(id, *v);
            } else if let Some(id) = ds.type_by_name(k) {
                levels.insert(id, *v);
            }
        }
        let mut lv: Vec<(u32, u8)> = levels.into_iter().filter(|(s, _)| ds.types.contains_key(s)).collect();
        lv.sort();
        for (s, l) in lv {
            let l = l.min(5);
            let e = fit.spawn_item(s, Kind::Skill, Loc::Char, State::Online, "/character/skills")?;
            fit.set_base(e, ds.a.skill_level, l as f64);
            fit.skills.push((e, s, l));
        }
        // tactical destroyers: default to the first mode like the client / Pyfa
        let mode_id = req.ship.mode_type_id.or_else(|| {
            let ship_name = ds.types.get(&req.ship.type_id)?.name.to_lowercase();
            let m = ds.t3d_modes.iter().find(|(n, _)| n.starts_with(&ship_name)).map(|x| x.1)?;
            fit.warnings.push(format!("no tactical mode given; defaulted to type {m}"));
            Some(m)
        });
        if let Some(mode) = mode_id {
            fit.spawn_item(mode, Kind::Mode, Loc::Nowhere, State::Online, "/ship/mode_type_id")?;
        }
        for (i, m) in req.modules.iter().enumerate() {
            fit.add_module(i, m)?;
        }
        for (i, d) in req.drones.iter().enumerate() {
            let quantity = d.quantity.max(1);
            let active = d.active.unwrap_or(0).min(quantity);
            let st = if active > 0 { State::Active } else { State::Offline };
            let e = fit.spawn_item(d.type_id, Kind::Drone, Loc::Space, st, &format!("/drones/{i}"))?;
            if let Some(mu) = &d.mutation {
                fit.apply_mutation(e, mu);
            }
            fit.world.insert_one(e, Squad { quantity, active, req_index: i }).unwrap();
            fit.drones.push(e);
        }
        for (i, f) in req.fighters.iter().enumerate() {
            let st = if f.active { State::Active } else { State::Offline };
            let e = fit.spawn_item(f.type_id, Kind::Fighter, Loc::Space, st, &format!("/fighters/{i}"))?;
            let t = &ds.types[&f.type_id];
            let maxsq = t.attr(ds.a.fighter_sq_max).map(|v| v as u32).unwrap_or(1);
            let quantity = f.quantity.unwrap_or(maxsq).clamp(1, maxsq.max(1));
            if f.quantity.unwrap_or(0) > maxsq {
                fit.warnings.push(format!("fighters/{i}: squadron size {} capped to {maxsq}", f.quantity.unwrap_or(0)));
            }
            let abilities = f.abilities.clone().unwrap_or_else(|| default_fighter_abilities(ds, t));
            fit.world
                .insert(e, (Squad { quantity, active: if f.active { quantity } else { 0 }, req_index: i }, FighterAbilities(abilities)))
                .unwrap();
            fit.fighters.push(e);
        }
        for (i, imp) in req.implants.iter().enumerate() {
            fit.spawn_item(*imp, Kind::Implant, Loc::Char, State::Online, &format!("/implants/{i}"))?;
        }
        for (i, b) in req.boosters.iter().enumerate() {
            let e = fit.spawn_item(b.type_id, Kind::Booster, Loc::Char, State::Online, &format!("/boosters/{i}"))?;
            fit.world.insert_one(e, SideEffects(b.side_effects.clone())).unwrap();
        }
        for (i, b) in req.environment.effect_type_ids.iter().enumerate() {
            fit.spawn_item(*b, Kind::Beacon, Loc::Nowhere, State::Online, &format!("/environment/effect_type_ids/{i}"))?;
        }
        for (i, p) in req.projected.iter().enumerate() {
            let path = format!("/projected/{i}");
            match p.kind.as_str() {
                "module" => {
                    if let Some(m) = &p.module {
                        for _ in 0..p.amount.max(1) {
                            let st = m.state.unwrap_or(State::Active);
                            let e = fit.spawn_item(m.type_id, Kind::Projected, Loc::Nowhere, st, &path)?;
                            let mut charge = None;
                            if let Some(c) = m.charge_type_id {
                                let ce = fit.spawn_item(c, Kind::Charge, Loc::Nowhere, st, &format!("{path}/module/charge_type_id"))?;
                                fit.world.insert_one(ce, LoadedIn(e)).unwrap();
                                fit.world.get::<&mut Item>(ce).unwrap().owned = false;
                                charge = Some(ce);
                            }
                            fit.world.insert(e, (Distance(p.distance_m), Fitted { slot: None, req_index: i, charge, spool: None })).unwrap();
                        }
                    }
                }
                "fit" => {
                    // whole projected fit: its own world (own skills, implants, fleet), then every active module /
                    // drone is projected as a frozen entity carrying the source-modified attribute values
                    if let Some(src_req) = &p.fit {
                        let mut sreq = (**src_req).clone();
                        sreq.projected.clear();
                        let frozen = match Fit::run(ds, &sreq) {
                            Ok(src) => src.frozen_projectors(),
                            Err(e) => {
                                fit.warnings.push(format!("projected[{i}] fit: {} {}", e.code, e.message));
                                continue;
                            }
                        };
                        for (type_id, copies, vals) in frozen {
                            for _ in 0..copies * p.amount.max(1) {
                                let e = fit.spawn_item(type_id, Kind::Projected, Loc::Nowhere, State::Active, &path)?;
                                fit.world.insert_one(e, Distance(p.distance_m)).unwrap();
                                for (a, v) in &vals {
                                    fit.set_base(e, *a, *v);
                                }
                            }
                        }
                    }
                }
                "drone" => {
                    if let Some(d) = &p.drone {
                        for _ in 0..(p.amount.max(1) * d.quantity.max(1)) {
                            let e = fit.spawn_item(d.type_id, Kind::Projected, Loc::Nowhere, State::Active, &path)?;
                            fit.world.insert_one(e, Distance(p.distance_m)).unwrap();
                        }
                    }
                }
                other => fit.warnings.push(format!("projected kind '{other}' not supported yet (index {i})")),
            }
        }
        // system security -> securityModifier (default nullsec, like Pyfa)
        let sec = req.environment.system_security.as_deref().unwrap_or("nullsec").to_lowercase();
        let src = match sec.as_str() {
            "hisec" | "highsec" | "high" => ds.a.hisec_mod,
            "lowsec" | "low" => ds.a.lowsec_mod,
            "nullsec" | "null" | "wspace" | "wormhole" | "w-space" => ds.a.nullsec_mod,
            other => {
                fit.warnings.push(format!("unknown system_security '{other}', using nullsec"));
                ds.a.nullsec_mod
            }
        };
        let order = fit.order.clone();
        for &e in &order {
            let v = {
                let c = fit.calc();
                if c.has(e, src) { Some(c.base(e, src)) } else { None }
            };
            if let Some(v) = v {
                fit.set_base(e, ds.a.sec_mod, v);
            }
        }
        for o in &req.overrides {
            for &e in &order {
                if fit.item(e).type_id == o.type_id {
                    fit.set_base(e, o.attribute_id, o.value);
                }
            }
        }
        Ok(fit)
    }

    /// Active modules (1 copy) and active drones (n copies) with all their evaluated attributes.
    pub fn frozen_projectors(&self) -> Vec<(u32, u32, Vec<(u32, f64)>)> {
        let c = self.calc();
        let v = self.views();
        let mut out = Vec::new();
        for &e in &self.order {
            let it = v.item(e);
            let copies = match it.kind {
                Kind::Module if v.state(e) >= State::Active => 1,
                Kind::Drone => v.squad(e).active,
                _ => 0,
            };
            if copies == 0 {
                continue;
            }
            let vals = c.attr_ids(e).into_iter().map(|a| (a, c.get(e, a))).collect();
            out.push((it.type_id, copies, vals));
        }
        out
    }

    // ------------------------------------------------------------------ system 2: index
    pub fn build_index(&mut self) {
        let mut ix = Index::default();
        let v = self.views();
        for &e in &self.order {
            let it = v.item(e);
            match it.loc {
                Loc::Ship => {
                    ix.ship_loc.push(e);
                    ix.ship_loc_group.entry(it.group).or_default().push(e);
                }
                Loc::Char => {
                    ix.char_loc.push(e);
                    ix.char_loc_group.entry(it.group).or_default().push(e);
                }
                _ => {}
            }
            for &s in v.req_skills(e) {
                ix.by_skill.entry(s).or_default().push((e, it.loc, it.owned, it.kind == Kind::Skill));
            }
        }
        drop(v);
        self.index = ix;
    }

    fn targets(&self, v: &Views, src: Entity, func: Func, domain: Domain, extra: u32, out: &mut Vec<Entity>) {
        out.clear();
        let ix = &self.index;
        match domain {
            Domain::Item => {
                if func == Func::Item {
                    out.push(src)
                }
            }
            Domain::Other => {
                if let Some(c) = v.charge(src).or_else(|| v.parent(src)) {
                    out.push(c)
                }
            }
            Domain::Ship | Domain::Structure => {
                if domain == Domain::Structure && !self.is_structure {
                    return;
                }
                match func {
                    Func::Item => out.push(self.ship),
                    Func::Location => out.extend_from_slice(&ix.ship_loc),
                    Func::LocationGroup => {
                        if let Some(v) = ix.ship_loc_group.get(&extra) {
                            out.extend_from_slice(v)
                        }
                    }
                    Func::LocationRequiredSkill => {
                        if let Some(v) = ix.by_skill.get(&extra) {
                            out.extend(v.iter().filter(|x| x.1 == Loc::Ship).map(|x| x.0))
                        }
                    }
                    Func::OwnerRequiredSkill => {
                        if let Some(v) = ix.by_skill.get(&extra) {
                            out.extend(v.iter().filter(|x| x.2).map(|x| x.0))
                        }
                    }
                    Func::EffectStopper => {}
                }
            }
            Domain::Char => match func {
                Func::Item => out.push(self.char),
                Func::Location => out.extend_from_slice(&ix.char_loc),
                Func::LocationGroup => {
                    if let Some(v) = ix.char_loc_group.get(&extra) {
                        out.extend_from_slice(v)
                    }
                }
                Func::LocationRequiredSkill | Func::OwnerRequiredSkill => {
                    if let Some(v) = ix.by_skill.get(&extra) {
                        out.extend(v.iter().filter(|x| (x.2 || x.1 == Loc::Char) && !x.3).map(|x| x.0))
                    }
                }
                Func::EffectStopper => {}
            },
            _ => {}
        }
    }

    fn pending(&self, target: Entity, attr: u32, op: i32, src: Src, source_cat: u32, out: &mut Vec<PendingMod>) {
        let stackable = self.ds.attrs.get(&attr).map(|a| a.stackable).unwrap_or(true);
        let penalized = !stackable && !EXEMPT_CATEGORIES.contains(&source_cat);
        out.push(PendingMod { target, attr, m: Mod { op: op as i8, penalized, src } });
    }

    /// write phase: append pending modifiers to the target entities' Attrs
    fn apply(&self, pend: Vec<PendingMod>) {
        let ds = self.ds;
        let mut view = self.world.view::<&mut Attrs>();
        for p in pend {
            let a = view.get_mut(p.target).unwrap();
            let tid = a.type_id;
            a.slots
                .entry(p.attr)
                .or_insert_with(|| {
                    let base = ds.types.get(&tid).and_then(|t| t.attr(p.attr)).unwrap_or_else(|| ds.attr_default(p.attr));
                    AttrSlot::new(base)
                })
                .mods
                .push(p.m);
        }
    }

    // ------------------------------------------------------------------ system 3: local effects
    pub fn local_effects(&mut self) {
        let ds = self.ds;
        let (a, ef) = (&ds.a, &ds.e);
        let mut pend: Vec<PendingMod> = Vec::with_capacity(4096);
        let mut tg: Vec<Entity> = Vec::with_capacity(64);
        let ship = self.ship;
        let v = self.views();
        for &e in &self.order {
            let it = v.item(e);
            if it.kind == Kind::Projected {
                continue;
            }
            if self.is_structure && matches!(it.kind, Kind::Drone | Kind::Implant | Kind::Booster) {
                continue; // structures ignore pilot implants/boosters and cannot use drones
            }
            let state = v.state(e);
            let src_cat = it.category;
            let side_effects =
                if it.kind == Kind::Booster { self.world.get::<&SideEffects>(e).ok().map(|s| s.0.clone()).unwrap_or_default() } else { Vec::new() };
            let abilities = if it.kind == Kind::Fighter { self.world.get::<&FighterAbilities>(e).ok().map(|s| s.0.clone()) } else { None };
            for &(eid, _) in v.effects(e) {
                if eid == ef.skill_effect {
                    continue;
                }
                let Some(eff) = ds.effects.get(&eid) else { continue };
                if self.is_structure
                    && it.kind == Kind::Skill
                    && !ef.structure_skill_ok.contains(&eid)
                    && !eff.mods.iter().all(|m| m.domain == Domain::Item)
                {
                    continue;
                }
                if eff.fitting_usage_chance_attr.is_some() && !side_effects.contains(&eid) {
                    continue; // booster side effects only when selected
                }
                if it.kind == Kind::Fighter && eff.category != 0 {
                    if !abilities.as_ref().map(|l| l.contains(&eid)).unwrap_or(false) {
                        continue;
                    }
                }
                if !state_ok(eff.category, state) {
                    continue;
                }
                // ---- special handlers (effects without modifierInfo in the SDE)
                if eid == ef.afterburner || eid == ef.mwd {
                    self.pending(ship, a.mass, 2, Src::Attr { e, attr: a.mass_addition }, src_cat, &mut pend);
                    self.pending(ship, a.max_velocity, 4, Src::Prop { module: e, ship }, src_cat, &mut pend);
                    if eid == ef.mwd {
                        self.pending(ship, a.sig, 6, Src::Attr { e, attr: a.sig_bonus }, src_cat, &mut pend);
                    }
                    continue;
                }
                if eid == ef.mjd {
                    // MJD sig bloom is not stacking-penalised (unlike the MWD's)
                    self.pending(ship, a.sig, 6, Src::Attr { e, attr: a.sig_bonus_percent }, 6, &mut pend);
                    continue;
                }
                if eid == ef.slot_mod {
                    for (t, s) in [(a.hi_slots, a.hi_slot_mod), (a.med_slots, a.med_slot_mod), (a.low_slots, a.low_slot_mod)] {
                        self.pending(ship, t, 2, Src::Attr { e, attr: s }, src_cat, &mut pend);
                    }
                    continue;
                }
                if eid == ef.hardpoint_mod {
                    for (t, s) in [(a.turret_slots, a.turret_hp_mod), (a.launcher_slots, a.launcher_hp_mod)] {
                        self.pending(ship, t, 2, Src::Attr { e, attr: s }, src_cat, &mut pend);
                    }
                    continue;
                }
                for m in &eff.mods {
                    if m.func == Func::EffectStopper || m.op == 9 || matches!(m.domain, Domain::TargetId | Domain::Target) {
                        continue;
                    }
                    // EXCT convention: skill filter 0 = the type owning the effect (skill self-bonuses)
                    let extra = if m.extra == 0 && matches!(m.func, Func::LocationRequiredSkill | Func::OwnerRequiredSkill) { it.type_id } else { m.extra };
                    self.targets(&v, e, m.func, m.domain, extra, &mut tg);
                    // Bastion hull resists are not stacking penalised in game; SDE marks the attrs non-stackable
                    let cat = if eid == ef.bastion && HULL_RESONANCES.contains(&m.modified) { 6 } else { src_cat };
                    for &t in &tg {
                        self.pending(t, m.modified, m.op, Src::Attr { e, attr: m.modifying }, cat, &mut pend);
                    }
                }
            }
        }
        self.apply(pend);
    }

    // ------------------------------------------------------------------ system 4: projected effects
    pub fn projected_effects(&mut self) {
        let ds = self.ds;
        let a = &ds.a;
        let ship = self.ship;
        let mut pend = Vec::new();
        let v = self.views();
        let sources: Vec<Entity> = self.order.iter().copied().filter(|&e| v.item(e).kind == Kind::Projected).collect();
        let mut specials: Vec<(Entity, Special)> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        for e in sources {
            let it = v.item(e);
            let state = v.state(e);
            let dist = self.world.get::<&Distance>(e).map(|d| d.0).unwrap_or(None);
            for &(eid, _) in v.effects(e) {
                let Some(eff) = ds.effects.get(&eid) else { continue };
                if (eff.category != 2 && eff.category != 3) || state < State::Active {
                    continue;
                }
                let (factor, resist) = {
                    let c = self.calc();
                    let opt = eff.range_attr.filter(|&x| c.has(e, x)).map(|x| c.base(e, x)).unwrap_or(0.0);
                    let fo = eff.falloff_attr.filter(|&x| c.has(e, x)).map(|x| c.base(e, x)).unwrap_or(0.0);
                    let resist = eff.resistance_attr.unwrap_or_else(|| if c.has(e, a.remote_resistance_id) { c.base(e, a.remote_resistance_id) as u32 } else { 0 });
                    (crate::stats::range_factor(opt, fo, dist, true), resist)
                };
                let push = |fit: &Fit, target_attr: u32, src_attr: u32, op: i32, pend: &mut Vec<PendingMod>| {
                    let mul = op == 4 || op == 0;
                    fit.pending(ship, target_attr, op, Src::Projected { e, attr: src_attr, factor, target: ship, resist, mul }, it.category, pend);
                };
                if !eff.mods.is_empty() {
                    for m in &eff.mods {
                        if matches!(m.domain, Domain::TargetId | Domain::Target | Domain::Ship) && m.func == Func::Item {
                            push(self, m.modified, m.modifying, m.op, &mut pend);
                        }
                    }
                    continue;
                }
                let name = eff.name.as_str();
                if name.starts_with("remoteWebifier") || name == "structureModuleEffectStasisWebifier" {
                    push(self, a.max_velocity, a.speed_factor, 6, &mut pend);
                } else if name.starts_with("remoteTargetPaint") || name == "structureModuleEffectTargetPainter" {
                    push(self, a.sig, a.sig_bonus, 6, &mut pend);
                } else if name.starts_with("remoteSensorDamp") || name == "structureModuleEffectRemoteSensorDampener" {
                    push(self, a.max_target_range, a.max_target_range_bonus, 6, &mut pend);
                    push(self, a.scan_resolution, a.scan_resolution_bonus, 6, &mut pend);
                } else if name.starts_with("remoteSensorBoost") {
                    push(self, a.max_target_range, a.max_target_range_bonus, 6, &mut pend);
                    push(self, a.scan_resolution, a.scan_resolution_bonus, 6, &mut pend);
                    for k in 0..4 {
                        push(self, a.sensor[k], a.sensor_percent[k], 6, &mut pend);
                    }
                } else if let Some(sp) = self.incoming_special(e, name, resist, dist) {
                    specials.extend(sp);
                } else if !DAMAGE_EFFECTS.contains(&name) {
                    warnings.push(format!("projected effect '{name}' not modelled yet"));
                }
            }
        }
        self.apply(pend);
        drop(v);
        self.warnings.extend(warnings);
        for (e, sp) in specials {
            match sp {
                Special::Rep(r) => self.world.insert_one(e, r).unwrap(),
                Special::Drain(d) => self.world.insert_one(e, d).unwrap(),
            }
        }
    }

    /// Projected effects that feed tank / capacitor instead of attributes (remote reps, cap transfer, neuts, nos).
    fn incoming_special(&self, e: Entity, name: &str, resist: u32, dist: Option<f64>) -> Option<Vec<(Entity, Special)>> {
        let ds = self.ds;
        let a = &ds.a;
        let c = self.calc();
        let v = self.views();
        let base = |attr: u32| if c.has(e, attr) { c.base(e, attr) } else { 0.0 };
        let falloff_factor = || crate::stats::range_factor(base(a.max_range), base(a.falloff_effectiveness), dist, true);
        let gate = |opt: f64| if opt < dist.unwrap_or(0.0) { 0.0 } else { 1.0 };
        let no_assist = c.has(self.ship, a.disallow_assistance) && c.base(self.ship, a.disallow_assistance) != 0.0;
        let rep = |layer: u8, amount_attr: u32, mult: f64, factor: f64| {
            if no_assist { vec![] } else { vec![(e, Special::Rep(IncomingRep { layer, amount_attr, mult, factor }))] }
        };
        let drain = |amount_attr: u32, duration_attr: u32, factor: f64, sign: f64| {
            vec![(e, Special::Drain(IncomingDrain { amount_attr, duration_attr, factor, resist, sign }))]
        };
        let paste = self
            .world
            .get::<&Fitted>(e)
            .ok()
            .and_then(|f| f.charge)
            .map(|ch| ds.types[&v.item(ch).type_id].name == "Nanite Repair Paste")
            .unwrap_or(false);
        Some(match name {
            "shipModuleRemoteShieldBooster" | "shipModuleAncillaryRemoteShieldBooster" => rep(0, a.shield_bonus, 1.0, falloff_factor()),
            "shipModuleRemoteArmorRepairer" | "ShipModuleRemoteArmorMutadaptiveRepairer" => rep(1, a.armor_dmg_amount, 1.0, falloff_factor()),
            "shipModuleAncillaryRemoteArmorRepairer" => rep(1, a.armor_dmg_amount, if paste { 3.0 } else { 1.0 }, falloff_factor()),
            "shipModuleRemoteHullRepairer" => rep(2, a.structure_dmg_amount, 1.0, falloff_factor()),
            "npcEntityRemoteShieldBooster" => rep(0, a.shield_bonus, 1.0, gate(base(a.max_range))),
            "npcEntityRemoteArmorRepairer" => rep(1, a.armor_dmg_amount, 1.0, gate(base(a.max_range))),
            "npcEntityRemoteHullRepairer" => rep(2, a.structure_dmg_amount, 1.0, gate(base(a.max_range))),
            "shipModuleRemoteCapacitorTransmitter" => {
                if no_assist { vec![] } else { drain(a.power_transfer, a.duration, gate(base(a.max_range)), -1.0) }
            }
            "energyNeutralizerFalloff" => drain(a.neut_amount, a.duration, falloff_factor(), 1.0),
            "energyNosferatuFalloff" => drain(a.power_transfer, a.duration, falloff_factor(), 1.0),
            "structureEnergyNeutralizerFalloff" => drain(a.neut_amount, a.duration, 1.0, 1.0),
            "entityEnergyNeutralizerFalloff" => drain(a.neut_amount, a.neut_duration, gate(base(a.neut_range)), 1.0),
            _ => return None,
        })
    }

    // ------------------------------------------------------------------ system 5: fleet buffs
    fn buff_mods(&self, v: &Views, id: u32, src: Src, pend: &mut Vec<PendingMod>, tg: &mut Vec<Entity>) {
        let Some(info) = self.ds.dbuffs.get(&id) else { return };
        let op = info.op;
        let ship = self.ship;
        for &at in &info.item {
            self.pending(ship, at, op, src, 0, pend);
        }
        for &at in &info.location {
            self.targets(v, ship, Func::Location, Domain::Ship, 0, tg);
            for &t in tg.iter() {
                self.pending(t, at, op, src, 0, pend);
            }
        }
        for &(at, g) in &info.location_group {
            self.targets(v, ship, Func::LocationGroup, Domain::Ship, g, tg);
            for &t in tg.iter() {
                self.pending(t, at, op, src, 0, pend);
            }
        }
        for &(at, s) in &info.location_skill {
            self.targets(v, ship, Func::LocationRequiredSkill, Domain::Ship, s, tg);
            for &t in tg.iter() {
                self.pending(t, at, op, src, 0, pend);
            }
        }
    }

    /// (buff id, value) of every active command burst on this fit (values read after local effects).
    pub fn burst_buffs(&self) -> Vec<(u32, f64)> {
        let ds = self.ds;
        let c = self.calc();
        let v = self.views();
        let mut out = Vec::new();
        for &m in &self.modules {
            if v.state(m) < State::Active {
                continue;
            }
            for k in 0..4 {
                let ida = ds.a.warfare_id[k];
                let id = if ida != 0 && c.has(m, ida) { c.get(m, ida) as u32 } else { 0 };
                if id != 0 {
                    out.push((id, c.get(m, ds.a.warfare_value[k])));
                }
            }
        }
        out
    }

    /// Fleet boosts: explicit buffs, this fit's own bursts and the bursts of every booster fit
    /// (each booster is its own world run through spawn/index/local_effects). Per buff id the strongest
    /// value wins (largest magnitude, first seen on ties - Pyfa's commandBonuses rule).
    pub fn fleet_buffs(&mut self, req: &FitRequest) {
        let ds = self.ds;
        let mut agg: Vec<(u32, f64)> = Vec::new();
        let add = |id: u32, v: f64, agg: &mut Vec<(u32, f64)>| match agg.iter_mut().find(|x| x.0 == id) {
            Some(x) => {
                if x.1.abs() < v.abs() {
                    x.1 = v
                }
            }
            None => agg.push((id, v)),
        };
        // explicit buffs: aggregated with the dbuff's own aggregate mode, and they override fleet/own bursts
        let mut explicit: Vec<(u32, f64)> = Vec::new();
        for b in &req.fleet.buffs {
            let Some(info) = ds.dbuffs.get(&b.buff_id) else {
                self.warnings.push(format!("unknown warfare buff {}", b.buff_id));
                continue;
            };
            match explicit.iter_mut().find(|x| x.0 == b.buff_id) {
                Some(x) => x.1 = if info.aggregate.as_deref() == Some("Minimum") { x.1.min(b.value) } else { x.1.max(b.value) },
                None => explicit.push((b.buff_id, b.value)),
            }
        }
        for (id, v) in self.burst_buffs() {
            add(id, v, &mut agg);
        }
        for (i, bf) in req.fleet.booster_fits.iter().enumerate() {
            let mut breq = bf.clone();
            breq.fleet.booster_fits.clear();
            match Fit::spawn(ds, &breq) {
                Ok(mut sub) => {
                    sub.build_index();
                    sub.local_effects();
                    for (id, v) in sub.burst_buffs() {
                        add(id, v, &mut agg);
                    }
                }
                Err(e) => self.warnings.push(format!("fleet/booster_fits/{i}: {} {}", e.code, e.message)),
            }
        }
        agg.retain(|x| !explicit.iter().any(|y| y.0 == x.0));
        agg.extend(explicit);
        agg.sort_by_key(|x| x.0);
        let mut pend = Vec::new();
        let mut tg = Vec::new();
        let views = self.views();
        for (id, val) in agg {
            self.buff_mods(&views, id, Src::Const(val), &mut pend, &mut tg);
        }
        drop(views);
        self.apply(pend);
    }

    // ------------------------------------------------------------------ system 6: reactive armor hardener
    /// Simulates RAH cycles against the incoming damage pattern until the resist profile loops, averages the
    /// loop and applies it (same observable behaviour as Pyfa). `options.rah = "disable"` keeps base resists.
    pub fn rah_adapt(&mut self, req: &FitRequest) {
        let ds = self.ds;
        let eid = ds.e.rah;
        if eid == 0 {
            return;
        }
        let attrs = ds.a.res_armor;
        let v = self.views();
        let rahs: Vec<Entity> = self.modules.iter().copied().filter(|&m| v.state(m) >= State::Active && v.has_effect(m, eid)).collect();
        let disable = req.options.rah.as_deref() == Some("disable");
        let dp = req.damage_pattern.unwrap_or(Resists { em: 25.0, thermal: 25.0, kinetic: 25.0, explosive: 25.0 });
        let pattern = [dp.em, dp.thermal, dp.kinetic, dp.explosive];
        let ship = self.ship;
        for m in rahs {
            let res = {
                let c = self.calc();
                let mut res: [f64; 4] = std::array::from_fn(|k| c.get(m, attrs[k]));
                if !disable {
                    let base: [f64; 4] = std::array::from_fn(|k| pattern[k] * c.get(ship, attrs[k]));
                    let shift = c.get(m, ds.a.resistance_shift) / 100.0;
                    res = rah_simulate(res, base, shift);
                }
                res
            };
            let cat = v.item(m).category;
            let mut pend = Vec::new();
            for k in 0..4 {
                if !disable {
                    self.pending(m, attrs[k], 7, Src::Const(res[k]), cat, &mut pend);
                }
                self.pending(ship, attrs[k], 0, Src::Const(res[k]), cat, &mut pend);
            }
            self.apply(pend);
        }
    }

    /// The whole schedule.
    pub fn run(ds: &'a Dataset, req: &FitRequest) -> Result<Fit<'a>, EngineError> {
        let mut fit = Fit::spawn(ds, req)?;
        fit.build_index();
        fit.local_effects();
        fit.projected_effects();
        fit.fleet_buffs(req);
        fit.rah_adapt(req);
        Ok(fit)
    }
}

fn rah_simulate(mut res: [f64; 4], base: [f64; 4], shift: f64) -> [f64; 4] {
    let mut cycles: Vec<[f64; 4]> = Vec::new();
    let mut loop_start: Option<usize> = None;
    for _ in 0..50 {
        // in-game tie order em, explosive, kinetic, thermal; stable sort by damage taken
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
            loop_start = Some(i);
            break;
        }
        cycles.push(res);
    }
    let start = loop_start.unwrap_or(cycles.len().saturating_sub(20));
    let lp = &cycles[start..];
    if !lp.is_empty() {
        for k in 0..4 {
            res[k] = ((lp.iter().map(|v| v[k]).sum::<f64>() / lp.len() as f64) * 1000.0).round() / 1000.0;
        }
    }
    res
}

/// Pyfa default: standard attack on; other abilities (except MWD/evasive/MJD) on only if they come before
/// the standard attack in effect id order.
fn default_fighter_abilities(ds: &Dataset, t: &TypeInfo) -> Vec<u32> {
    let mut ids: Vec<u32> = t.effects.iter().map(|(e, _)| *e).collect();
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
