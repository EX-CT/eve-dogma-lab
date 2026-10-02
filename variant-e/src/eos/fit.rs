//! Fit construction from a FitRequest and Pyfa's calculation loop
//! (eos/saveddata/fit.py calculateModifiedAttributes + the per-item calculateModifiedAttributes of
//! ship.py, character.py, module.py, drone.py, fighter.py, implant.py, booster.py, mode.py).
//! GPL-3.0-or-later.
use super::cx::*;
use super::mad::Mad;
use crate::data::{Dataset, TypeInfo};
use crate::generated::effects;
use crate::request::{FitRequest, Mutation, SlotReq, StateReq};
use rustc_hash::FxHashMap;
use std::sync::OnceLock;

pub struct BuildError {
    pub code: &'static str,
    pub message: String,
    pub path: String,
}

/// effect metadata indexed directly by effect id (dense table instead of a hash map)
pub struct MetaTable(Vec<u16>);

impl MetaTable {
    #[inline]
    pub fn get(&self, e: &u32) -> Option<&'static EffMeta> {
        match self.0.get(*e as usize) {
            Some(&i) if i != u16::MAX => Some(&effects::META[i as usize]),
            _ => None,
        }
    }
}

pub fn meta() -> &'static MetaTable {
    static M: OnceLock<MetaTable> = OnceLock::new();
    M.get_or_init(|| {
        let n = effects::META.iter().map(|m| m.id as usize + 1).max().unwrap_or(0);
        let mut v = vec![u16::MAX; n];
        for (i, m) in effects::META.iter().enumerate() {
            v[m.id as usize] = i as u16;
        }
        MetaTable(v)
    })
}

pub fn extra_attr(name: &str) -> u32 {
    effects::EXTRA_ATTRS.iter().find(|x| x.1 == name).map(|x| x.0).unwrap_or(0)
}

/// Slot from the item's slot effect (Pyfa Module.calculateSlot)
pub fn infer_slot(t: &TypeInfo) -> Option<SlotReq> {
    for (e, _) in &t.effects {
        match *e {
            12 => return Some(SlotReq::High),
            13 => return Some(SlotReq::Mid),
            11 => return Some(SlotReq::Low),
            2663 => return Some(SlotReq::Rig),
            3772 => return Some(SlotReq::Subsystem),
            6306 => return Some(SlotReq::Service),
            _ => {}
        }
    }
    None
}

impl<'a> Fit<'a> {
    fn new_item(&mut self, type_id: u32, kind: Kind, path: &str) -> Result<It, BuildError> {
        let ds: &'a Dataset = self.ds;
        let t = ds.types.get(&type_id).ok_or_else(|| BuildError {
            code: "UNKNOWN_TYPE",
            message: format!("unknown type_id {type_id}"),
            path: path.to_string(),
        })?;
        self.items.push(Item {
            t,
            kind,
            mad: Mad::new(&t.attrs),
            charge: NONE,
            parent: NONE,
            state: ONLINE,
            amount: 1,
            amount_active: 0,
            level: 0,
            reload_time: None,
            force_reload: None,
            proj_range: None,
            slot: None,
            req_index: 0,
            spool: None,
            effects: &t.effect_ids,
            abilities: Vec::new(),
            side_effects: Vec::new(),
            active: true,
        });
        Ok(self.items.len() - 1)
    }

