//! `remote_reps` graph (Pyfa "Remote Repairs"): outgoing remote repair vs distance or time.
use super::common::*;
use super::cycles::{Cycles, module_cycles};
use super::{GErr, GraphRequest, in_range};
use crate::api::build_calc;
use crate::data::Dataset;
use crate::eos::cx::{Fit, It, calc_range_factor, ACTIVE};
use crate::eos::stats::{calculate_spoolup, float_unerr};
use crate::request::{Spool, SpoolType};

/// [shield, armor, hull, capacitor]
type Rr = [f64; 4];

fn rr_sum(a: &Rr) -> f64 {
    a[0] + a[1] + a[2]
}
fn any_rep(a: &Rr) -> bool {
    a[0] > 0.0 || a[1] > 0.0 || a[2] > 0.0
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Module,
    Drone,
}

struct Key {
    it: It,
    kind: Kind,
    anc_shield: bool,
    anc_armor: bool,
}

fn rr_type(fit: &Fit, m: It) -> Option<&'static str> {
    Some(match fit.ds.group_name(fit.items[m].t.group) {
        "Remote Armor Repairer" | "Ancillary Remote Armor Repairer" | "Mutadaptive Remote Armor Repairer" => "Armor",
        "Remote Hull Repairer" => "Hull",
        "Remote Shield Booster" | "Ancillary Remote Shield Booster" => "Shield",
        "Remote Capacitor Transmitter" => "Capacitor",
        _ => return None,
    })
}

/// Module.getRepAmountParameters: [(delay ms, amounts)]
fn module_rep_params(fit: &Fit, m: It, spool: Option<Spool>, forced: bool) -> Vec<(f64, Rr)> {
    if fit.items[m].state < ACTIVE {
        return vec![];
    }
    let Some(t) = rr_type(fit, m) else { return vec![] };
    let mut a = [0.0; 4];
    match t {
        "Hull" => a[2] = fit.g(m, "structureDamageAmount"),
        "Armor" => {
            let mult = if fit.ds.group_name(fit.items[m].t.group) == "Ancillary Remote Armor Repairer" && fit.items[m].charge != crate::eos::cx::NONE {
                fit.gd(m, "chargedArmorDamageMultiplier", 1.0)
            } else {
                1.0
            };
            a[1] = fit.g(m, "armorDamageAmount") * mult
        }
        "Shield" => a[0] = fit.g(m, "shieldBonus"),
        _ => a[3] = fit.g(m, "powerTransferAmount"),
    }
    let delay = if t == "Shield" { 0.0 } else { fit.raw_cycle_time(m) };
    let sp = if forced { spool } else { fit.items[m].spool.or(spool) };
    let boost = calculate_spoolup(fit.g(m, "repairMultiplierBonusMax"), fit.g(m, "repairMultiplierBonusPerCycle"), fit.raw_cycle_time(m) / 1000.0, sp).0;
    let k = 1.0 + boost;
    if k != 1.0 {
        for x in a.iter_mut() {
            *x *= k;
        }
    }
    vec![(delay, a)]
}

fn drone_cycle_time(fit: &Fit, d: It) -> f64 {
    fit.drone_cycle(d)
}

fn drone_rep_params(fit: &Fit, d: It) -> Vec<(f64, Rr)> {
    let n = fit.items[d].amount_active as f64;
    if n <= 0.0 {
        return vec![];
    }
    let mut v = Vec::new();
    let (h, a, s) = (fit.g(d, "structureDamageAmount"), fit.g(d, "armorDamageAmount"), fit.g(d, "shieldBonus"));
    if s != 0.0 {
        v.push((0.0, [s * n, 0.0, 0.0, 0.0]));
    }
    if a != 0.0 || h != 0.0 {
        let ct = drone_cycle_time(fit, d);
        let e = [0.0, a * n, h * n, 0.0];
        match v.iter_mut().find(|x| x.0 == ct) {
            Some(x) => x.1 = e,
            None => v.push((ct, e)),
        }
    }
    v
}

