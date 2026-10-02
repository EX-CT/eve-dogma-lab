//! The fit under calculation and the effect-handler API the transpiled Pyfa handlers call
//! (`fit.ship.boostItemAttr`, `fit.modules.filteredItemBoost`, `module.getModifiedChargeAttr`, ...).
//! GPL-3.0-or-later (mirrors Pyfa eos/saveddata/fit.py, module.py, effectHandlerHelpers.py).
use super::mad::Mad;
use crate::data::{Dataset, TypeInfo};
use crate::request::{SlotReq, Spool};
use rustc_hash::FxHashMap;

pub type It = usize;
pub const NONE: It = usize::MAX;

pub const RT_EARLY: u8 = 0;
pub const RT_NORMAL: u8 = 1;
pub const RT_LATE: u8 = 2;

pub const T_PASSIVE: u16 = 1;
pub const T_ACTIVE: u16 = 2;
pub const T_PROJECTED: u16 = 4;
pub const T_GANG: u16 = 8;
pub const T_OFFLINE: u16 = 16;
pub const T_OVERHEAT: u16 = 32;
pub const T_STRUCTURE: u16 = 64;
pub const T_BOOSTERSIDEEFFECT: u16 = 128;

/// FittingModuleState
pub const OFFLINE: i8 = -1;
pub const ONLINE: i8 = 0;
pub const ACTIVE: i8 = 1;
pub const OVERHEATED: i8 = 2;

