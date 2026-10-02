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
    pub fn fitted(&self, e: Entity) -> Fitted {
        *self.world.get::<&Fitted>(e).unwrap()
    }
    pub fn squad(&self, e: Entity) -> Squad {
        *self.world.get::<&Squad>(e).unwrap()
    }
    pub fn effects(&self, e: Entity) -> Vec<(u32, bool)> {
        if let Ok(m) = self.world.get::<&Mutated>(e) {
            return m.effects.clone();
        }
        self.ds.types[&self.item(e).type_id].effects.clone()
    }
    pub fn has_effect(&self, e: Entity, eid: u32) -> bool {
        if eid == 0 {
            return false;
        }
        if let Ok(m) = self.world.get::<&Mutated>(e) {
            return m.effects.iter().any(|x| x.0 == eid);
        }
        self.ds.types[&self.item(e).type_id].has_effect(eid)
    }
    fn req_skills(&self, e: Entity) -> Vec<u32> {
        if let Ok(m) = self.world.get::<&Mutated>(e) {
            return m.req_skills.clone();
        }
        self.ds.types[&self.item(e).type_id].req_skills.clone()
    }
    pub fn calc(&self) -> Calc<'_> {
        Calc::new(self.ds, &self.world)
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
                            let e = fit.spawn_item(m.type_id, Kind::Projected, Loc::Nowhere, m.state.unwrap_or(State::Active), &path)?;
                            fit.world.insert_one(e, Distance(p.distance_m)).unwrap();
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

    // ------------------------------------------------------------------ system 2: index
    pub fn build_index(&mut self) {
        let mut ix = Index::default();
        for &e in &self.order {
            let it = self.item(e);
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
            for s in self.req_skills(e) {
                ix.by_skill.entry(s).or_default().push((e, it.loc, it.owned, it.kind == Kind::Skill));
            }
        }
        self.index = ix;
    }

    fn targets(&self, src: Entity, func: Func, domain: Domain, extra: u32, out: &mut Vec<Entity>) {
        out.clear();
        let ix = &self.index;
        match domain {
            Domain::Item => {
                if func == Func::Item {
                    out.push(src)
                }
            }
            Domain::Other => {
                if let Ok(f) = self.world.get::<&Fitted>(src) {
                    if let Some(c) = f.charge {
                        out.push(c);
                        return;
                    }
                }
                if let Ok(p) = self.world.get::<&LoadedIn>(src) {
                    out.push(p.0)
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
    fn apply(&mut self, pend: Vec<PendingMod>) {
        let ds = self.ds;
        let mut view = self.world.view_mut::<&mut Attrs>();
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
        for &e in &self.order {
            let it = self.item(e);
            if it.kind == Kind::Projected {
                continue;
            }
            if self.is_structure && matches!(it.kind, Kind::Drone | Kind::Implant | Kind::Booster) {
                continue; // structures ignore pilot implants/boosters and cannot use drones
            }
            let state = self.state(e);
            let src_cat = it.category;
            let side_effects = self.world.get::<&SideEffects>(e).ok().map(|s| s.0.clone()).unwrap_or_default();
            let abilities = self.world.get::<&FighterAbilities>(e).ok().map(|s| s.0.clone());
            for (eid, _) in self.effects(e) {
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
                    self.targets(e, m.func, m.domain, extra, &mut tg);
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
        let sources: Vec<Entity> = self.order.iter().copied().filter(|&e| self.item(e).kind == Kind::Projected).collect();
        for e in sources {
            let it = self.item(e);
            let state = self.state(e);
            let dist = self.world.get::<&Distance>(e).map(|d| d.0).unwrap_or(None);
            for (eid, _) in self.effects(e) {
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
                } else if name.starts_with("remoteSensorDamp") || name == "structureModuleEffectRemoteSensorDampener" || name.starts_with("remoteSensorBoost") {
                    push(self, a.max_target_range, a.max_target_range_bonus, 6, &mut pend);
                    push(self, a.scan_resolution, a.scan_resolution_bonus, 6, &mut pend);
                } else {
                    self.warnings.push(format!("projected effect '{name}' not modelled yet"));
                }
            }
        }
        self.apply(pend);
    }

    // ------------------------------------------------------------------ system 5: fleet buffs
    fn buff_mods(&self, id: u32, src: Src, pend: &mut Vec<PendingMod>, tg: &mut Vec<Entity>) {
        let Some(info) = self.ds.dbuffs.get(&id) else { return };
        let op = info.op;
        let ship = self.ship;
        for &at in &info.item {
            self.pending(ship, at, op, src, 0, pend);
        }
        for &at in &info.location {
            self.targets(ship, Func::Location, Domain::Ship, 0, tg);
            for &t in tg.iter() {
                self.pending(t, at, op, src, 0, pend);
            }
        }
        for &(at, g) in &info.location_group {
            self.targets(ship, Func::LocationGroup, Domain::Ship, g, tg);
            for &t in tg.iter() {
                self.pending(t, at, op, src, 0, pend);
            }
        }
        for &(at, s) in &info.location_skill {
            self.targets(ship, Func::LocationRequiredSkill, Domain::Ship, s, tg);
            for &t in tg.iter() {
                self.pending(t, at, op, src, 0, pend);
            }
        }
    }

    pub fn fleet_buffs(&mut self, req: &FitRequest) {
        let ds = self.ds;
        let mut agg: Vec<(u32, f64)> = Vec::new();
        for b in &req.fleet.buffs {
            let Some(info) = ds.dbuffs.get(&b.buff_id) else {
                self.warnings.push(format!("unknown warfare buff {}", b.buff_id));
                continue;
            };
            match agg.iter_mut().find(|x| x.0 == b.buff_id) {
                Some(x) => {
                    x.1 = if info.aggregate.as_deref() == Some("Minimum") { x.1.min(b.value) } else { x.1.max(b.value) };
                }
                None => agg.push((b.buff_id, b.value)),
            }
        }
        agg.sort_by_key(|x| x.0);
        let mut pend = Vec::new();
        let mut tg = Vec::new();
        for (id, v) in agg {
            self.buff_mods(id, Src::Const(v), &mut pend, &mut tg);
        }
        // local command bursts: warfareBuffNID / warfareBuffNValue read (modified) from active modules
        let explicit: Vec<u32> = req.fleet.buffs.iter().map(|b| b.buff_id).collect();
        let mut local: Vec<(u32, Src)> = Vec::new();
        {
            let c = self.calc();
            for &m in &self.modules {
                if self.state(m) < State::Active {
                    continue;
                }
                for k in 0..4 {
                    let ida = ds.a.warfare_id[k];
                    let id = if ida != 0 && c.has(m, ida) { c.get(m, ida) as u32 } else { 0 };
                    if id == 0 || explicit.contains(&id) {
                        continue;
                    }
                    local.push((id, Src::Attr { e: m, attr: ds.a.warfare_value[k] }));
                }
            }
        }
        for (id, src) in local {
            self.buff_mods(id, src, &mut pend, &mut tg);
        }
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
        let rahs: Vec<Entity> = self.modules.iter().copied().filter(|&m| self.state(m) >= State::Active && self.has_effect(m, eid)).collect();
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
            let cat = self.item(m).category;
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