fn keys(fit: &Fit) -> Vec<Key> {
    let mut v = Vec::new();
    for m in active_modules(fit) {
        let p = module_rep_params(fit, m, None, false);
        if p.iter().any(|x| x.1.iter().any(|&y| y != 0.0)) {
            v.push(Key { it: m, kind: Kind::Module, anc_shield: has_effect(fit, m, "shipModuleAncillaryRemoteShieldBooster"),
                         anc_armor: has_effect(fit, m, "shipModuleAncillaryRemoteArmorRepairer") });
        }
    }
    for d in active_drones(fit) {
        if drone_rep_params(fit, d).iter().any(|x| x.1.iter().any(|&y| y != 0.0)) {
            v.push(Key { it: d, kind: Kind::Drone, anc_shield: false, anc_armor: false });
        }
    }
    v
}

fn application(fit: &Fit, req: &GraphRequest, k: &Key, d: Option<f64>) -> f64 {
    let lock = in_lock_range(fit, req.settings.ignore_lock_range, d);
    let v = match k.kind {
        Kind::Module => {
            if !lock {
                0.0
            } else {
                calc_range_factor(fit.max_range(k.it).unwrap_or(0.0), fit.falloff(k.it).unwrap_or(0.0), d, true)
            }
        }
        Kind::Drone => {
            if !lock || !in_drone_range(fit, req.settings.ignore_drone_control_range, d) {
                0.0
            } else {
                1.0
            }
        }
    };
    float_unerr(v)
}

/// per-key time series: rps change points [(t, rps)] and rep events [(t, amount)]
struct Series {
    rps: Vec<(f64, Rr)>,
    amt: Vec<(f64, Rr)>,
}

fn time_series(fit: &Fit, k: &Key, anc_reload: bool, max_t: f64) -> Series {
    let mut rps_list: Vec<(f64, f64, Rr)> = Vec::new();
    let mut amt: Vec<(f64, Rr)> = Vec::new();
    let mut add_amt = |t: f64, a: Rr| {
        if any_rep(&a) {
            match amt.iter_mut().find(|x| x.0 == t) {
                Some(x) => x.1 = a,
                None => amt.push((t, a)),
            }
        }
    };
    let mut add_rps = |t0: f64, t1: f64, list: &[Rr]| {
        if list.is_empty() {
            return;
        }
        let mut s = [0.0; 4];
        for a in list {
            for i in 0..4 {
                s[i] += a[i];
            }
        }
        if any_rep(&s) {
            let d = t1 - t0;
            rps_list.push((t0, t1, [s[0] / d, s[1] / d, s[2] / d, s[3] / d]));
        }
    };
    match k.kind {
        Kind::Module => {
            let m = k.it;
            let cyc = if k.anc_shield || k.anc_armor { module_cycles(fit, m, Some(anc_reload)) } else { module_cycles(fit, m, Some(true)) };
            if let Some(cyc) = cyc {
                let mut t = 0.0;
                let mut nonstop = 0.0;
                let mut without_reload = 0.0;
                let until = fit.num_shots(m);
                for (ct, it, is_rel) in cyc.iter() {
                    without_reload += 1.0;
                    let mut list = Vec::new();
                    let sp = Some(Spool { kind: SpoolType::Cycles, amount: nonstop });
                    for (rt, mut a) in module_rep_params(fit, m, sp, true) {
                        if k.anc_armor && fit.items[m].charge != crate::eos::cx::NONE && !anc_reload && without_reload > until {
                            let d = fit.gd(m, "chargedArmorDamageMultiplier", 1.0);
                            for x in a.iter_mut() {
                                *x /= d;
                            }
                        }
                        list.push(a);
                        add_amt(t + rt / 1000.0, a);
                    }
                    add_rps(t, t + ct / 1000.0, &list);
                    if it > 0.0 {
                        nonstop = 0.0;
                    } else {
                        nonstop += 1.0;
                    }
                    if is_rel {
                        without_reload = 0.0;
                    }
                    if t > max_t {
                        break;
                    }
                    t += ct / 1000.0 + it / 1000.0;
                }
            }
        }
        Kind::Drone => {
            let d = k.it;
            let ct0 = drone_cycle_time(fit, d);
            if ct0 != 0.0 {
                let cyc = Cycles::single(ct0, 0.0, f64::INFINITY, false);
                let params = drone_rep_params(fit, d);
                let mut t = 0.0;
                for (ct, it, _) in cyc.iter() {
                    let mut list = Vec::new();
                    for &(rt, a) in &params {
                        list.push(a);
                        add_amt(t + rt / 1000.0, a);
                    }
                    add_rps(t, t + ct / 1000.0, &list);
                    if t > max_t {
                        break;
                    }
                    t += ct / 1000.0 + it / 1000.0;
                }
            }
        }
    }
    // rps change points
    let mut pts: Vec<(f64, Rr)> = Vec::new();
    let mut prev: Option<(f64, Rr)> = None; // (end, rps)
    for &(t0, t1, r) in &rps_list {
        match prev {
            None => pts.push((t0, r)),
            Some((pe, pr)) => {
                if float_unerr(pe) < float_unerr(t0) {
                    pts.push((pe, [0.0; 4]));
                    pts.push((t0, r));
                } else if r != pr {
                    pts.push((t0, r));
                }
            }
        }
        prev = Some((t1, r));
    }
    Series { rps: pts, amt }
}