#[derive(Debug, Clone, Copy)]
pub struct EffMeta {
    pub id: u32,
    pub name: &'static str,
    pub run_time: u8,
    pub types: u16,
    pub grouped: bool,
    pub deals_damage: bool,
    pub active_by_default: bool,
    pub has_handler: bool,
    pub prefix: &'static str,
    pub has_charges: bool,
}
impl EffMeta {
    pub fn is(&self, t: u16) -> bool {
        self.types & t != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    PreAssign,
    Increase,
    Multiply,
    Boost,
    Force,
}

/// keyword arguments of the Pyfa modification calls
#[derive(Debug, Clone, Copy)]
pub struct O {
    /// skill='Name' -> multiply by that skill's level (type id, 0 = none)
    pub skill: u32,
    pub stack: bool,
    pub group: u8,
    /// increase(position='post')
    pub post: bool,
    /// **kwargs (carries `effect`, enabling projected resistances)
    pub kw: bool,
}
impl Default for O {
    fn default() -> Self {
        O { skill: 0, stack: false, group: 0, post: false, kw: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum L {
    Modules,
    Drones,
    Fighters,
    Implants,
    Boosters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Ctx {
    Skill = 1,
    Implant = 2,
    Booster = 4,
    Ship = 8,
    Module = 16,
    ModuleCharge = 32,
    Drone = 64,
    Fighter = 128,
    Projected = 256,
    DroneCharge = 512,
    CommandRun = 1024,
    System = 2048,
    Structure = 4096,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ship,
    Skill,
    Module,
    Charge,
    Drone,
    Fighter,
    Implant,
    Booster,
    Mode,
    ProjModule,
    ProjDrone,
    ProjFighter,
}

pub struct Item<'a> {
    pub t: &'a TypeInfo,
    pub kind: Kind,
    pub mad: Mad<'a>,
    pub charge: It,
    pub parent: It,
    pub state: i8,
    pub amount: u32,
    pub amount_active: u32,
    pub level: u8,
    pub reload_time: Option<f64>,
    pub force_reload: Option<bool>,
    pub proj_range: Option<f64>,
    pub slot: Option<SlotReq>,
    pub req_index: usize,
    pub spool: Option<Spool>,
    /// effect ids in item order (Pyfa item.effects)
    pub effects: &'a [u32],
    /// fighters: (ability effect id, active)
    pub abilities: Vec<(u32, bool)>,
    pub side_effects: Vec<u32>,
    pub active: bool,
}

pub struct CommandBonus {
    pub id: u32,
    pub run_time: u8,
    pub value: f64,
    pub thing: It,
    pub effect: u32,
}

pub struct Fit<'a> {
    pub ds: &'a Dataset,
    pub items: Vec<Item<'a>>,
    pub ship: It,
    pub chr: It,
    pub mode: It,
    pub modules: Vec<It>,
    pub drones: Vec<It>,
    pub fighters: Vec<It>,
    pub implants: Vec<It>,
    pub boosters: Vec<It>,
    pub skills: Vec<It>,
    pub skill_by_type: FxHashMap<u32, It>,
    pub default_level: u8,
    pub proj_modules: Vec<It>,
    pub proj_drones: Vec<It>,
    pub proj_fighters: Vec<It>,
    pub structure: bool,
    pub factor_reload: bool,
    pub pilot_sec: Option<f64>,
    pub sys_sec: u8,
    pub command_bonuses: Vec<CommandBonus>,
    /// incoming damage pattern for the RAH (None = 'disable')
    pub damage_pattern: Option<[f64; 4]>,
    /// Fit.__extraDrains: (cycleTime ms, capNeed, clipSize, reloadTime)
    pub extra_drains: Vec<(f64, f64, f64, f64)>,
    /// Fit._shieldRr/_armorRr/_hullRr: (layer 0/1/2, amount, cycleTime ms)
    pub rr: Vec<(usize, f64, f64)>,
    /// set while running gang effects of a command fit: bonuses go here instead of the fit
    pub gang_sink: Option<Vec<CommandBonus>>,
    /// buff ids given explicitly in `fleet.buffs`: they override booster fits and the fit's own bursts
    pub explicit_buff_ids: Vec<u32>,
    /// Fit.__ecmProjectedList
    pub ecm: Vec<f64>,
    /// afflictions of the local repair extras (tank kind 0 shield / 1 armor / 2 hull, afflictor), in order
    pub rep_afflictions: Vec<(usize, It)>,
    pub warnings: Vec<String>,
    // ---- current handler context
    pub ctx_flags: u16,
    pub effect: u32,
    pub proj_range: Option<f64>,
    pub modifier: It,
    pub meta: &'static crate::eos::fit::MetaTable,
}

pub type Cx<'a> = Fit<'a>;

pub fn calc_range_factor(opt: f64, falloff: f64, distance: Option<f64>, restricted: bool) -> f64 {
    let Some(d) = distance else { return 1.0 };
    if falloff > 0.0 {
        if restricted && d > opt + 3.0 * falloff {
            return 0.0;
        }
        0.5f64.powf(((d - opt).max(0.0) / falloff).powi(2))
    } else if d <= opt {
        1.0
    } else {
        0.0
    }
}

pub fn py_round(v: f64) -> f64 {
    let r = v.round();
    if (v - v.trunc()).abs() == 0.5 { 2.0 * (v / 2.0).round() } else { r }
}

impl<'a> Fit<'a> {
    // ------------------------------------------------------------ reads
    #[inline]
    pub fn attr(&self, it: It, a: u32) -> f64 {
        if it == NONE {
            return 0.0;
        }
        self.items[it].mad.get(a, self.ds).unwrap_or(0.0)
    }
    pub fn attr_opt(&self, it: It, a: u32) -> Option<f64> {
        if it == NONE {
            return None;
        }
        self.items[it].mad.get(a, self.ds)
    }
    #[inline]
    pub fn charge_attr(&self, it: It, a: u32) -> f64 {
        self.attr(self.charge_of(it), a)
    }
    pub fn base_attr(&self, it: It, a: u32) -> f64 {
        if it == NONE {
            return 0.0;
        }
        self.items[it].mad.original(a, self.ds).unwrap_or(0.0)
    }
    pub fn charge_base_attr(&self, it: It, a: u32) -> f64 {
        self.base_attr(self.charge_of(it), a)
    }
    #[inline]
    pub fn charge_of(&self, it: It) -> It {
        if it == NONE { NONE } else { self.items[it].charge }
    }
    pub fn level(&self, it: It) -> f64 {
        if it == NONE { 0.0 } else { self.items[it].level as f64 }
    }
    pub fn skill_level(&self, skill: u32) -> f64 {
        match self.skill_by_type.get(&skill) {
            Some(&i) => self.items[i].level as f64,
            None => self.default_level as f64,
        }
    }
    #[inline]
    pub fn req_skill(&self, it: It, skill: u32) -> bool {
        it != NONE && self.items[it].t.req_skills.iter().any(|x| x.0 == skill)
    }
    #[inline]
    pub fn group_id(&self, it: It) -> u32 {
        if it == NONE { u32::MAX } else { self.items[it].t.group }
    }
    pub fn type_id(&self, it: It) -> u32 {
        if it == NONE { u32::MAX } else { self.items[it].t.id }
    }
    pub fn type_attr(&self, it: It, a: u32) -> f64 {
        if it == NONE {
            return 0.0;
        }
        self.items[it].mad.in_original(a).then(|| self.items[it].mad.original(a, self.ds).unwrap_or(0.0)).unwrap_or(0.0)
    }
    pub fn type_has_attr(&self, it: It, a: u32) -> bool {
        it != NONE && self.items[it].mad.in_original(a)
    }
    pub fn mad_contains(&self, it: It, a: u32) -> bool {
        it != NONE && self.items[it].mad.contains(a)
    }
    pub fn has_py_attr(&self, it: It, name: &str) -> bool {
        if it == NONE {
            return false;
        }
        let k = self.items[it].kind;
        match name {
            "state" => matches!(k, Kind::Module | Kind::ProjModule),
            "amountActive" => matches!(k, Kind::Drone | Kind::ProjDrone),
            "amount" => matches!(k, Kind::Drone | Kind::ProjDrone | Kind::Fighter),
            _ => false,
        }
    }
    pub fn amount(&self, it: It) -> f64 {
        if it == NONE { 0.0 } else { self.items[it].amount as f64 }
    }
    pub fn amount_active(&self, it: It) -> f64 {
        if it == NONE { 0.0 } else { self.items[it].amount_active as f64 }
    }
    pub fn state(&self, it: It) -> f64 {
        if it == NONE { -1.0 } else { self.items[it].state as f64 }
    }
    #[inline]
    pub fn ctx(&self, c: Ctx) -> bool {
        self.ctx_flags & (c as u16) != 0
    }
    pub fn is_structure(&self) -> bool {
        self.structure
    }
    pub fn factor_reload(&self) -> bool {
        self.factor_reload
    }
    pub fn pilot_security(&self, lo: f64, hi: f64) -> f64 {
        self.pilot_sec.unwrap_or(0.0).min(hi).max(lo)
    }
    pub fn system_security(&self) -> f64 {
        self.sys_sec as f64
    }
    /// Fit.scanType: 0 Magnetometric, 1 Ladar, 2 Radar, 3 Gravimetric, 4 Multispectral
    pub fn scan_type(&self) -> usize {
        let names = ["scanMagnetometricStrength", "scanLadarStrength", "scanRadarStrength", "scanGravimetricStrength"];
        let mut best = -1.0;
        let mut t = 4;
        for (i, n) in names.iter().enumerate() {
            let v = self.attr(self.ship, self.ds.attr_id(n));
            if v > best {
                best = v;
                t = i;
            } else if v == best {
                t = 4;
            }
        }
        t
    }
    pub fn extra(&self, a: u32) -> f64 {
        self.attr(self.ship, a)
    }

    /// ModifiedAttributeDict.getResistance for the effect being run by the registered modifier
    pub fn resistance(&self) -> f64 {
        let Some(m) = self.meta.get(&self.effect) else { return 1.0 };
        if !m.is(T_PROJECTED) {
            return 1.0;
        }
        let mut rid = self.ds.effects.get(&self.effect).and_then(|e| e.resistance_attr).unwrap_or(0);
        if rid == 0 {
            rid = self.attr(self.modifier, self.ds.attr_id("remoteResistanceID")) as u32;
        }
        if rid == 0 {
            return 1.0;
        }
        let r = self.attr(self.ship, rid);
        if r == 0.0 { 1.0 } else { r }
    }

    // ------------------------------------------------------------ writes
    pub fn op(&mut self, t: It, op: Op, a: u32, v: f64, o: O) {
        if t == NONE {
            return;
        }
        let mut v = v;
        // scale first (reads other state), then a single entry lookup for the write
        match op {
            Op::PreAssign | Op::Force => {}
            Op::Increase => {
                if o.skill != 0 {
                    v *= self.skill_level(o.skill);
                }
                if o.kw {
                    let r = self.resistance();
                    v *= if r == 0.0 { 1.0 } else { r };
                }
            }
            Op::Multiply | Op::Boost => {
                if o.skill != 0 {
                    v *= self.skill_level(o.skill);
                }
                if op == Op::Boost {
                    v = 1.0 + v / 100.0;
                }
                if o.kw {
                    let r = self.resistance();
                    if r != 1.0 {
                        v = (v - 1.0) * r + 1.0;
                    }
                }
            }
        }
        let e = self.items[t].mad.entry(a);
        match op {
            Op::PreAssign => e.pre_assign = Some(v),
            Op::Force => e.forced = Some(v),
            Op::Increase => {
                if o.post {
                    e.post_inc += v;
                } else {
                    e.pre_inc += v;
                }
            }
            Op::Multiply | Op::Boost => {
                if o.stack {
                    match e.pen.iter_mut().find(|x| x.0 == o.group) {
                        Some(g) => g.1.push(v),
                        None => e.pen.push((o.group, vec![v])),
                    }
                } else {
                    e.mult *= v;
                }
            }
        }
        e.placeholder = true;
        e.cache.set(None);
    }

    pub fn list(&self, l: L) -> &[It] {
        match l {
            L::Modules => &self.modules,
            L::Drones => &self.drones,
            L::Fighters => &self.fighters,
            L::Implants => &self.implants,
            L::Boosters => &self.boosters,
        }
    }

    pub fn filtered(&mut self, l: L, charge: bool, f: &dyn Fn(&Cx, It) -> bool, op: Op, a: u32, v: f64, o: O) {
        let n = self.list(l).len();
        for k in 0..n {
            let m = self.list(l)[k];
            if f(self, m) {
                let t = if charge { self.items[m].charge } else { m };
                self.op(t, op, a, v, o);
            }
        }
    }

    pub fn extra_increase(&mut self, a: u32, v: f64) {
        let s = self.ship;
        if v != 0.0 && self.modifier != NONE {
            let k = match crate::generated::effects::EXTRA_ATTRS.iter().find(|x| x.0 == a).map(|x| x.1) {
                Some("shieldRepair") => 0,
                Some("armorRepair") => 1,
                Some("hullRepair") => 2,
                _ => 9,
            };
            if k < 9 {
                let m = self.modifier;
                self.rep_afflictions.push((k, m));
            }
        }
        self.op(s, Op::Increase, a, v, O::default());
    }
    pub fn extra_boost(&mut self, a: u32, v: f64) {
        let s = self.ship;
        self.op(s, Op::Boost, a, v, O::default());
    }
    pub fn extra_set(&mut self, a: u32, v: f64) {
        let s = self.ship;
        self.set_intermediary(s, a, v);
    }
    pub fn set_intermediary(&mut self, it: It, a: u32, v: f64) {
        if it == NONE {
            return;
        }
        let e = self.items[it].mad.entry(a);
        e.inter = Some(v);
    }
    pub fn set_reload_time(&mut self, it: It, v: f64) {
        if it != NONE {
            self.items[it].reload_time = Some(v);
        }
    }
    pub fn set_force_reload(&mut self, it: It, v: bool) {
        if it != NONE {
            self.items[it].force_reload = Some(v);
        }
    }
    pub fn add_command_bonus(&mut self, id: f64, value: f64, thing: It, rt: u8) {
        let id = id as u32;
        if self.gang_sink.is_none() && self.explicit_buff_ids.contains(&id) {
            return;
        }
        let effect = self.effect;
        let list = match self.gang_sink.as_mut() {
            Some(l) => l,
            None => &mut self.command_bonuses,
        };
        match list.iter_mut().find(|b| b.id == id) {
            Some(b) => {
                if b.value.abs() < value.abs() {
                    *b = CommandBonus { id, run_time: rt, value, thing, effect };
                }
            }
            None => list.push(CommandBonus { id, run_time: rt, value, thing, effect }),
        }
    }
}
