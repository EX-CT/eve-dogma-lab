//! Fit-only graphs: mobility, warp time, lock time, capacitor, shield regeneration.
use super::{GErr, GraphRequest, in_range};
use crate::api::build_calc;
use crate::data::Dataset;
use crate::eos::capsim;
use crate::eos::cx::{ACTIVE, Fit};
use crate::request::StateReq;

pub const AU_M: f64 = 149597870700.0;

type Series = Vec<(String, Vec<Option<f64>>)>;

pub fn run(ds: &Dataset, req: &GraphRequest) -> Result<Series, GErr> {
    let fit = build_calc(ds, &req.fit)?;
    let xs = &req.x.values;
    let mut out = Vec::new();
    for y in &req.y {
        let v: Vec<Option<f64>> = match req.graph.as_str() {
            "mobility" => mobility(&fit, req, y, xs),
            "warp_time" => warp(ds, &fit, req, xs)?,
            "lock_time" => xs.iter().map(|&x| lock_time(&fit, x)).collect(),
            "capacitor" => capacitor(&fit, req, y, xs),
            "shield_regen" => shield(&fit, req, y, xs),
            _ => unreachable!(),
        };
        out.push((y.clone(), v));
    }
    Ok(out)
}

/// Charging curve shared by capacitor and shield: amount after `t` s from `a0`.
pub fn regen_amount(max: f64, tau_s: f64, a0: f64, t: f64) -> f64 {
    max * (1.0 + (5.0 * -t / tau_s).exp() * ((a0 / max).sqrt() - 1.0)).powi(2)
}
/// Regeneration rate at amount `a`.
pub fn regen_rate(max: f64, tau_s: f64, a: f64) -> f64 {
    10.0 * max / tau_s * ((a / max).sqrt() - a / max)
}

fn mobility(fit: &Fit, req: &GraphRequest, y: &str, xs: &[f64]) -> Vec<Option<f64>> {
    let ship = fit.ship;
    let v = fit.g(ship, "maxVelocity");
    let mass = fit.g(ship, "mass");
    let ag = fit.g(ship, "agility");
    let tgt_mass = req.pd("tgt_mass_kg", 1300e6) / 1e6;
    let inertia = req.pd("tgt_inertia", 0.015);
    let k = ag * mass;
    let speed = |t: f64| v * (1.0 - ((-t * 1e6) / k).exp());
    xs.iter()
        .map(|&t| {
            Some(match y {
                "speed_mps" => speed(t),
                "distance_m" => {
                    let d = |t: f64| v * t + (v * k * ((-t * 1e6) / k).exp() / 1e6);
                    d(t) - d(0.0)
                }
                "momentum_kg_mps" => speed(t) * mass,
                "bump_speed_mps" | "bump_distance_m" => {
                    let bm = mass / 1e6;
                    let s = 2.0 * speed(t) * bm / (bm + tgt_mass);
                    if y == "bump_speed_mps" { s } else { s * tgt_mass * inertia }
                }
                _ => return None,
            })
        })
        .collect()
}

/// Pyfa fitWarpTime: EVE University warp model.
pub fn time_in_warp(warp_au_s: f64, subwarp: f64, dist: f64) -> f64 {
    if dist == 0.0 {
        return 0.0;
    }
    let ka = warp_au_s;
    let kd = (warp_au_s / 3.0).min(2.0);
    let dropout = (subwarp / 2.0).min(100.0);
    let mut vmax = warp_au_s * AU_M;
    let min_dist = AU_M + vmax / kd;
    let mut cruise = 0.0;
    if min_dist > dist {
        vmax = dist * ka * kd / (ka + kd);
    } else {
        cruise = (dist - min_dist) / vmax;
    }
    cruise + (vmax / ka).ln() / ka + (vmax / dropout).ln() / kd
}

const SUBWARP_OFF_GROUPS: &[&str] = &["Propulsion Module", "Mass Entanglers", "Cloaking Device", "Siege Module", "Super Weapon",
    "Cynosural Field Generator", "Clone Vat Bay", "Jump Portal Generator"];

/// Subwarp speed: max velocity with prop/cloak/siege/... modules online and projections switched off.
fn subwarp_speed(ds: &Dataset, fit: &Fit, req: &GraphRequest) -> Result<f64, GErr> {
    let mut r = req.fit.clone();
    let mut changed = !r.projected.is_empty();
    r.projected.clear();
    for &m in &fit.modules {
        let it = &fit.items[m];
        if it.state >= ACTIVE && SUBWARP_OFF_GROUPS.contains(&ds.group_name(it.t.group)) {
            r.modules[it.req_index].state = Some(StateReq::Online);
            changed = true;
        }
    }
    if !changed {
        return Ok(fit.g(fit.ship, "maxVelocity"));
    }
    let f2 = build_calc(ds, &r)?;
    Ok(f2.g(f2.ship, "maxVelocity"))
}

