//! FitStats from a calculated fit. Ports of Pyfa eos/saveddata/fit.py (cpuUsed, hp/ehp, tank, capacitor,
//! maxSpeed, alignTime, warpSpeed, maxTargets, scanStrength, probeSize), module.py (getCycleParameters,
//! numShots, getVolleyParameters, getDps, capUse), drone.py, fighter.py, fighterAbility.py and
//! damagePattern.py. GPL-3.0-or-later.
use super::capsim::{self, Drain};
use super::cx::*;
use super::fit::{extra_attr, meta};
use crate::request::{FitRequest, SlotReq, Spool, SpoolType};
use crate::jv::{obj, Map, Value};

const INF: f64 = f64::INFINITY;

#[derive(Clone, Copy, Default)]
pub struct Dmg {
    pub em: f64,
    pub th: f64,
    pub ki: f64,
    pub ex: f64,
}
impl Dmg {
    pub(crate) fn total(&self) -> f64 {
        self.em + self.th + self.ki + self.ex
    }
    pub(crate) fn add(&mut self, o: &Dmg) {
        self.em += o.em;
        self.th += o.th;
        self.ki += o.ki;
        self.ex += o.ex;
    }
    pub(crate) fn mul(&self, f: f64) -> Dmg {
        Dmg { em: self.em * f, th: self.th * f, ki: self.ki * f, ex: self.ex * f }
    }
    pub(crate) fn json(&self) -> Value {
        obj(vec![("em", Value::from(self.em)), ("thermal", Value::from(self.th)), ("kinetic", Value::from(self.ki)), ("explosive", Value::from(self.ex)), ("total", Value::from(self.total()))])
    }
    pub(crate) fn vs(&self, r: [f64; 4]) -> Value {
        let d = Dmg { em: self.em * (1.0 - r[0]), th: self.th * (1.0 - r[1]), ki: self.ki * (1.0 - r[2]), ex: self.ex * (1.0 - r[3]) };
        d.json()
    }
}

/// eos/utils/float.py floatUnerr (keepDigits = 7)
pub fn float_unerr(v: f64) -> f64 {
    if v == 0.0 || v == INF {
        return v;
    }
    let f = 7 - (v.abs().log10()).ceil() as i32;
    py_round_digits(v, f)
}

const POW10: [f64; 23] = [1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19, 1e20, 1e21, 1e22];

/// Python round(v, n)
pub fn py_round_digits(v: f64, n: i32) -> f64 {
    if !v.is_finite() {
        return v;
    }
    if n >= 0 {
        // fast path: with p = 10^n exact (n <= 22) and |v*p| < 2^52, z = round(v*p) is the correctly
        // rounded decimal unless v*p is within its rounding error of a .5 tie; z/p is then the correctly
        // rounded double of that decimal (Clinger fast path), i.e. what format+parse yields.
        if n <= 22 {
            let p = POW10[n as usize];
            let y = v * p;
            if y.abs() < 4.0e15 {
                let d = (y - y.floor() - 0.5).abs();
                if d > y.abs() * 1e-15 + 1e-300 {
                    return y.round() / p;
                }
            }
        }
        format!("{:.*}", n as usize, v).parse().unwrap_or(v)
    } else {
        let p = 10f64.powi(-n);
        let q = v / p;
        let r = q.round();
        let r = if (q - q.trunc()).abs() == 0.5 { 2.0 * (q / 2.0).round() } else { r };
        r * p
    }
}

/// eos/utils/spoolSupport.py calculateSpoolup -> (value, cycles, time)
pub fn calculate_spoolup(max: f64, step: f64, cycle_s: f64, sp: Option<Spool>) -> (f64, f64, f64) {
    if max == 0.0 || step == 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let Some(sp) = sp else { return (0.0, 0.0, 0.0) };
    let cycles = match sp.kind {
        SpoolType::SpoolScale => float_unerr(max * sp.amount / step).ceil(),
        SpoolType::CycleScale => {
            let c = sp.amount * float_unerr(max / step).ceil();
            py_round_digits(c, 0)
        }
        SpoolType::Time => float_unerr(sp.amount / cycle_s).floor().min(float_unerr(max / step).ceil()),
        SpoolType::Cycles => sp.amount.floor().min(float_unerr(max / step).ceil()),
    };
    (max.min(cycles * step), cycles, cycles * cycle_s)
}

pub struct Ids {
    pub cpu: u32,
    pub power: u32,
    pub upgrade_cost: u32,
    pub cap_need: u32,
    pub reload_time: u32,
    pub charge_rate: u32,
    pub crystals: u32,
    pub reactivation: u32,
}

impl<'a> Fit<'a> {
    pub fn a(&self, name: &str) -> u32 {
        self.ds.attr_id(name)
    }
    /// getModifiedItemAttr(name, default)
    pub fn g(&self, it: It, name: &str) -> f64 {
        let id = self.ds.attr_id(name);
        if id == 0 {
            return 0.0;
        }
        self.attr_opt(it, id).unwrap_or(0.0)
    }
    pub fn gd(&self, it: It, name: &str, d: f64) -> f64 {
        let id = self.ds.attr_id(name);
        if id == 0 {
            return d;
        }
        self.attr_opt(it, id).unwrap_or(d)
    }
    pub(crate) fn has_effect_name(&self, it: It, name: &str) -> bool {
        let id = self.ds.effect_id(name);
        id != 0 && self.items[it].effects.contains(&id)
    }

    pub(crate) fn raw_cycle_time(&self, m: It) -> f64 {
        static IDS: std::sync::OnceLock<[u32; 7]> = std::sync::OnceLock::new();
        let ds = self.ds;
        let ids = IDS.get_or_init(|| {
            [
                "speed",
                "duration",
                "durationHighisGood",
                "durationSensorDampeningBurstProjector",
                "durationTargetIlluminationBurstProjector",
                "durationECMJammerBurstProjector",
                "durationWeaponDisruptionBurstProjector",
            ]
            .map(|n| ds.attr_id(n))
        });
        ids.iter().map(|&a| if a == 0 { 0.0 } else { self.attr_opt(m, a).unwrap_or(0.0) }).fold(0.0, f64::max)
    }

    pub(crate) fn num_charges(&self, m: It) -> f64 {
        let c = self.items[m].charge;
        if c == NONE {
            return 0.0;
        }
        let vol = self.items[c].t.attr(161);
        let cap = self.items[m].t.attr(38);
        match (vol, cap) {
            (Some(v), Some(cap)) if v != 0.0 => float_unerr(cap / v).trunc(),
            _ => 0.0,
        }
    }

    pub fn num_shots(&self, m: It) -> f64 {
        let c = self.items[m].charge;
        if c == NONE {
            return 0.0;
        }
        let n = self.num_charges(m);
        let cr = self.a("chargeRate");
        let cg = self.a("crystalsGetDamaged");
        if n > 0.0 && self.items[m].mad.contains(cr) {
            (n / self.attr(m, cr)).floor()
        } else if n > 0.0 && self.items[c].mad.contains(cg) {
            if self.attr(c, cg) == 1.0 {
                let hp = self.g(c, "hp");
                let ch = self.g(c, "crystalVolatilityChance");
                let dm = self.g(c, "crystalVolatilityDamage");
                ((n * hp) / (dm * ch)).floor()
            } else {
                0.0
            }
        } else {
            0.0
        }
    }