pub fn run(ds: &Dataset, req: &GraphRequest) -> Result<Vec<(String, Vec<Option<f64>>)>, GErr> {
    let fit = build_calc(ds, &req.fit)?;
    let anc = req.pb("anc_reload", true);
    let ptime = req.p("time_s").map(|t| t.clamp(0.0, 2500.0));
    let pdist = req.p("distance_m");
    let keys = keys(&fit);
    let time_axis = req.x.axis == "time_s";
    let mut out = Vec::new();
    for y in &req.y {
        let mut vals = Vec::new();
        for &x in &req.x.values {
            if time_axis && !in_range(x, 0.0, 2500.0) {
                vals.push(None);
                continue;
            }
            let (t, d) = if time_axis { (Some(x), pdist) } else { (ptime, Some(x)) };
            if y == "total" && t.is_none() {
                vals.push(None);
                continue;
            }
            let mut total = [0.0; 4];
            for k in &keys {
                let a = application(&fit, req, k, d);
                let r: Rr = match t {
                    None => {
                        // stats-panel rps (default spool, not forced)
                        let default = Some(Spool { kind: SpoolType::SpoolScale, amount: 1.0 });
                        match k.kind {
                            Kind::Module => {
                                let ro = if k.anc_shield || k.anc_armor { Some(anc) } else { None };
                                let mut s = [0.0; 4];
                                if let Some(c) = module_cycles(&fit, k.it, ro) {
                                    let avg = c.average();
                                    let p = module_rep_params(&fit, k.it, default, false);
                                    if !p.is_empty() && avg != 0.0 {
                                        for (_, x) in p {
                                            for i in 0..4 {
                                                s[i] += x[i];
                                            }
                                        }
                                        for v in s.iter_mut() {
                                            *v *= 1.0 / (avg / 1000.0);
                                        }
                                    }
                                }
                                s
                            }
                            Kind::Drone => {
                                let ct = drone_cycle_time(&fit, k.it);
                                let mut s = [0.0; 4];
                                if ct != 0.0 {
                                    for (_, x) in drone_rep_params(&fit, k.it) {
                                        for i in 0..4 {
                                            s[i] += x[i];
                                        }
                                    }
                                    for v in s.iter_mut() {
                                        *v *= 1.0 / (ct / 1000.0);
                                    }
                                }
                                s
                            }
                        }
                    }
                    Some(t) => {
                        let s = time_series(&fit, k, anc, t);
                        let tu = float_unerr(t);
                        if y == "rps" {
                            s.rps.iter().filter(|p| float_unerr(p.0) <= tu).map(|p| p.1).next_back().unwrap_or([0.0; 4])
                        } else {
                            let mut acc = [0.0; 4];
                            for p in s.amt.iter().filter(|p| float_unerr(p.0) <= tu) {
                                for i in 0..4 {
                                    acc[i] += p.1[i];
                                }
                            }
                            acc
                        }
                    }
                };
                for i in 0..4 {
                    total[i] += r[i] * a;
                }
            }
            vals.push(Some(rr_sum(&total)));
        }
        out.push((y.clone(), vals));
    }
    Ok(out)
}