    /// Pyfa MutatedMixin: attributes = {**base.attributes, **mutated.attributes}; mutators start at the base
    /// value, take the rolled value, clamped to the mutaplasmid range.
    fn mutate(&mut self, it: It, m: &Mutation) -> Result<(), BuildError> {
        let ds: &'a Dataset = self.ds;
        let Some(base) = ds.types.get(&m.base_type_id) else {
            return Err(BuildError { code: "UNKNOWN_TYPE", message: format!("unknown base_type_id {}", m.base_type_id), path: "mutation".into() });
        };
        let muta = m.mutaplasmid_type_id.and_then(|id| ds.mutaplasmids.get(&id));
        if let Some(mu) = muta {
            if let Some(out) = mu.mapping.iter().find(|x| x.0.contains(&m.base_type_id)).map(|x| x.1) {
                if let Some(t) = ds.types.get(&out) {
                    let item = &mut self.items[it];
                    item.t = t;
                    item.mad = Mad::new(&t.attrs);
                    item.effects = &t.effect_ids;
                }
            }
        }
        let item = &mut self.items[it];
        for (a, v) in &base.attrs {
            if !item.mad.in_original(*a) {
                item.mad.set_over(*a, *v);
            }
        }
        if let Some(mu) = muta {
            for (a, lo, hi) in &mu.attrs {
                let bv = base.attr(*a).unwrap_or(0.0);
                let mut v = m.attributes.get(&a.to_string()).copied().unwrap_or(bv);
                if bv == 0.0 {
                    v = 0.0;
                } else {
                    let (lo, hi) = (py_round_n(*lo, 3), py_round_n(*hi, 3));
                    let r = v / bv;
                    if !(lo <= r && r <= hi) {
                        let (a1, b1) = (lo * bv, hi * bv);
                        v = v.max(a1.min(b1)).min(a1.max(b1));
                    }
                }
                item.mad.set_over(*a, v);
            }
        }
        Ok(())
    }

    fn add_fighter(&mut self, f: &crate::request::FighterReq, kind: Kind, path: &str) -> Result<It, BuildError> {
        let ds: &'a Dataset = self.ds;
        let idx = self.new_item(f.type_id, kind, path)?;
        let maxsq = self.attr(idx, ds.attr_id("fighterSquadronMaxSize")) as u32;
        let it = &mut self.items[idx];
        it.amount = match f.quantity {
            Some(q) if q > 0 && q < maxsq => q,
            _ => maxsq,
        };
        it.active = f.active;
        // Pyfa Fighter.__init__ default abilities
        let m = meta();
        let mut std_seen = false;
        let mut ab = Vec::new();
        for &e in it.effects {
            let Some(em) = m.get(&e) else {
                ab.push((e, false));
                continue;
            };
            let on = if em.name == "fighterAbilityAttackM" {
                std_seen = true;
                true
            } else {
                !std_seen && em.name != "fighterAbilityMicroWarpDrive" && em.name != "fighterAbilityEvasiveManeuvers"
            };
            ab.push((e, on));
        }
        if let Some(list) = &f.abilities {
            for a in ab.iter_mut() {
                a.1 = list.contains(&a.0);
            }
        }
        it.abilities = ab;
        let bomb = self.attr(idx, ds.attr_id("fighterAbilityLaunchBombType")) as u32;
        if bomb != 0 && ds.types.contains_key(&bomb) {
            let ci = self.new_item(bomb, Kind::Charge, path)?;
            self.items[ci].parent = idx;
            self.items[idx].charge = ci;
        }
        Ok(idx)
    }

    fn item_is_type(&self, it: It, t: u16) -> bool {
        let m = meta();
        self.items[it].effects.iter().any(|e| m.get(e).map(|x| x.is(t)).unwrap_or(false))
    }

    /// Module.isValidState
    fn valid_state(&self, it: It, st: i8) -> bool {
        if st >= ACTIVE && (!self.item_is_type(it, T_ACTIVE) || self.attr(it, self.ds.attr_id("activationBlocked")) > 0.0) {
            return false;
        }
        if st == OVERHEATED && !self.item_is_type(it, T_OVERHEAT) {
            return false;
        }
        true
    }