    pub(crate) fn reload_time(&self, m: It) -> f64 {
        let id = self.a("reloadTime");
        let v = if id == 0 { None } else { self.attr_opt(m, id) };
        let v = v.or(self.items[m].reload_time);
        v.unwrap_or(0.0)
    }

    /// getCycleParameters().averageTime (None = no cycle)
    pub fn cycle_avg(&self, m: It, reload_override: Option<bool>) -> Option<f64> {
        let factor = reload_override.unwrap_or(self.items[m].force_reload.unwrap_or(self.factor_reload));
        let mut cur = self.num_shots(m);
        if cur == 0.0 {
            cur = INF;
        }
        let active = self.raw_cycle_time(m);
        if active == 0.0 {
            return None;
        }
        let inactive = self.g(m, "moduleReactivationDelay");
        let reload = self.reload_time(m);
        if !factor || cur == INF || inactive >= reload {
            return Some(active + inactive);
        }
        let early = cur - 1.0;
        if early == 0.0 {
            return Some(active + reload);
        }
        // CycleSequence((active, inactive, early), (active, reload, 1)) -> averageTime
        Some(((active + inactive) * early + (active + reload)) / (early + 1.0))
    }

    pub(crate) fn is_breacher(&self, m: It) -> bool {
        let c = self.items[m].charge;
        c != NONE && self.has_effect_name(c, "dotMissileLaunching")
    }

    pub(crate) fn spool_opts(&self, m: It, default: Option<Spool>) -> Option<Spool> {
        self.items[m].spool.or(default)
    }

    /// getVolleyParameters -> list of (time, volley)
    pub(crate) fn volley_params(&self, m: It, default_spool: Option<Spool>) -> Vec<(f64, Dmg)> {
        self.volley_params_sp(m, self.spool_opts(m, default_spool))
    }
    /// getVolleyParameters with already-resolved spool options
    pub(crate) fn volley_params_sp(&self, m: It, spool: Option<Spool>) -> Vec<(f64, Dmg)> {
        if self.items[m].state < ACTIVE {
            return vec![(0.0, Dmg::default())];
        }
        let c = self.items[m].charge;
        let mut base: Vec<(f64, Dmg)> = Vec::new();
        if self.is_breacher(m) {
            // breacher DoT: not damage in DmgTypes terms (absolute/relative per tick)
            let sub = (self.g(c, "dotDuration") / 1000.0).floor();
            for i in 0..(sub as i64) {
                base.push((1.0 + i as f64, Dmg::default()));
            }
        } else {
            let src = if c != NONE { c } else { m };
            let mult = self.gd(m, "damageMultiplier", 1.0);
            let eff = |n: &str| self.has_effect_name(m, n);
            let delay = if ["superWeaponAmarr", "superWeaponCaldari", "superWeaponGallente", "superWeaponMinmatar", "lightningWeapon"].iter().any(|n| eff(n)) {
                self.g(m, "damageDelayDuration")
            } else if ["doomsdayBeamDOT", "doomsdaySlash", "doomsdayConeDOT", "debuffLance"].iter().any(|n| eff(n)) {
                self.g(m, "doomsdayWarningDuration")
            } else {
                0.0
            };
            let dur = self.g(m, "doomsdayDamageDuration");
            let sub = self.g(m, "doomsdayDamageCycleTime");
            let n = if dur != 0.0 && sub != 0.0 && !eff("doomsdaySlash") { float_unerr(dur / sub).floor() } else { 1.0 };
            let d = Dmg {
                em: self.g(src, "emDamage") * mult,
                th: self.g(src, "thermalDamage") * mult,
                ki: self.g(src, "kineticDamage") * mult,
                ex: self.g(src, "explosiveDamage") * mult,
            };
            for i in 0..(n as i64) {
                let t = delay + sub * i as f64;
                match base.iter_mut().find(|x| x.0 == t) {
                    Some(x) => x.1 = d,
                    None => base.push((t, d)),
                }
            }
        }
        let sp = calculate_spoolup(
            self.g(m, "damageMultiplierBonusMax"),
            self.g(m, "damageMultiplierBonusPerCycle"),
            self.raw_cycle_time(m) / 1000.0,
            spool,
        )
        .0;
        base.into_iter().map(|(t, d)| (t, d.mul(1.0 + sp))).collect()
    }

    pub fn module_volley(&self, m: It, sp: Option<Spool>) -> Dmg {
        let p = self.volley_params(m, sp);
        p.iter().min_by(|a, b| a.0.partial_cmp(&b.0).unwrap()).map(|x| x.1).unwrap_or_default()
    }

    pub fn module_dps(&self, m: It, sp: Option<Spool>) -> Dmg {
        let Some(avg) = self.cycle_avg(m, None) else { return Dmg::default() };
        let p = self.volley_params(m, sp);
        if p.is_empty() || avg == 0.0 {
            return Dmg::default();
        }
        if self.is_breacher(m) {
            return self.module_volley(m, sp);
        }
        let mut d = Dmg::default();
        for (_, v) in &p {
            d.add(v);
        }
        d.mul(1.0 / (avg / 1000.0))
    }

    /// (module_volley, module_dps, cycle_avg) with one getVolleyParameters / getCycleParameters evaluation
    pub(crate) fn volley_dps_cycle(&self, m: It, sp: Option<Spool>) -> (Dmg, Dmg, Option<f64>) {
        let p = self.volley_params(m, sp);
        let vol = p.iter().min_by(|a, b| a.0.partial_cmp(&b.0).unwrap()).map(|x| x.1).unwrap_or_default();
        let avg = self.cycle_avg(m, None);
        let dps = match avg {
            None => Dmg::default(),
            Some(a) if p.is_empty() || a == 0.0 => Dmg::default(),
            Some(_) if self.is_breacher(m) => vol,
            Some(a) => {
                let mut d = Dmg::default();
                for (_, v) in &p {
                    d.add(v);
                }
                d.mul(1.0 / (a / 1000.0))
            }
        };
        (vol, dps, avg)
    }

    pub fn cap_use(&self, m: It) -> f64 {
        let need = self.g(m, "capacitorNeed");
        if need != 0.0 && self.items[m].state >= ACTIVE {
            match self.cycle_avg(m, None) {
                Some(t) if t > 0.0 => need / (t / 1000.0),
                _ => 0.0,
            }
        } else {
            0.0
        }
    }

    pub(crate) fn drone_deals_damage(&self, d: It) -> bool {
        let c = self.items[d].charge;
        ["emDamage", "kineticDamage", "explosiveDamage", "thermalDamage"].iter().any(|n| {
            let id = self.a(n);
            self.items[d].mad.contains(id) || (c != NONE && self.items[c].mad.contains(id))
        })
    }

    pub(crate) fn drone_cycle(&self, d: It) -> f64 {
        let ct = if self.items[d].charge != NONE {
            self.g(d, "missileLaunchDuration")
        } else {
            let mut c = 0.0;
            for n in ["speed", "duration", "durationHighisGood"] {
                c = self.g(d, n);
                if c != 0.0 {
                    break;
                }
            }
            c
        };
        ct.max(0.0)
    }

    pub fn drone_volley(&self, d: It) -> Dmg {
        let n = self.items[d].amount_active as f64;
        if !self.drone_deals_damage(d) || n <= 0.0 {
            return Dmg::default();
        }
        let c = self.items[d].charge;
        let src = if c != NONE { c } else { d };
        let mult = n * self.gd(d, "damageMultiplier", 1.0);
        Dmg {
            em: self.g(src, "emDamage") * mult,
            th: self.g(src, "thermalDamage") * mult,
            ki: self.g(src, "kineticDamage") * mult,
            ex: self.g(src, "explosiveDamage") * mult,
        }
    }