fn warp(ds: &Dataset, fit: &Fit, req: &GraphRequest, xs: &[f64]) -> Result<Vec<Option<f64>>, GErr> {
    let s = |n: &str| fit.g(fit.ship, n);
    let base = if s("baseWarpSpeed") != 0.0 { s("baseWarpSpeed") } else { 1.0 };
    let mult = if s("warpSpeedMultiplier") != 0.0 { s("warpSpeedMultiplier") } else { 1.0 };
    let need = s("warpCapacitorNeed");
    let max_au = if need != 0.0 { s("capacitorCapacity") / (s("mass") * need) } else { 0.0 };
    let sub = subwarp_speed(ds, fit, req)?;
    Ok(xs.iter().map(|&x| if in_range(x, 0.0, max_au * AU_M) { Some(time_in_warp(base * mult, sub, x)) } else { None }).collect())
}

fn lock_time(fit: &Fit, sig: f64) -> Option<f64> {
    if sig < 1.0 {
        return None;
    }
    let sr = fit.g(fit.ship, "scanResolution");
    if sr > 0.0 {
        if sig == 0.0 {
            return None;
        }
        Some((40000.0 / sr / sig.asinh().powi(2)).min(1800.0))
    } else {
        Some(fit.g(fit.ship, "scanSpeed") / 1000.0)
    }
}

fn capacitor(fit: &Fit, req: &GraphRequest, y: &str, xs: &[f64]) -> Vec<Option<f64>> {
    let cmax = fit.g(fit.ship, "capacitorCapacity");
    let rr = fit.g(fit.ship, "rechargeRate");
    let tau = rr / 1000.0;
    let c0 = req.pd("cap_start_pct", 100.0).clamp(0.0, 100.0) / 100.0 * cmax;
    let use_sim = req.pb("use_capsim", true);
    if req.x.axis == "cap_pct" {
        return xs
            .iter()
            .map(|&x| {
                if !in_range(x, 0.0, 100.0) {
                    return None;
                }
                let a = x / 100.0 * cmax;
                Some(if y == "cap_gj" { a } else { regen_rate(cmax, tau, a) })
            })
            .collect();
    }
    if y == "cap_regen_gj_s" {
        return xs.iter().map(|&t| if in_range(t, 0.0, 3600.0) { Some(regen_rate(cmax, tau, regen_amount(cmax, tau, c0, t))) } else { None }).collect();
    }
    let mut saved = Vec::new();
    if use_sim {
        let (drains, _, _) = fit.cap_drains();
        if !drains.is_empty() {
            capsim::run_ex(&drains, cmax, rr, c0, 3600.0 * 1000.0, fit.factor_reload, true, false, Some(&mut saved));
        }
    }
    xs.iter()
        .map(|&t| {
            if !in_range(t, 0.0, 3600.0) {
                return None;
            }
            if saved.is_empty() {
                return Some(regen_amount(cmax, tau, c0, t));
            }
            let last_t = saved[saved.len() - 1].0;
            // last saved point <= t
            let n = saved.partition_point(|p| p.0 <= t);
            if n == 0 {
                return Some(regen_amount(cmax, tau, c0, t));
            }
            let (tb, cb) = saved[n - 1];
            if tb == last_t {
                return None;
            }
            Some(if tb == t { cb } else { regen_amount(cmax, tau, cb, t - tb) })
        })
        .collect()
}

fn shield(fit: &Fit, req: &GraphRequest, y: &str, xs: &[f64]) -> Vec<Option<f64>> {
    let smax = fit.g(fit.ship, "shieldCapacity");
    let tau = fit.g(fit.ship, "shieldRechargeRate") / 1000.0;
    let s0 = req.pd("shield_start_pct", 0.0).clamp(0.0, 100.0) / 100.0 * smax;
    let eff = req.pb("effective", false);
    let pat = req.fit.damage_pattern.map(|p| [p.em, p.thermal, p.kinetic, p.explosive]).unwrap_or([25.0; 4]);
    xs.iter()
        .map(|&x| {
            let amount = if req.x.axis == "shield_pct" {
                if !in_range(x, 0.0, 100.0) {
                    return None;
                }
                x / 100.0 * smax
            } else {
                regen_amount(smax, tau, s0, x)
            };
            let v = if y == "shield_hp" { amount } else { regen_rate(smax, tau, amount) };
            Some(if eff { fit.effectivify(pat, v, "shield") } else { v })
        })
        .collect()
}