    pub fn build(ds: &'a Dataset, req: &FitRequest) -> Result<Fit<'a>, BuildError> {
        let mut fit = Fit {
            ds,
            items: Vec::with_capacity(600),
            ship: NONE,
            chr: NONE,
            mode: NONE,
            modules: Vec::new(),
            drones: Vec::new(),
            fighters: Vec::new(),
            implants: Vec::new(),
            boosters: Vec::new(),
            skills: Vec::new(),
            skill_by_type: FxHashMap::default(),
            default_level: req.character.skills.default_level.unwrap_or(0).min(5),
            proj_modules: Vec::new(),
            proj_drones: Vec::new(),
            proj_fighters: Vec::new(),
            structure: false,
            factor_reload: req.options.factor_reload,
            pilot_sec: req.character.security_status,
            sys_sec: 2,
            command_bonuses: Vec::new(),
            damage_pattern: None,
            extra_drains: Vec::new(),
            rr: Vec::new(),
            gang_sink: None,
            explicit_buff_ids: Vec::new(),
            ecm: Vec::new(),
            rep_afflictions: Vec::new(),
            warnings: Vec::new(),
            ctx_flags: 0,
            effect: 0,
            proj_range: None,
            modifier: NONE,
            meta: meta(),
        };
        let ship = fit.new_item(req.ship.type_id, Kind::Ship, "/ship/type_id")?;
        fit.ship = ship;
        fit.structure = fit.items[ship].t.category == 65;
        // Ship.EXTRA_ATTRIBUTES (fit.extraAttributes is the ship's attribute dict)
        for (n, v) in [
            ("armorRepair", 0.0),
            ("armorRepairPreSpool", 0.0),
            ("armorRepairFullSpool", 0.0),
            ("hullRepair", 0.0),
            ("shieldRepair", 0.0),
            ("maxTargetsLockedFromSkills", 2.0),
            ("droneControlRange", 20000.0),
            ("cloaked", 0.0),
        ] {
            let a = extra_attr(n);
            if a != 0 {
                fit.items[ship].mad.set_over(a, v);
            }
        }
        let mad = ds.attr_id("maxActiveDrones");
        fit.items[ship].mad.set_over(mad, 0.0);
        fit.sys_sec = match req.environment.system_security.as_deref().map(|s| s.to_lowercase()) {
            Some(s) if s == "hisec" || s == "highsec" => 0,
            Some(s) if s == "lowsec" => 1,
            Some(s) if s == "wspace" || s == "wormhole" => 3,
            _ => 2,
        };
        fit.damage_pattern = if req.options.rah.as_deref() == Some("disable") {
            None
        } else {
            let d = req.damage_pattern.map(|p| [p.em, p.thermal, p.kinetic, p.explosive]).unwrap_or([25.0; 4]);
            Some(d)
        };
        // character: every skill at the default level, then explicit levels
        let mut levels: FxHashMap<u32, u8> = FxHashMap::default();
        for (k, v) in &req.character.skills.levels {
            let id = k.parse::<u32>().ok().or_else(|| ds.types.by_name(k));
            if let Some(id) = id {
                levels.insert(id, (*v).min(5));
            }
        }
        for &s in &ds.skills {
            let idx = fit.new_item(s, Kind::Skill, "/character/skills")?;
            fit.items[idx].level = levels.get(&s).copied().unwrap_or(fit.default_level);
            fit.skills.push(idx);
            fit.skill_by_type.insert(s, idx);
        }
        // T3D mode
        let mode_id = req.ship.mode_type_id.or_else(|| {
            let st = &fit.items[ship].t;
            if ds.group_name(st.group) != "Tactical Destroyer" && st.name != "Anhinga" {
                return None;
            }
            let lname = st.name.to_lowercase();
            let mut ids: Vec<u32> = ds.types.in_group(1306).filter(|t| t.name.to_lowercase().starts_with(&lname)).map(|t| t.id).collect();
            ids.sort();
            ids.first().copied()
        });
        if let Some(m) = mode_id {
            fit.mode = fit.new_item(m, Kind::Mode, "/ship/mode_type_id")?;
        }
        for (i, m) in req.modules.iter().enumerate() {
            let path = format!("/modules/{i}");
            let idx = fit.new_item(m.type_id, Kind::Module, &path)?;
            if let Some(mu) = &m.mutation {
                fit.mutate(idx, mu)?;
            }
            fit.items[idx].slot = m.slot.or_else(|| infer_slot(fit.items[idx].t));
            fit.items[idx].req_index = i;
            fit.items[idx].spool = m.spool;
            if let Some(c) = m.charge_type_id {
                let ct = ds.types.get(&c).ok_or_else(|| BuildError { code: "UNKNOWN_TYPE", message: format!("unknown charge_type_id {c}"), path: format!("{path}/charge_type_id") })?;
                if ct.category == 8 {
                    let ci = fit.new_item(c, Kind::Charge, &path)?;
                    fit.items[ci].parent = idx;
                    fit.items[idx].charge = ci;
                }
            }
            let want = match m.state.unwrap_or(StateReq::Online) {
                StateReq::Offline => OFFLINE,
                StateReq::Online => ONLINE,
                StateReq::Active => ACTIVE,
                StateReq::Overheated => OVERHEATED,
            };
            let st = if fit.valid_state(idx, want) { want } else { ONLINE };
            fit.items[idx].state = st;
            fit.modules.push(idx);
        }
        for (i, d) in req.drones.iter().enumerate() {
            let idx = fit.new_item(d.type_id, Kind::Drone, &format!("/drones/{i}"))?;
            if let Some(mu) = &d.mutation {
                fit.mutate(idx, mu)?;
            }
            let it = &mut fit.items[idx];
            it.amount = d.quantity;
            it.amount_active = d.active.unwrap_or(0).min(d.quantity);
            it.req_index = i;
            let mt = fit.attr(idx, ds.attr_id("entityMissileTypeID")) as u32;
            if mt != 0 && ds.types.contains_key(&mt) {
                let ci = fit.new_item(mt, Kind::Charge, &format!("/drones/{i}"))?;
                fit.items[ci].parent = idx;
                fit.items[idx].charge = ci;
            }
            fit.drones.push(idx);
        }
        for (i, f) in req.fighters.iter().enumerate() {
            let idx = fit.add_fighter(f, Kind::Fighter, &format!("/fighters/{i}"))?;
            fit.items[idx].req_index = i;
            fit.fighters.push(idx);
        }
        for (i, imp) in req.implants.iter().enumerate() {
            let idx = fit.new_item(*imp, Kind::Implant, &format!("/implants/{i}"))?;
            fit.items[idx].req_index = i;
            fit.implants.push(idx);
        }
        for (i, b) in req.boosters.iter().enumerate() {
            let idx = fit.new_item(b.type_id, Kind::Booster, &format!("/boosters/{i}"))?;
            fit.items[idx].req_index = i;
            fit.items[idx].side_effects = b.side_effects.clone();
            fit.boosters.push(idx);
        }
        for (i, p) in req.projected.iter().enumerate() {
            match p.kind.as_str() {
                "module" => {
                    let Some(m) = &p.module else { continue };
                    for _ in 0..p.amount.max(1) {
                        let idx = fit.new_item(m.type_id, Kind::ProjModule, &format!("/projected/{i}"))?;
                        if let Some(mu) = &m.mutation {
                            fit.mutate(idx, mu)?;
                        }
                        if let Some(c) = m.charge_type_id {
                            if ds.types.get(&c).map(|t| t.category == 8).unwrap_or(false) {
                                let ci = fit.new_item(c, Kind::Charge, &format!("/projected/{i}"))?;
                                fit.items[ci].parent = idx;
                                fit.items[idx].charge = ci;
                            }
                        }
                        let st = if fit.valid_state(idx, ACTIVE) { ACTIVE } else { ONLINE };
                        fit.items[idx].state = st;
                        fit.items[idx].proj_range = p.distance_m;
                        fit.items[idx].req_index = i;
                        fit.proj_modules.push(idx);
                    }
                }
                "drone" => {
                    let Some(d) = &p.drone else { continue };
                    let idx = fit.new_item(d.type_id, Kind::ProjDrone, &format!("/projected/{i}"))?;
                    let n = d.quantity.max(1) * p.amount.max(1);
                    fit.items[idx].amount = n;
                    fit.items[idx].amount_active = n;
                    fit.items[idx].proj_range = p.distance_m;
                    fit.items[idx].req_index = i;
                    fit.proj_drones.push(idx);
                }
                "fighter" => {
                    let Some(fr) = &p.fighter else { continue };
                    for _ in 0..p.amount.max(1) {
                        let idx = fit.add_fighter(fr, Kind::ProjFighter, &format!("/projected/{i}"))?;
                        fit.items[idx].proj_range = p.distance_m;
                        fit.items[idx].req_index = i;
                        fit.proj_fighters.push(idx);
                    }
                }
                "fit" => {}
                other => fit.warnings.push(format!("projected kind '{other}' not supported (index {i})")),
            }
        }
        for (i, e) in req.environment.effect_type_ids.iter().enumerate() {
            // Pyfa: system effects (beacons) are projected modules
            let idx = fit.new_item(*e, Kind::ProjModule, &format!("/environment/effect_type_ids/{i}"))?;
            fit.items[idx].state = ONLINE;
            fit.proj_modules.push(idx);
        }
        if false {
            fit.warnings.push("fleet.booster_fits: command fits are not applied yet (use fleet.buffs)".into());
        }
        for o in &req.overrides {
            for it in fit.items.iter_mut().filter(|it| it.t.id == o.type_id) {
                it.mad.set_over(o.attribute_id, o.value);
            }
        }
        Ok(fit)
    }

    fn run_effect(&mut self, eid: u32, me: It, ctx: u16, proj_range: Option<f64>, modifier: It) {
        self.ctx_flags = ctx;
        self.effect = eid;
        self.proj_range = proj_range;
        self.modifier = modifier;
        if !effects::run(self, eid, me) {
            super::custom::run(self, eid, me);
        }
    }

    /// Fit.calculateModifiedAttributes (local fit)
    pub fn calculate(&mut self, explicit_buffs: &[(u32, f64)]) {
        self.calc_inner(explicit_buffs, None);
    }

    /// Fit.calculateModifiedAttributes(targetFit, CalcType.COMMAND): local calc of a command fit, collecting the
    /// warfare bonuses its modules' gang effects add to the target fit.
    pub fn calculate_command(&mut self) -> Vec<CommandBonus> {
        let mut out = Vec::new();
        self.calc_inner(&[], Some(&mut out));
        out
    }

    fn calc_inner(&mut self, explicit_buffs: &[(u32, f64)], mut gang: Option<&mut Vec<CommandBonus>>) {
        for rt in [RT_EARLY, RT_NORMAL, RT_LATE] {
            self.calc_rt(rt, explicit_buffs, gang.as_deref_mut());
        }
    }

    /// one runtime pass of Fit.calculateModifiedAttributes
    pub fn calc_rt(&mut self, rt: u8, explicit_buffs: &[(u32, f64)], mut gang: Option<&mut Vec<CommandBonus>>) {
        let m = self.meta;
        {
            // (character, ship)
            for k in 0..self.skills.len() {
                let s = self.skills[k];
                let effs: &'a [u32] = self.items[s].effects;
                for &e in effs {
                    let Some(em) = m.get(&e) else { continue };
                    if em.run_time == rt && em.is(T_PASSIVE) && (!self.structure || em.is(T_STRUCTURE)) && em.active_by_default {
                        self.run_effect(e, s, Ctx::Skill as u16, None, s);
                    }
                }
            }
            let ship = self.ship;
            for &e in self.items[ship].effects {
                let Some(em) = m.get(&e) else { continue };
                if em.run_time == rt && em.is(T_PASSIVE) && em.active_by_default {
                    self.run_effect(e, ship, Ctx::Ship as u16, None, ship);
                }
            }
            if !self.structure {
                for k in 0..self.drones.len() {
                    let d = self.drones[k];
                    self.calc_drone(d, rt, false);
                }
            }
            for k in 0..self.fighters.len() {
                let f = self.fighters[k];
                self.calc_fighter(f, rt);
            }
            if !self.structure {
                for k in 0..self.boosters.len() {
                    let b = self.boosters[k];
                    for &e in self.items[b].effects {
                        let Some(em) = m.get(&e) else { continue };
                        if em.run_time == rt && (em.is(T_PASSIVE) || em.is(T_BOOSTERSIDEEFFECT)) {
                            if em.is(T_BOOSTERSIDEEFFECT) && !self.items[b].side_effects.contains(&e) {
                                continue;
                            }
                            self.run_effect(e, b, Ctx::Booster as u16, None, b);
                        }
                    }
                }
                for k in 0..self.implants.len() {
                    let i = self.implants[k];
                    for &e in self.items[i].effects {
                        let Some(em) = m.get(&e) else { continue };
                        if em.run_time == rt && em.is(T_PASSIVE) && em.active_by_default {
                            self.run_effect(e, i, Ctx::Implant as u16, None, i);
                        }
                    }
                }
            }
            for k in 0..self.modules.len() {
                let md = self.modules[k];
                self.calc_module(md, rt, false);
                if let Some(g) = gang.as_deref_mut() {
                    self.gang_sink = Some(std::mem::take(g));
                    self.calc_module_gang(md, rt);
                    *g = self.gang_sink.take().unwrap_or_default();
                }
            }
            // restricted: mode, projected drones, projected fighters, projected modules
            if self.mode != NONE {
                let md = self.mode;
                for &e in self.items[md].effects {
                    let Some(em) = m.get(&e) else { continue };
                    if em.run_time == rt && em.active_by_default {
                        self.run_effect(e, md, Ctx::Module as u16, None, md);
                    }
                }
            }
            for k in 0..self.proj_drones.len() {
                let d = self.proj_drones[k];
                self.calc_drone(d, rt, true);
            }
            for k in 0..self.proj_fighters.len() {
                let f = self.proj_fighters[k];
                self.calc_fighter_projected(f, rt);
            }
            for k in 0..self.proj_modules.len() {
                let md = self.proj_modules[k];
                self.calc_module(md, rt, true);
            }
            if rt == RT_NORMAL {
                // contract v1.4.2: explicit fleet.buffs override (aggregated per id by the buff's aggregate
                // mode); booster-fit values and the fit's own bursts for those ids are dropped
                let mut agg: Vec<(u32, f64)> = Vec::new();
                for &(id, v) in explicit_buffs {
                    let maxm = self.ds.dbuffs.get(&id).map(|b| b.aggregate_max).unwrap_or(true);
                    match agg.iter_mut().find(|x| x.0 == id) {
                        Some(x) => x.1 = if maxm { x.1.max(v) } else { x.1.min(v) },
                        None => agg.push((id, v)),
                    }
                }
                for (id, v) in agg {
                    if !self.explicit_buff_ids.contains(&id) {
                        self.explicit_buff_ids.push(id);
                    }
                    let b = CommandBonus { id, run_time: RT_NORMAL, value: v, thing: NONE, effect: 0 };
                    match self.command_bonuses.iter_mut().find(|x| x.id == id) {
                        Some(x) => *x = b,
                        None => self.command_bonuses.push(b),
                    }
                }
            }
            if gang.is_none() {
                self.run_command_boosts(rt);
            }
        }
    }

    /// Fit.__runProjectionEffects: `src` (a projected fit, calculated up to `rt`) projects onto self.
    /// Source items are mirrored into this fit with their current modified values.
    pub fn project_from(&mut self, src: &Fit<'a>, rt: u8, amount: u32, range: Option<f64>, mirror: &mut Vec<(It, It)>) {
        if mirror.is_empty() {
            let srcs: Vec<It> = src.drones.iter().chain(src.fighters.iter()).chain(src.modules.iter()).copied().collect();
            for si in srcs {
                let s = &src.items[si];
                let kind = match s.kind {
                    Kind::Drone => Kind::ProjDrone,
                    Kind::Fighter => Kind::Fighter,
                    _ => Kind::ProjModule,
                };
                self.items.push(Item {
                    t: s.t,
                    kind,
                    mad: Mad::new(&[]),
                    charge: NONE,
                    parent: NONE,
                    state: s.state,
                    amount: s.amount,
                    amount_active: s.amount_active,
                    level: 0,
                    reload_time: s.reload_time,
                    force_reload: s.force_reload,
                    proj_range: if kind == Kind::ProjModule { range } else { Some(0.0) },
                    slot: s.slot,
                    req_index: s.req_index,
                    spool: s.spool,
                    effects: s.effects,
                    abilities: s.abilities.clone(),
                    side_effects: Vec::new(),
                    active: s.active,
                });
                let ti = self.items.len() - 1;
                mirror.push((si, ti));
                if s.charge != NONE {
                    let c = &src.items[s.charge];
                    self.items.push(Item { t: c.t, kind: Kind::Charge, mad: Mad::new(&[]), charge: NONE, parent: ti, state: 0, amount: 1,
                        amount_active: 0, level: 0, reload_time: None, force_reload: None, proj_range: None, slot: None, req_index: 0,
                        spool: None, effects: c.effects, abilities: Vec::new(), side_effects: Vec::new(), active: true });
                    let ci = self.items.len() - 1;
                    self.items[ti].charge = ci;
                    mirror.push((s.charge, ci));
                }
            }
        }
        for &(si, ti) in mirror.iter() {
            let mut over: Vec<(u32, f64)> = Vec::new();
            let mut ids: Vec<u32> = src.items[si].t.attrs.iter().map(|x| x.0).collect();
            ids.extend(src.items[si].mad.over.iter().map(|x| x.0));
            ids.extend(src.items[si].mad.entries.keys().copied());
            ids.sort_unstable();
            ids.dedup();
            for a in ids {
                if let Some(v) = src.attr_opt(si, a) {
                    over.push((a, v));
                }
            }
            let it = &mut self.items[ti];
            it.mad = Mad::new(&[]);
            it.mad.over = over;
            it.reload_time = src.items[si].reload_time;
        }
        let m = self.meta;
        for &(_, ti) in mirror.clone().iter() {
            let kind = self.items[ti].kind;
            if kind == Kind::Charge {
                continue;
            }
            for _ in 0..amount {
                let pr = self.items[ti].proj_range;
                let effs: &'a [u32] = self.items[ti].effects;
                match kind {
                    Kind::ProjDrone => {
                        for &e in effs {
                            let Some(em) = m.get(&e) else { continue };
                            if em.run_time == rt && em.active_by_default && em.is(T_PROJECTED) {
                                let n = if em.grouped { 1 } else { self.items[ti].amount_active };
                                for _ in 0..n {
                                    self.run_effect(e, ti, Ctx::Projected as u16 | Ctx::Drone as u16, pr, ti);
                                }
                            }
                        }
                    }
                    Kind::Fighter => {
                        if !self.items[ti].active {
                            continue;
                        }
                        for (e, on) in self.items[ti].abilities.clone() {
                            let Some(em) = m.get(&e) else { continue };
                            if on && em.run_time == rt && em.active_by_default && em.is(T_PROJECTED) {
                                let n = if em.grouped { 1 } else { self.items[ti].amount };
                                for _ in 0..n {
                                    self.run_effect(e, ti, Ctx::Projected as u16 | Ctx::Fighter as u16, pr, ti);
                                }
                            }
                        }
                    }
                    _ => {
                        let state = self.items[ti].state;
                        for &e in effs {
                            let Some(em) = m.get(&e) else { continue };
                            if em.run_time == rt && em.active_by_default && self.effect_ok_module(em, state) && em.is(T_PROJECTED) {
                                self.run_effect(e, ti, Ctx::Projected as u16 | Ctx::Module as u16, pr, ti);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Module.calculateModifiedAttributes(targetFit, runTime, gang=True)
    fn calc_module_gang(&mut self, md: It, rt: u8) {
        let m = self.meta;
        let state = self.items[md].state;
        let pr = self.items[md].proj_range;
        let ch = self.items[md].charge;
        if ch != NONE {
            for &e in self.items[ch].effects {
                let Some(em) = m.get(&e) else { continue };
                if em.run_time == rt && em.active_by_default && self.effect_ok_module(em, state) && em.is(T_GANG) {
                    self.run_effect(e, md, Ctx::ModuleCharge as u16, pr, md);
                }
            }
        }
        let effs: &'a [u32] = self.items[md].effects;
        if state >= OVERHEATED {
            for &e in effs {
                let Some(em) = m.get(&e) else { continue };
                if em.run_time == rt && em.is(T_OVERHEAT) && em.active_by_default && em.is(T_GANG) {
                    self.run_effect(e, md, Ctx::Module as u16, pr, md);
                }
            }
        }
        for &e in effs {
            let Some(em) = m.get(&e) else { continue };
            if em.run_time == rt && em.active_by_default && self.effect_ok_module(em, state) && em.is(T_GANG) {
                self.run_effect(e, md, Ctx::Module as u16, pr, md);
            }
        }
    }

    fn run_command_boosts(&mut self, rt: u8) {
        let mut k = 0;
        while k < self.command_bonuses.len() {
            if self.command_bonuses[k].run_time != rt {
                k += 1;
                continue;
            }
            let b = self.command_bonuses.remove(k);
            let gang = b.effect == 0 || self.meta.get(&b.effect).map(|x| x.is(T_GANG)).unwrap_or(false);
            if gang {
                self.effect = 0;
                self.modifier = b.thing;
                if !effects::command_buff(self, b.id, b.value) {
                    self.warnings.push(format!("warfare buff {} has no Pyfa handler", b.id));
                }
            }
        }
    }

    fn effect_ok_module(&self, em: &EffMeta, state: i8) -> bool {
        em.is(T_OFFLINE) || (em.is(T_PASSIVE) && state >= ONLINE) || (em.is(T_ACTIVE) && state >= ACTIVE)
    }

    /// Module.calculateModifiedAttributes
    fn calc_module(&mut self, md: It, rt: u8, projected: bool) {
        let m = self.meta;
        let state = self.items[md].state;
        let pr = self.items[md].proj_range;
        let ch = self.items[md].charge;
        if ch != NONE {
            for &e in self.items[ch].effects {
                let Some(em) = m.get(&e) else { continue };
                if em.run_time == rt && em.active_by_default && self.effect_ok_module(em, state) {
                    self.run_effect(e, md, Ctx::ModuleCharge as u16, pr, md);
                }
            }
        }
        let ctx = if projected { Ctx::Projected as u16 | Ctx::Module as u16 } else { Ctx::Module as u16 };
        let effs: &'a [u32] = self.items[md].effects;
        if state >= OVERHEATED {
            for &e in effs {
                let Some(em) = m.get(&e) else { continue };
                if em.run_time == rt && em.is(T_OVERHEAT) && em.active_by_default {
                    self.run_effect(e, md, ctx, pr, md);
                }
            }
        }
        for &e in effs {
            let Some(em) = m.get(&e) else { continue };
            if em.run_time == rt && em.active_by_default && self.effect_ok_module(em, state) && (!projected || em.is(T_PROJECTED)) {
                self.run_effect(e, md, ctx, pr, md);
            }
        }
    }

    /// Drone.calculateModifiedAttributes
    fn calc_drone(&mut self, d: It, rt: u8, projected: bool) {
        let m = self.meta;
        let ctx = if projected { Ctx::Projected as u16 | Ctx::Drone as u16 } else { Ctx::Drone as u16 };
        let pr = self.items[d].proj_range;
        for &e in self.items[d].effects {
            let Some(em) = m.get(&e) else { continue };
            if em.run_time == rt && em.active_by_default && ((projected && em.is(T_PROJECTED)) || (!projected && em.is(T_PASSIVE))) {
                if em.grouped {
                    self.run_effect(e, d, ctx, pr, d);
                } else {
                    for _ in 0..self.items[d].amount_active {
                        self.run_effect(e, d, ctx, pr, d);
                    }
                }
            }
        }
        let ch = self.items[d].charge;
        if ch != NONE {
            for &e in self.items[ch].effects {
                let Some(em) = m.get(&e) else { continue };
                if em.run_time == rt && em.active_by_default {
                    self.run_effect(e, d, Ctx::DroneCharge as u16, pr, d);
                }
            }
        }
    }

    /// Fighter.calculateModifiedAttributes for a projected fighter
    fn calc_fighter_projected(&mut self, f: It, rt: u8) {
        if !self.items[f].active {
            return;
        }
        let m = self.meta;
        let pr = self.items[f].proj_range;
        for (e, on) in self.items[f].abilities.clone() {
            if !on {
                continue;
            }
            let Some(em) = m.get(&e) else { continue };
            if em.run_time == rt && em.active_by_default && em.is(T_PROJECTED) {
                let n = if em.grouped { 1 } else { self.items[f].amount };
                for _ in 0..n {
                    self.run_effect(e, f, Ctx::Projected as u16 | Ctx::Fighter as u16, pr, f);
                }
            }
        }
    }

    /// Fighter.calculateModifiedAttributes
    fn calc_fighter(&mut self, f: It, rt: u8) {
        if !self.items[f].active {
            return;
        }
        let m = self.meta;
        for (e, on) in self.items[f].abilities.clone() {
            if !on {
                continue;
            }
            let Some(em) = m.get(&e) else { continue };
            if em.run_time == rt && em.active_by_default {
                if em.grouped {
                    self.run_effect(e, f, Ctx::Fighter as u16, None, f);
                } else {
                    for _ in 0..self.items[f].amount {
                        self.run_effect(e, f, Ctx::Fighter as u16, None, f);
                    }
                }
            }
        }
    }
}

pub fn py_round_n(v: f64, n: usize) -> f64 {
    format!("{:.*}", n, v).parse().unwrap_or(v)
}