    pub fn drone_dps(&self, d: It) -> Dmg {
        let v = self.drone_volley(d);
        if v.total() == 0.0 {
            return Dmg::default();
        }
        let ct = self.drone_cycle(d);
        if ct == 0.0 {
            return Dmg::default();
        }
        v.mul(1.0 / (ct / 1000.0))
    }

    pub(crate) fn ability_prefix(&self, e: u32) -> &'static str {
        meta().get(&e).map(|m| m.prefix).unwrap_or("")
    }

    pub(crate) fn ability_volley(&self, f: It, e: u32, active: bool) -> Dmg {
        let p = self.ability_prefix(e);
        let deals = self.items[f].mad.contains(self.a(&format!("{p}DamageMultiplier"))) || self.items[f].charge != NONE;
        if !deals || !active {
            return Dmg::default();
        }
        let (em, th, ki, ex) = if p == "fighterAbilityLaunchBomb" {
            let c = self.items[f].charge;
            (self.g(c, "emDamage"), self.g(c, "thermalDamage"), self.g(c, "kineticDamage"), self.g(c, "explosiveDamage"))
        } else {
            (
                self.g(f, &format!("{p}DamageEM")),
                self.g(f, &format!("{p}DamageTherm")),
                self.g(f, &format!("{p}DamageKin")),
                self.g(f, &format!("{p}DamageExp")),
            )
        };
        let mult = self.items[f].amount as f64 * self.gd(f, &format!("{p}DamageMultiplier"), 1.0);
        Dmg { em: em * mult, th: th * mult, ki: ki * mult, ex: ex * mult }
    }

    pub(crate) fn ability_cycle(&self, f: It, e: u32) -> f64 {
        self.g(f, &format!("{}Duration", self.ability_prefix(e)))
    }
    pub(crate) fn ability_has_charges(e: u32) -> bool {
        meta().get(&e).map(|m| m.has_charges).unwrap_or(false)
    }
    pub(crate) fn ability_num_shots(&self, f: It, e: u32) -> f64 {
        if !Self::ability_has_charges(e) {
            return 0.0;
        }
        match self.g(f, "fighterSquadronRole") as i64 {
            2 => 12.0,
            4 => 6.0,
            5 => 3.0,
            _ => 0.0,
        }
    }
    pub(crate) fn ability_reload(&self, f: It, e: u32, spent: Option<f64>) -> f64 {
        let ns = self.ability_num_shots(f, e);
        let spent = spent.map(|s| s.max(ns)).unwrap_or(ns);
        let rearm = if Self::ability_has_charges(e) {
            match self.g(f, "fighterSquadronRole") as i64 {
                2 => 4000.0,
                4 => 6000.0,
                5 => 20000.0,
                _ => 0.0,
            }
        } else {
            0.0
        };
        self.g(f, "fighterRefuelingTime") + rearm * spent
    }

    /// fighter.getCycleParametersPerEffect -> (effect, averageTime)
    pub(crate) fn fighter_cycles(&self, f: It, factor: bool) -> Vec<(u32, f64)> {
        let ab = &self.items[f].abilities;
        let all: Vec<(u32, f64)> = ab.iter().map(|a| (a.0, self.ability_cycle(f, a.0))).filter(|x| x.1 > 0.0).collect();
        if !factor {
            return all;
        }
        let limited: Vec<u32> = ab.iter().map(|a| a.0).filter(|&e| self.ability_num_shots(f, e) > 0.0 && self.ability_cycle(f, e) > 0.0).collect();
        if limited.is_empty() {
            return all;
        }
        if all.is_empty() {
            return vec![];
        }
        let mut most = limited[0];
        for &e in &limited {
            if self.ability_cycle(f, e) * self.ability_num_shots(f, e) < self.ability_cycle(f, most) * self.ability_num_shots(f, most) {
                most = e;
            }
        }
        let dur = self.ability_cycle(f, most) * self.ability_num_shots(f, most);
        let mut until: Vec<(u32, f64, Option<f64>)> = vec![(most, self.ability_num_shots(f, most), None)];
        for &(e, ct) in &all {
            if e == most {
                continue;
            }
            let full = float_unerr(dur / ct).trunc();
            let mut extra = Some(float_unerr(dur - full * ct));
            if extra == Some(0.0) {
                extra = None;
            }
            until.push((e, full, extra));
        }
        let get = |e: u32| until.iter().find(|x| x.0 == e).copied().unwrap();
        let mut refuel = 0.0f64;
        for &(e, _) in &all {
            let (_, mut spent, extra) = get(e);
            if extra.is_some() {
                spent += 1.0;
            }
            refuel = refuel.max(self.ability_reload(f, e, Some(spent)));
        }
        let mut out = Vec::new();
        for &(e, ct) in &all {
            let (_, regular, extra) = get(e);
            let mut seq: Vec<(f64, f64, f64)> = Vec::new();
            if let Some(x) = extra {
                if regular > 0.0 {
                    seq.push((ct, 0.0, regular));
                }
                seq.push((x, refuel, 1.0));
            } else {
                if regular - 1.0 > 0.0 {
                    seq.push((ct, 0.0, regular - 1.0));
                }
                seq.push((ct, refuel, 1.0));
            }
            let t: f64 = seq.iter().map(|s| (s.0 + s.1) * s.2).sum();
            let q: f64 = seq.iter().map(|s| s.2).sum();
            out.push((e, t / q));
        }
        out
    }

    pub fn fighter_volley(&self, f: It) -> Dmg {
        let mut v = Dmg::default();
        if !self.items[f].active || self.items[f].amount == 0 {
            return v;
        }
        for &(e, on) in &self.items[f].abilities {
            v.add(&self.ability_volley(f, e, on));
        }
        v
    }

    pub fn fighter_dps(&self, f: It) -> Dmg {
        let mut d = Dmg::default();
        if !self.items[f].active || self.items[f].amount == 0 {
            return d;
        }
        let ab = self.items[f].abilities.clone();
        let dps_with = |cyc: &[(u32, f64)]| -> Vec<(u32, Dmg)> {
            ab.iter()
                .filter_map(|&(e, on)| cyc.iter().find(|x| x.0 == e).map(|x| (e, self.ability_volley(f, e, on).mul(1.0 / (x.1 / 1000.0)))))
                .collect()
        };
        let inf: Vec<(u32, f64)> = ab
            .iter()
            .filter(|a| self.ability_num_shots(f, a.0) == 0.0 && self.ability_cycle(f, a.0) > 0.0)
            .map(|a| (a.0, self.ability_cycle(f, a.0)))
            .collect();
        let rel = self.fighter_cycles(f, self.factor_reload);
        let ti: f64 = dps_with(&inf).iter().map(|x| x.1.total()).sum();
        let tr: f64 = dps_with(&rel).iter().map(|x| x.1.total()).sum();
        let chosen = if ti >= tr { inf } else { rel };
        for (_, x) in dps_with(&chosen) {
            d.add(&x);
        }
        d
    }

    pub(crate) fn slot_of(&self, m: It) -> Option<SlotReq> {
        self.items[m].slot
    }
    pub(crate) fn hardpoint(&self, m: It) -> u8 {
        let t = self.items[m].t;
        if t.has_effect(42) {
            1
        } else if t.has_effect(40) {
            2
        } else {
            0
        }
    }

    /// Drone.maxRange
    pub(crate) fn drone_max_range(&self, d: It) -> Option<f64> {
        for a in ["shieldTransferRange", "powerTransferRange", "energyDestabilizationRange", "empFieldRange", "ecmBurstRange", "maxRange", "ECMRangeOptimal"] {
            let v = self.g(d, a);
            if v != 0.0 {
                return Some(v);
            }
        }
        let c = self.items[d].charge;
        if c != NONE {
            return Some(self.g(c, "explosionDelay") / 1000.0 * self.g(c, "maxVelocity"));
        }
        None
    }
    pub(crate) fn drone_falloff(&self, d: It) -> Option<f64> {
        for a in ["falloff", "falloffEffectiveness"] {
            let v = self.g(d, a);
            if v != 0.0 {
                return Some(v);
            }
        }
        None
    }

    /// Fit.calculateSustainableTank: local repairers limited by capacitor (shield, armor, hull)
    pub(crate) fn sustainable_tank(&self, base: [f64; 3], cap_stable: bool, cap_used: f64, cap_recharge: f64) -> [f64; 3] {
        let mut out = base;
        if cap_stable && !self.factor_reload {
            return out;
        }
        let ds = self.ds;
        let gattr = |g: &str| -> Option<&'static str> {
            match g {
                "Shield Booster" | "Ancillary Shield Booster" => Some("shieldBonus"),
                "Armor Repair Unit" | "Ancillary Armor Repairer" => Some("armorDamageAmount"),
                "Hull Repair Unit" => Some("structureDamageAmount"),
                _ => None,
            }
        };
        let gstore = |g: &str| -> usize {
            match g {
                "Shield Booster" | "Ancillary Shield Booster" => 0,
                "Armor Repair Unit" | "Ancillary Armor Repairer" => 1,
                _ => 2,
            }
        };
        let mut adj = [0.0f64; 3];
        let mut repairers: Vec<It> = Vec::new();
        let mut cap_used = cap_used;
        for kind in 0..3 {
            for &(k, a) in &self.rep_afflictions {
                if k != kind {
                    continue;
                }
                let it = &self.items[a];
                if !matches!(it.kind, Kind::Module) {
                    continue;
                }
                let gname = ds.group_name(it.t.group);
                let Some(attr) = gattr(gname) else { continue };
                let cu = self.cap_use(a);
                let uses_cap = cu != 0.0;
                if uses_cap {
                    cap_used -= cu;
                }
                let c = it.charge;
                let ct = self.raw_cycle_time(a);
                if uses_cap && c == NONE {
                    adj[kind] -= self.g(a, attr) / (ct / 1000.0);
                    repairers.push(a);
                } else if uses_cap {
                    let mult = if self.items[c].t.name == "Nanite Repair Paste" {
                        let m = self.g(a, "chargedArmorDamageMultiplier");
                        if m != 0.0 { m } else { 1.0 }
                    } else {
                        1.0
                    };
                    adj[kind] -= self.g(a, attr) * mult / (ct / 1000.0);
                    repairers.push(a);
                } else if gname == "Ancillary Shield Booster" || gname == "Ancillary Remote Shield Booster" {
                    let rt = if self.factor_reload && c != NONE { self.reload_time(a) } else { 0.0 };
                    let off = rt / ((self.num_shots(a).max(1.0) * ct) + rt);
                    adj[kind] -= self.g(a, attr) * off / (ct / 1000.0);
                }
            }
        }
        let key = |m: It| {
            let attr = gattr(ds.group_name(self.items[m].t.group)).unwrap_or("");
            let cm = self.g(m, "chargedArmorDamageMultiplier");
            self.g(m, attr) * (if cm != 0.0 { cm } else { 1.0 }) / self.g(m, "capacitorNeed")
        };
        // Python sort(reverse=True) is stable
        repairers.sort_by(|a, b| key(*b).partial_cmp(&key(*a)).unwrap_or(std::cmp::Ordering::Equal));
        for &a in &repairers {
            if cap_used > cap_recharge {
                break;
            }
            let c = self.items[a].charge;
            let rt = if self.factor_reload && c != NONE { self.reload_time(a) } else { 0.0 };
            let ct = self.raw_cycle_time(a);
            let cps = self.cap_use(a);
            let sustain = ((cap_recharge - cap_used) / cps).min(1.0);
            let gname = ds.group_name(self.items[a].t.group);
            let amount = self.g(a, gattr(gname).unwrap_or(""));
            if c == NONE {
                adj[gstore(gname)] += sustain * amount / (ct / 1000.0);
            } else {
                let mult = if self.items[c].t.name == "Nanite Repair Paste" {
                    let m = self.g(a, "chargedArmorDamageMultiplier");
                    if m != 0.0 { m } else { 1.0 }
                } else {
                    1.0
                };
                let ns = self.num_shots(a).max(1.0);
                let on = (ns * ct) / ((ns * ct) + rt);
                adj[gstore(gname)] += sustain * amount * on * mult / (ct / 1000.0);
            }
            cap_used += cps;
        }
        for i in 0..3 {
            out[i] += adj[i];
        }
        out
    }

    /// Module.maxRange
    pub fn max_range(&self, m: It) -> Option<f64> {
        for a in ["maxRange", "shieldTransferRange", "powerTransferRange", "energyDestabilizationRange", "empFieldRange",
                  "ecmBurstRange", "warpScrambleRange", "cargoScanRange", "shipScanRange", "surveyScanRange"] {
            let v = self.g(m, a);
            if v != 0.0 {
                if self.items[m].t.name.to_lowercase().contains("burst projector") {
                    return Some(v - self.g(self.ship, "radius"));
                }
                return Some(v);
            }
        }
        let (lo, hi, ch) = self.missile_range(m)?;
        Some(lo * (1.0 - ch) + hi * ch)
    }

    pub(crate) fn missile_range(&self, m: It) -> Option<(f64, f64, f64)> {
        let c = self.items[m].charge;
        if c == NONE {
            return None;
        }
        let gn = self.ds.group_name(self.items[c].t.group);
        if gn == "Scanner Probe" || gn == "Survey Probe" {
            return None;
        }
        let v = self.g(c, "maxVelocity");
        if v == 0.0 {
            return None;
        }
        let r = self.g(self.ship, "radius");
        let ft = float_unerr(self.g(c, "explosionDelay") / 1000.0 + r / v);
        let mass = self.g(c, "mass");
        let ag = self.g(c, "agility");
        let calc = |t: f64| {
            let acc = t.min(mass * ag / 1e6);
            v / 2.0 * acc + v * (t - acc)
        };
        let (lt, ht) = (ft.floor(), ft.ceil());
        let (mut lo, mut hi) = (calc(lt), calc(ht));
        if self.has_effect_name(c, "fofMissileLaunching") {
            let lim = self.g(c, "maxFOFTargetRange");
            if lim != 0.0 {
                lo = lo.min(lim);
                hi = hi.min(lim);
            }
        }
        Some(((lo - r).max(0.0), (hi - r).max(0.0), ft - lt))
    }

    pub fn falloff(&self, m: It) -> Option<f64> {
        for a in ["falloffEffectiveness", "falloff", "shipScanFalloff"] {
            let v = self.g(m, a);
            if v != 0.0 {
                return Some(v);
            }
        }
        None
    }

    /// weapon category label (contract `offense.weapons[].kind`)
    pub(crate) fn weapon_kind(&self, m: It) -> &'static str {
        if self.has_effect_name(m, "turretFitted") {
            "turret"
        } else if self.has_effect_name(m, "launcherFitted") {
            "missile"
        } else if self.has_effect_name(m, "empWave") {
            "smartbomb"
        } else if self.has_effect_name(m, "ChainLightning") {
            "vorton"
        } else {
            "other"
        }
    }

    pub(crate) fn effectivify(&self, pattern: [f64; 4], amount: f64, layer: &str) -> f64 {
        let r = self.resonances(layer);
        let tot: f64 = pattern.iter().sum();
        let tot = if tot == 0.0 { 1.0 } else { tot };
        let div: f64 = (0..4).map(|i| pattern[i] / tot * r[i]).sum();
        amount / if div == 0.0 { 1.0 } else { div }
    }

    pub(crate) fn resonances(&self, layer: &str) -> [f64; 4] {
        const N: [[&str; 4]; 3] = [
            ["shieldEmDamageResonance", "shieldThermalDamageResonance", "shieldKineticDamageResonance", "shieldExplosiveDamageResonance"],
            ["armorEmDamageResonance", "armorThermalDamageResonance", "armorKineticDamageResonance", "armorExplosiveDamageResonance"],
            ["emDamageResonance", "thermalDamageResonance", "kineticDamageResonance", "explosiveDamageResonance"],
        ];
        let s = self.ship;
        let n = match layer {
            "shield" => &N[0],
            "armor" => &N[1],
            "hull" => &N[2],
            _ => {
                let f = |t: &str| format!("{layer}{t}DamageResonance");
                return [self.g(s, &f("Em")), self.g(s, &f("Thermal")), self.g(s, &f("Kinetic")), self.g(s, &f("Explosive"))];
            }
        };
        [self.g(s, n[0]), self.g(s, n[1]), self.g(s, n[2]), self.g(s, n[3])]
    }

    /// Fit.__generateDrain: capsim drains plus (capUsed GJ/s, capAdded GJ/s)
    pub fn cap_drains(&self) -> (Vec<Drain>, f64, f64) {
        let ds = self.ds;
        let mods = &self.modules;
        let mut drains = Vec::new();
        let (mut used, mut added) = (0.0, 0.0);
        let inj_group = |m: It| ds.group_name(self.items[m].t.group) == "Capacitor Booster";
        for &m in mods {
            if self.items[m].state < ACTIVE {
                continue;
            }
            let need = self.g(m, "capacitorNeed");
            if need == 0.0 {
                continue;
            }
            let full = self.raw_cycle_time(m) + self.g(m, "moduleReactivationDelay");
            if full > 0.0 {
                let cu = self.cap_use(m);
                if cu > 0.0 {
                    used += cu;
                } else {
                    added -= cu;
                }
                drains.push(Drain {
                    duration: full.trunc(),
                    cap_need: need,
                    clip_size: self.num_shots(m),
                    disable_stagger: self.hardpoint(m) == 1,
                    reload_time: self.reload_time(m),
                    is_injector: inj_group(m),
                });
            }
        }
        for d in &self.extra_drains {
            drains.push(Drain { duration: d.0.trunc(), cap_need: d.1, clip_size: d.2, disable_stagger: false, reload_time: d.3, is_injector: false });
            if d.1 > 0.0 {
                used += d.1 / (d.0 / 1000.0);
            } else {
                added += -d.1 / (d.0 / 1000.0);
            }
        }
        (drains, used, added)
    }

    pub(crate) fn cap_recharge_at(&self, pct: f64, capacity: f64, rate_s: f64) -> f64 {
        10.0 / rate_s * pct.sqrt() * (1.0 - pct.sqrt()) * capacity
    }

    pub fn stats(&self, req: &FitRequest) -> Value {
        let ds = self.ds;
        let ship = self.ship;
        let s = |n: &str| self.g(ship, n);
        let online = |m: It| self.items[m].state >= ONLINE;
        let mods = &self.modules;
        let sum_online = |n: &str| mods.iter().filter(|&&m| online(m)).map(|&m| self.g(m, n)).sum::<f64>();
        let cpu_used = py_round_digits(sum_online("cpu"), 2);
        let pg_used = py_round_digits(sum_online("power"), 2);
        let calib = sum_online("upgradeCost");
        let bw: f64 = self.drones.iter().map(|&d| self.g(d, "droneBandwidthUsed") * self.items[d].amount_active as f64).sum();
        let bay: f64 = self.drones.iter().map(|&d| self.items[d].t.attr(161).unwrap_or(0.0) * self.items[d].amount as f64).sum();
        let fbay: f64 = self.fighters.iter().map(|&f| self.items[f].t.attr(161).unwrap_or(0.0) * self.items[f].amount as f64).sum();
        let cargo: f64 = req
            .cargo
            .iter()
            .map(|c| ds.types.get(&c.type_id).and_then(|t| t.attr(161)).unwrap_or(0.0) * c.quantity as f64)
            .sum();
        let usage = |u: f64, t: f64| obj(vec![("used", Value::from(u)), ("total", Value::from(t))]);
        let count = |sl: SlotReq| mods.iter().filter(|&&m| self.slot_of(m) == Some(sl)).count() as f64;
        let hp_used = |h: u8| mods.iter().filter(|&&m| self.hardpoint(m) == h).count() as f64;
        let fclass = |m: It| -> &'static str {
            let f = &self.items[m];
            let a = |n: &str| f.t.attr(ds.attr_id(n)).unwrap_or(0.0) != 0.0;
            if a("fighterSquadronIsHeavy") || a("fighterSquadronIsStandupHeavy") {
                "heavy"
            } else if a("fighterSquadronIsSupport") || a("fighterSquadronIsStandupSupport") {
                "support"
            } else {
                "light"
            }
        };
        let factive: Vec<It> = self.fighters.iter().copied().filter(|&f| self.items[f].active).collect();
        let fcls = |c: &str| factive.iter().filter(|&&f| fclass(f) == c).count() as f64;
        let resources = obj(vec![("cpu", Value::from(usage(cpu_used, s("cpuOutput")))), ("power", Value::from(usage(pg_used, s("powerOutput")))), ("calibration", Value::from(usage(calib, s("upgradeCapacity")))), ("drone_bandwidth", Value::from(usage(bw, s("droneBandwidth")))), ("drone_bay", Value::from(usage(bay, s("droneCapacity")))), ("fighter_bay", Value::from(usage(fbay, s("fighterCapacity")))), ("cargo", Value::from(usage(cargo, s("capacity")))), ("slots", obj(vec![("high", Value::from(usage(count(SlotReq::High), s("hiSlots")))), ("mid", Value::from(usage(count(SlotReq::Mid), s("medSlots")))), ("low", Value::from(usage(count(SlotReq::Low), s("lowSlots")))), ("rig", Value::from(usage(count(SlotReq::Rig), s("rigSlots")))), ("subsystem", Value::from(usage(count(SlotReq::Subsystem), s("maxSubSystems")))), ("service", Value::from(usage(count(SlotReq::Service), s("serviceSlots"))))])), ("hardpoints", obj(vec![("turret", Value::from(usage(hp_used(1), s("turretSlotsLeft")))), ("launcher", Value::from(usage(hp_used(2), s("launcherSlotsLeft"))))])), ("fighter_tubes", obj(vec![("total", Value::from(usage(factive.len() as f64, s("fighterTubes")))), ("light", Value::from(usage(fcls("light"), s("fighterLightSlots") + s("fighterStandupLightSlots")))), ("support", Value::from(usage(fcls("support"), s("fighterSupportSlots") + s("fighterStandupSupportSlots")))), ("heavy", Value::from(usage(fcls("heavy"), s("fighterHeavySlots") + s("fighterStandupHeavySlots"))))]))]);

        // ---------------- offense
        let spool = req.options.default_spool.or(Some(Spool { kind: SpoolType::SpoolScale, amount: 1.0 }));
        let mut weapons = Vec::new();
        let (mut w_vol, mut w_dps) = (Dmg::default(), Dmg::default());
        for &m in mods {
            let (v, d, cyc) = self.volley_dps_cycle(m, spool);
            w_vol.add(&v);
            w_dps.add(&d);
            if self.items[m].state >= ACTIVE && d.total() > 0.0 {
                let c = self.items[m].charge;
                let mut w = obj(vec![("module_index", Value::from(self.items[m].req_index)), ("type_id", Value::from(self.items[m].t.id)), ("name", Value::from(self.items[m].t.name.clone())), ("kind", Value::from(self.weapon_kind(m))), ("charge_type_id", Value::from(if c != NONE { Some(self.items[c].t.id) } else { None })), ("volley", Value::from(v.json())), ("dps", Value::from(d.json())), ("cycle_time_ms", Value::from(cyc))]);
                match self.hardpoint(m) {
                    1 => {
                        w["optimal_m"] = Value::from(self.max_range(m));
                        w["falloff_m"] = Value::from(self.falloff(m));
                        w["tracking"] = Value::from(self.g(m, "trackingSpeed"));
                    }
                    2 if c != NONE => {
                        w["range_m"] = Value::from(self.max_range(m));
                        w["explosion_radius"] = Value::from(self.g(c, "aoeCloudSize"));
                        w["explosion_velocity"] = Value::from(self.g(c, "aoeVelocity"));
                    }
                    _ => {}
                }
                weapons.push(w);
            }
        }
        let (mut d_vol, mut d_dps) = (Dmg::default(), Dmg::default());
        let mut drone_out = Vec::new();
        for &d in &self.drones {
            let v = self.drone_volley(d);
            let p = self.drone_dps(d);
            d_vol.add(&v);
            d_dps.add(&p);
            if self.items[d].amount_active > 0 {
                drone_out.push(obj(vec![("drone_index", Value::from(self.items[d].req_index)), ("type_id", Value::from(self.items[d].t.id)), ("name", Value::from(self.items[d].t.name.clone())), ("count", Value::from(self.items[d].amount_active)), ("volley", Value::from(v.json())), ("dps", Value::from(p.json())),
                    ("optimal_m", Value::from(self.drone_max_range(d))), ("falloff_m", Value::from(self.drone_falloff(d))),
                    ("tracking", Value::from(self.g(d, "trackingSpeed"))), ("max_velocity", Value::from(self.g(d, "maxVelocity"))),
                    ("signature_radius", Value::from(self.g(d, "signatureRadius")))]));
            }
        }
        let (mut f_vol, mut f_dps) = (Dmg::default(), Dmg::default());
        let mut fighter_out = Vec::new();
        for &f in &self.fighters {
            let v = self.fighter_volley(f);
            let p = self.fighter_dps(f);
            f_vol.add(&v);
            f_dps.add(&p);
            if self.items[f].active {
                fighter_out.push(obj(vec![("fighter_index", Value::from(self.items[f].req_index)), ("type_id", Value::from(self.items[f].t.id)), ("name", Value::from(self.items[f].t.name.clone())), ("squadron_size", Value::from(self.items[f].amount)), ("volley", Value::from(v.json())), ("dps", Value::from(p.json())),
                    ("max_velocity", Value::from(self.g(f, "maxVelocity"))), ("signature_radius", Value::from(self.g(f, "signatureRadius")))]));
            }
        }
        let mut t_vol = w_vol;
        t_vol.add(&d_vol);
        t_vol.add(&f_vol);
        let mut t_dps = w_dps;
        t_dps.add(&d_dps);
        t_dps.add(&f_dps);
        let tp = req.target_profile.clone().unwrap_or_default();
        let tp_r = [tp.em, tp.thermal, tp.kinetic, tp.explosive];
        let offense = obj(vec![("weapons", Value::from(weapons)), ("drones", Value::from(drone_out)), ("fighters", Value::from(fighter_out)), ("total", obj(vec![("weapon_dps", Value::from(w_dps.total())), ("weapon_volley", Value::from(w_vol.total())), ("drone_dps", Value::from(d_dps.total())), ("drone_volley", Value::from(d_vol.total())), ("fighter_dps", Value::from(f_dps.total())), ("fighter_volley", Value::from(f_vol.total())), ("dps", Value::from(t_dps.json())), ("volley", Value::from(t_vol.json()))])), ("vs_target_profile", obj(vec![("dps", Value::from(t_dps.vs(tp_r))), ("volley", Value::from(t_vol.vs(tp_r)))]))]);

        // ---------------- defense
        let pat = req.damage_pattern.map(|p| [p.em, p.thermal, p.kinetic, p.explosive]).unwrap_or([25.0; 4]);
        let (hs, ha, hh) = (s("shieldCapacity"), s("armorHP"), s("hp"));
        let (es, ea, eh) = (self.effectivify(pat, hs, "shield"), self.effectivify(pat, ha, "armor"), self.effectivify(pat, hh, "hull"));
        let rj = |r: [f64; 4]| obj(vec![("em", Value::from(r[0])), ("thermal", Value::from(r[1])), ("kinetic", Value::from(r[2])), ("explosive", Value::from(r[3]))]);
        let ex = |n: &str| self.attr(ship, extra_attr(n));
        let srr = self.applied_rr(0);
        let arr = self.applied_rr(1);
        let hrr = self.applied_rr(2);
        let passive = {
            let rate = s("shieldRechargeRate") / 1000.0;
            self.cap_recharge_at(0.25, hs, rate)
        };
        let shield_rep = ex("shieldRepair") + srr;
        let armor_rep = ex("armorRepair") + arr;
        let hull_rep = ex("hullRepair") + hrr;
        let mut defense = obj(vec![("hp", obj(vec![("shield", Value::from(hs)), ("armor", Value::from(ha)), ("hull", Value::from(hh)), ("total", Value::from(hs + ha + hh))])), ("ehp", obj(vec![("shield", Value::from(es)), ("armor", Value::from(ea)), ("hull", Value::from(eh)), ("total", Value::from(es + ea + eh))])), ("resonance", obj(vec![("shield", Value::from(rj(self.resonances("shield")))), ("armor", Value::from(rj(self.resonances("armor")))), ("hull", Value::from(rj(self.resonances("hull"))))])), ("damage_pattern", obj(vec![("em", Value::from(pat[0])), ("thermal", Value::from(pat[1])), ("kinetic", Value::from(pat[2])), ("explosive", Value::from(pat[3]))])), ("tank", obj(vec![("raw", obj(vec![("passive_shield", Value::from(passive)), ("shield_repair", Value::from(shield_rep)), ("armor_repair", Value::from(armor_rep)), ("hull_repair", Value::from(hull_rep))])), ("effective", obj(vec![("passive_shield", Value::from(self.effectivify(pat, passive, "shield"))), ("shield_repair", Value::from(self.effectivify(pat, shield_rep, "shield"))), ("armor_repair", Value::from(self.effectivify(pat, armor_rep, "armor"))), ("hull_repair", Value::from(self.effectivify(pat, hull_rep, "hull")))]))]))]);

        // ---------------- modules table
        let mut mrows = Vec::new();
        for &m in mods {
            let st = match self.items[m].state {
                OFFLINE => "offline",
                ONLINE => "online",
                ACTIVE => "active",
                _ => "overheated",
            };
            mrows.push(obj(vec![("module_index", Value::from(self.items[m].req_index)), ("type_id", Value::from(self.items[m].t.id)), ("name", Value::from(self.items[m].t.name.clone())), ("slot", Value::from(self.items[m].slot.map(slot_name))), ("state", Value::from(st)), ("cpu", Value::from(self.g(m, "cpu"))), ("power", Value::from(self.g(m, "power"))), ("cycle_time_ms", Value::from(self.cycle_avg(m, None))), ("cap_use_gj_s", Value::from(self.cap_use(m)))]));
        }

        // ---------------- capacitor (fit.simulateCap)
        let cap = s("capacitorCapacity");
        let rr = s("rechargeRate");
        let (drains, used, added) = self.cap_drains();
        let peak = self.cap_recharge_at(0.25, cap, rr / 1000.0);
        let recharge = added + peak;
        let mut capj = obj(vec![("capacity", Value::from(cap)), ("recharge_time_s", Value::from(rr / 1000.0)), ("peak_recharge_gj_s", Value::from(peak)), ("use_gj_s", Value::from(used)), ("injected_gj_s", Value::from(added)), ("delta_gj_s", Value::from(recharge - used))]);
        let mut cap_stable = true;
        if drains.is_empty() {
            capj["stable"] = Value::from(true);
            capj["stable_percent"] = Value::from(100.0);
            capj["sim_iterations"] = Value::from(0);
        } else {
            let t_max = req.options.cap_sim.max_time_s.map(|x| x * 1000.0).unwrap_or(6.0 * 3600.0 * 1000.0);
            let r = capsim::run(&drains, cap, rr, cap, t_max, self.factor_reload, true);
            let st = (r.cap_stable_low + r.cap_stable_high) / (2.0 * cap);
            let stable = st > 0.0;
            cap_stable = stable;
            capj["stable"] = Value::from(stable);
            if stable {
                capj["stable_percent"] = Value::from((st * 100.0).min(100.0));
            } else {
                capj["depletes_in_s"] = Value::from(r.t / 1000.0);
            }
            capj["eve_stable_percent"] = Value::from(r.cap_stable_eve * 100.0);
            capj["sim_iterations"] = Value::from(r.iterations);
        }

        // ---------------- sustained tank (Fit.calculateSustainableTank)
        let sus = self.sustainable_tank([shield_rep, armor_rep, hull_rep], cap_stable, used, recharge);
        defense["tank"]["sustained"] = obj(vec![("passive_shield", Value::from(passive)), ("shield_repair", Value::from(sus[0])),
            ("armor_repair", Value::from(sus[1])), ("hull_repair", Value::from(sus[2]))]);
        defense["tank"]["sustained_effective"] = obj(vec![("passive_shield", Value::from(self.effectivify(pat, passive, "shield"))), ("shield_repair", Value::from(self.effectivify(pat, sus[0], "shield"))),
            ("armor_repair", Value::from(self.effectivify(pat, sus[1], "armor"))), ("hull_repair", Value::from(self.effectivify(pat, sus[2], "hull")))]);

        // ---------------- navigation
        let speed_limit = s("speedLimit");
        let mut vmax = s("maxVelocity");
        if speed_limit != 0.0 && vmax > speed_limit {
            vmax = speed_limit;
        }
        let agility = s("agility");
        let mass = s("mass");
        let base_warp = if s("baseWarpSpeed") != 0.0 { s("baseWarpSpeed") } else { 1.0 };
        let wm = if s("warpSpeedMultiplier") != 0.0 { s("warpSpeedMultiplier") } else { 1.0 };
        let wneed = s("warpCapacitorNeed");
        let navigation = obj(vec![("max_velocity", Value::from(vmax)), ("align_time_s", Value::from(-(0.25f64.ln()) * agility * mass / 1e6)), ("mass", Value::from(mass)), ("agility", Value::from(agility)), ("signature_radius", Value::from(s("signatureRadius"))), ("warp_speed_au_s", Value::from(base_warp * wm)), ("max_warp_distance_au", Value::from(if wneed != 0.0 { cap / (mass * wneed) } else { 0.0 })), ("warp_scramble_status", Value::from(s("warpScrambleStatus")))]);

        // ---------------- targeting
        let mt = ex("maxTargetsLockedFromSkills").min(s("maxLockedTargets"));
        let max_targets = float_unerr(mt).ceil();
        let mut best = ("", -1.0f64);
        for (n, a) in [("magnetometric", "scanMagnetometricStrength"), ("ladar", "scanLadarStrength"), ("radar", "scanRadarStrength"), ("gravimetric", "scanGravimetricStrength")] {
            let v = s(a);
            if v > best.1 {
                best = (n, v);
            } else if v == best.1 {
                best = ("multispectral", v);
            }
        }
        let sig = s("signatureRadius");
        let scan_res = s("scanResolution");
        let lt = |r: f64| -> Option<f64> {
            if scan_res > 0.0 && r > 0.0 {
                let v = 40000.0 / scan_res / (r + (r * r + 1.0).sqrt()).ln().powi(2);
                Some(v.max(0.0))
            } else {
                None
            }
        };
        let jam = {
            let mut retain = 1.0;
            for j in &self.ecm {
                retain *= 1.0 - (j / best.1).min(1.0);
            }
            (1.0 - retain) * 100.0
        };
        let targeting = obj(vec![("jam_chance_percent", Value::from(jam)), ("max_targets", Value::from(max_targets)), ("max_range_m", Value::from(s("maxTargetRange"))), ("scan_resolution", Value::from(scan_res)), ("sensor_strength", Value::from(best.1)), ("sensor_type", Value::from(best.0)), ("probe_size", Value::from(if best.1 != 0.0 { Some((sig / best.1).max(1.08)) } else { None })), ("lock_time_s", obj(vec![("sig_25m", Value::from(lt(25.0))), ("sig_40m", Value::from(lt(40.0))), ("sig_125m", Value::from(lt(125.0))), ("sig_400m", Value::from(lt(400.0))), ("sig_target_profile", Value::from(tp.signature_radius.and_then(lt)))]))]);
        let drones_j = obj(vec![("active", Value::from(self.drones.iter().map(|&d| self.items[d].amount_active).sum::<u32>())), ("max_active", Value::from(ex_or(self, "maxActiveDrones"))), ("control_range_m", Value::from(ex("droneControlRange")))]);

        let st = self.items[ship].t;
        let mut out = Map::new();
        out.insert("meta", obj(vec![("schema_version", Value::from(1)), ("engine", Value::from(concat!("eve-dogma-e ", env!("CARGO_PKG_VERSION"), " (pyfa port)"))), ("sde_build", Value::from(ds.build)), ("dataset_sha256", Value::from(ds.sha256.clone()))]));
        out.insert("ship", obj(vec![("type_id", Value::from(st.id)), ("name", Value::from(st.name.to_string())), ("group", Value::from(ds.group_name(st.group).to_string()))]));
        out.insert("resources", resources);
        out.insert("modules", Value::Array(mrows));
        out.insert("offense", offense);
        out.insert("defense", defense);
        out.insert("capacitor", capj);
        out.insert("navigation", navigation);
        out.insert("targeting", targeting);
        out.insert("drones", drones_j);
        out.insert("violations", Value::Array(if req.options.validate { self.violations(cpu_used, pg_used, calib, bw) } else { vec![] }));
        out.insert("warnings", Value::from(self.warnings.clone()));
        if let Some(inc) = &req.options.include_attributes {
            out.insert("attributes", self.dump_attributes(inc));
        }
        Value::Object(out)
    }

    /// Fit.__getAppliedRr for shield (0), armor (1), hull (2)
    pub(crate) fn applied_rr(&self, kind: usize) -> f64 {
        let list: Vec<(f64, f64)> = self.rr.iter().filter(|r| r.0 == kind).map(|r| (r.1, r.2)).collect();
        let total: f64 = list.iter().map(|(a, c)| a / c.trunc()).sum();
        let mut out = 0.0;
        for (a, c) in &list {
            let rrps = a / c.trunc();
            let m = 7000.0 + rrps * 20.0;
            let mult = 1.0 - (((rrps + m) / (total + m)) - 1.0).powi(2);
            out += mult * a / c;
        }
        out
    }

    pub(crate) fn violations(&self, cpu: f64, pg: f64, calib: f64, bw: f64) -> Vec<Value> {
        let mut v = Vec::new();
        let ship = self.ship;
        let s = |n: &str| self.g(ship, n);
        let mut push = |code: &'static str, msg: String, idx: Option<usize>| v.push(obj(vec![("code", Value::from(code)), ("message", Value::from(msg)), ("module_index", Value::from(idx))]));
        if cpu > s("cpuOutput") + 1e-9 {
            push("CPU_OVERLOAD", format!("CPU {cpu} > {}", s("cpuOutput")), None);
        }
        if pg > s("powerOutput") + 1e-9 {
            push("POWER_OVERLOAD", format!("powergrid {pg} > {}", s("powerOutput")), None);
        }
        if calib > s("upgradeCapacity") + 1e-9 {
            push("CALIBRATION_OVERLOAD", format!("calibration {calib} > {}", s("upgradeCapacity")), None);
        }
        if bw > s("droneBandwidth") + 1e-9 {
            push("DRONE_BANDWIDTH", format!("drone bandwidth {bw} > {}", s("droneBandwidth")), None);
        }
        for (sl, a) in [(SlotReq::High, "hiSlots"), (SlotReq::Mid, "medSlots"), (SlotReq::Low, "lowSlots"), (SlotReq::Rig, "rigSlots"), (SlotReq::Subsystem, "maxSubSystems"), (SlotReq::Service, "serviceSlots")] {
            let n = self.modules.iter().filter(|&&m| self.items[m].slot == Some(sl)).count() as f64;
            if n > s(a) {
                push("SLOTS_EXCEEDED", format!("{} {} slots used, {} available", n, slot_name(sl), s(a)), None);
            }
        }
        let tur = self.modules.iter().filter(|&&m| self.hardpoint(m) == 1).count() as f64;
        let lau = self.modules.iter().filter(|&&m| self.hardpoint(m) == 2).count() as f64;
        if tur > s("turretSlotsLeft") {
            push("TURRET_HARDPOINTS", format!("{tur} turrets, {} hardpoints", s("turretSlotsLeft")), None);
        }
        if lau > s("launcherSlotsLeft") {
            push("LAUNCHER_HARDPOINTS", format!("{lau} launchers, {} hardpoints", s("launcherSlotsLeft")), None);
        }
        for &m in &self.modules {
            for (sk, lvl) in &self.items[m].t.req_skills {
                if self.skill_level(*sk) < *lvl as f64 {
                    push("MISSING_SKILL", format!("{} requires skill {} level {}", self.items[m].t.name, sk, lvl), Some(self.items[m].req_index));
                    break;
                }
            }
        }
        v
    }

    pub(crate) fn dump_attributes(&self, inc: &str) -> Value {
        let ds = self.ds;
        let want: Option<Vec<u32>> = if inc == "all" { None } else { Some(inc.split(',').map(|n| ds.attr_id(n.trim())).filter(|&x| x != 0).collect()) };
        let dump = |it: It| -> Value {
            let mut m = Map::new();
            let mut ids: Vec<u32> = self.items[it].t.attrs.iter().map(|x| x.0).collect();
            for k in self.items[it].mad.entries.keys() {
                ids.push(*k);
            }
            ids.sort();
            ids.dedup();
            for a in ids {
                if let Some(w) = &want {
                    if !w.contains(&a) {
                        continue;
                    }
                }
                if let (Some(info), Some(v)) = (ds.attrs.get(&a), self.attr_opt(it, a)) {
                    m.insert(info.name.clone(), Value::from(v));
                }
            }
            Value::Object(m)
        };
        let modules: Vec<Value> = self
            .modules
            .iter()
            .map(|&m| {
                let c = self.items[m].charge;
                obj(vec![("module_index", Value::from(self.items[m].req_index)), ("type_id", Value::from(self.items[m].t.id)), ("attributes", Value::from(dump(m))), ("charge", Value::from(if c != NONE { Some(dump(c)) } else { None }))])
            })
            .collect();
        let drones: Vec<Value> = self.drones.iter().map(|&d| obj(vec![("drone_index", Value::from(self.items[d].req_index)), ("attributes", Value::from(dump(d)))])).collect();
        obj(vec![("ship", Value::from(dump(self.ship))), ("modules", Value::from(modules)), ("drones", Value::from(drones))])
    }
}

