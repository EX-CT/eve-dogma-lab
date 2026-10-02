//! Shared helpers for graph calculations (Pyfa eos.calc / graphs.calc behaviour).
use crate::eos::cx::{Fit, It};
use crate::eos::fit::extra_attr;

/// Stacking-penalised product of multipliers per stacking group (Pyfa `calculateMultiplier`): bonuses and
/// penalties separately, strongest first, the i-th one weighted by exp(-i²/7.1289).
pub fn calc_multiplier(groups: &[Vec<f64>]) -> f64 {
    let mut val = 1.0;
    for g in groups {
        let mut up: Vec<f64> = g.iter().copied().filter(|&v| v > 1.0).collect();
        let mut down: Vec<f64> = g.iter().copied().filter(|&v| v < 1.0).collect();
        let key = |v: &f64| -(v - 1.0).abs();
        up.sort_by(|a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
        down.sort_by(|a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
        for l in [&up, &down] {
            for (i, b) in l.iter().enumerate() {
                val *= 1.0 + (b - 1.0) * (-((i * i) as f64) / 7.1289).exp();
            }
        }
    }
    val
}

pub fn has_effect(fit: &Fit, it: It, name: &str) -> bool {
    let id = fit.ds.effect_id(name);
    id != 0 && fit.items[it].effects.contains(&id)
}

pub fn effect_name<'a>(fit: &Fit<'a>, e: u32) -> &'a str {
    fit.ds.effects.get(&e).map(|x| x.name.as_str()).unwrap_or("")
}

/// Pyfa graphs.calc checkLockRange
pub fn in_lock_range(fit: &Fit, ignore: bool, d: Option<f64>) -> bool {
    match d {
        None => true,
        Some(_) if ignore => true,
        Some(d) => d <= fit.g(fit.ship, "maxTargetRange"),
    }
}

/// Pyfa graphs.calc checkDroneControlRange
pub fn in_drone_range(fit: &Fit, ignore: bool, d: Option<f64>) -> bool {
    match d {
        None => true,
        Some(_) if ignore => true,
        Some(d) => d <= fit.attr(fit.ship, extra_attr("droneControlRange")),
    }
}

/// active modules (state >= active), in fit order
pub fn active_modules(fit: &Fit) -> Vec<It> {
    fit.modules.iter().copied().filter(|&m| fit.items[m].state >= crate::eos::cx::ACTIVE).collect()
}
/// drones with amountActive > 0
pub fn active_drones(fit: &Fit) -> Vec<It> {
    fit.drones.iter().copied().filter(|&d| fit.items[d].amount_active > 0).collect()
}
/// (fighter, ability effect) for active fighters' active abilities
pub fn active_abilities(fit: &Fit) -> Vec<(It, u32)> {
    let mut v = Vec::new();
    for &f in &fit.fighters {
        if !fit.items[f].active {
            continue;
        }
        for &(e, on) in &fit.items[f].abilities {
            if on {
                v.push((f, e));
            }
        }
    }
    v
}