fn ex_or(f: &Fit, n: &str) -> f64 {
    // Pyfa keeps maxActiveDrones in the ship's extra attributes (character skills add to it)
    let s = f.ship;
    f.g(s, n)
}

pub fn slot_name(s: SlotReq) -> &'static str {
    match s {
        SlotReq::High => "high",
        SlotReq::Mid => "mid",
        SlotReq::Low => "low",
        SlotReq::Rig => "rig",
        SlotReq::Subsystem => "subsystem",
        SlotReq::Service => "service",
    }
}

#[allow(dead_code)]
fn unused(_: &Ids) {}


#[cfg(test)]
mod round_tests {
    #[test]
    pub(crate) fn fast_round_matches_format() {
        let mut x: u64 = 0x9E3779B97F4A7C15;
        for i in 0..2_000_000u64 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let mant = (x >> 11) as f64 / (1u64 << 53) as f64;
            let e = ((x & 0xff) as i32 % 24) - 12;
            let mut v = mant * 10f64.powi(e);
            if i % 3 == 0 {
                v = -v;
            }
            if i % 7 == 0 {
                v = (v * 1000.0).round() / 1000.0 + 0.0005;
            }
            for n in 0..16 {
                let want: f64 = format!("{:.*}", n as usize, v).parse().unwrap();
                let got = super::py_round_digits(v, n);
                assert_eq!(got.to_bits(), want.to_bits(), "v={v:e} n={n}");
            }
        }
    }
}
