//! `ewar` graph (Pyfa "Electronic Warfare Stats"): EWAR strength vs distance.
use super::common::*;
use super::{GErr, GraphRequest};
use crate::api::build_calc;
use crate::data::Dataset;
use crate::eos::cx::{Fit, It, calc_range_factor};

const INF: f64 = f64::INFINITY;

/// one EWAR source: strengths (1 or 2 values), optimal, falloff, needs lock, needs drone control range
struct Src {
    s: [f64; 2],
    opt: f64,
    fall: f64,
    lock: bool,
    dcr: bool,
}

fn rng(fit: &Fit, m: It) -> (f64, f64) {
    (fit.max_range(m).unwrap_or(0.0), fit.falloff(m).unwrap_or(0.0))
}
fn burst_rng(fit: &Fit, m: It) -> (f64, f64) {
    ((fit.max_range(m).unwrap_or(0.0) + fit.g(m, "doomsdayAOERange")).max(0.0), fit.falloff(m).unwrap_or(0.0))
}

const ECM_GEN: [&str; 4] = ["scanGravimetricStrengthBonus", "scanLadarStrengthBonus", "scanMagnetometricStrengthBonus", "scanRadarStrengthBonus"];
const ECM_FTR: [&str; 4] = ["fighterAbilityECMStrengthGravimetric", "fighterAbilityECMStrengthLadar", "fighterAbilityECMStrengthMagnetometric", "fighterAbilityECMStrengthRadar"];

fn maxof(fit: &Fit, it: It, names: &[&str]) -> f64 {
    names.iter().map(|n| fit.g(it, n)).fold(f64::NEG_INFINITY, f64::max)
}

/// (module effects, burst effect, module attrs, drone effect, drone attr, fighter ability effect, fighter attr)
fn sources(fit: &Fit, y: &str, res: f64) -> Vec<Src> {
    let mut v = Vec::new();
    let push = |v: &mut Vec<Src>, s: [f64; 2], (opt, fall): (f64, f64), lock: bool, dcr: bool| v.push(Src { s, opt, fall, lock, dcr });
    let mods = active_modules(fit);
    let drones = active_drones(fit);
    let abil = active_abilities(fit);
    let dur = |m: It| fit.cycle_avg(m, None).unwrap_or(INF) / 1000.0;
    match y {
        "neut_gj_s" => {
            for &m in &mods {
                for e in ["energyNeutralizerFalloff", "structureEnergyNeutralizerFalloff"] {
                    if has_effect(fit, m, e) {
                        push(&mut v, [fit.g(m, "energyNeutralizerAmount") / dur(m) * res, 0.0], rng(fit, m), true, false);
                    }
                }
                if has_effect(fit, m, "energyNosferatuFalloff") && fit.g(m, "nosOverride") != 0.0 {
                    push(&mut v, [fit.g(m, "powerTransferAmount") / dur(m) * res, 0.0], rng(fit, m), true, false);
                }
                if has_effect(fit, m, "doomsdayAOENeut") {
                    push(&mut v, [fit.g(m, "energyNeutralizerAmount") / dur(m) * res, 0.0], burst_rng(fit, m), false, false);
                }
            }
            for &d in &drones {
                if has_effect(fit, d, "entityEnergyNeutralizerFalloff") {
                    for _ in 0..fit.items[d].amount_active {
                        push(&mut v, [fit.g(d, "energyNeutralizerAmount") / (fit.g(d, "energyNeutralizerDuration") / 1000.0) * res, 0.0], (INF, 0.0), true, true);
                    }
                }
            }
            for &(f, e) in &abil {
                if effect_name(fit, e) == "fighterAbilityEnergyNeutralizer" {
                    let nps = fit.g(f, "fighterAbilityEnergyNeutralizerAmount") / (fit.ability_cycle(f, e) / 1000.0);
                    push(&mut v, [nps * fit.items[f].amount as f64 * res, 0.0], (INF, 0.0), true, false);
                }
            }
        }
        "ecm_strength" => {
            for &m in &mods {
                for e in ["remoteECMFalloff", "structureModuleEffectECM"] {
                    if has_effect(fit, m, e) {
                        push(&mut v, [maxof(fit, m, &ECM_GEN) * res, 0.0], rng(fit, m), true, false);
                    }
                }
                if has_effect(fit, m, "doomsdayAOEECM") {
                    push(&mut v, [maxof(fit, m, &ECM_GEN) * res, 0.0], burst_rng(fit, m), false, false);
                }
            }
            for &d in &drones {
                if has_effect(fit, d, "entityECMFalloff") {
                    for _ in 0..fit.items[d].amount_active {
                        push(&mut v, [maxof(fit, d, &ECM_GEN) * res, 0.0], (INF, 0.0), true, true);
                    }
                }
            }
            for &(f, e) in &abil {
                if effect_name(fit, e) == "fighterAbilityECM" {
                    push(&mut v, [maxof(fit, f, &ECM_FTR) * fit.items[f].amount as f64 * res, 0.0], (INF, 0.0), true, false);
                }
            }
        }
        _ => {
            // multiplier-type EWAR: (module effects, burst effect, attribute(s), drone effect, fighter (ability, attr))
            let (meff, beff, attrs, deff, feff): (&[&str], &str, &[&str], &str, Option<(&str, &str)>) = match y {
                "web_pct" => (&["remoteWebifierFalloff", "structureModuleEffectStasisWebifier"], "doomsdayAOEWeb", &["speedFactor"], "remoteWebifierEntity",
                              Some(("fighterAbilityStasisWebifier", "fighterAbilityStasisWebifierSpeedPenalty"))),
                "damp_lock_range_pct" => (&["remoteSensorDampFalloff", "structureModuleEffectRemoteSensorDampener"], "doomsdayAOEDamp", &["maxTargetRangeBonus"], "remoteSensorDampEntity", None),
                "td_optimal_pct" => (&["shipModuleTrackingDisruptor", "structureModuleEffectWeaponDisruption"], "doomsdayAOETrack", &["maxRangeBonus"], "npcEntityWeaponDisruptor", None),
                "gd_range_pct" => (&["shipModuleGuidanceDisruptor", "structureModuleEffectWeaponDisruption"], "doomsdayAOETrack", &["missileVelocityBonus", "explosionDelayBonus"], "", None),
                "tp_sig_pct" => (&["remoteTargetPaintFalloff", "structureModuleEffectTargetPainter"], "doomsdayAOEPaint", &["signatureRadiusBonus"], "remoteTargetPaintEntity", None),
                _ => return v,
            };
            let st = |it: It| -> [f64; 2] { [fit.g(it, attrs[0]) * res, attrs.get(1).map(|a| fit.g(it, a) * res).unwrap_or(0.0)] };
            for &m in &mods {
                for e in meff {
                    if has_effect(fit, m, e) {
                        push(&mut v, st(m), rng(fit, m), true, false);
                    }
                }
                if has_effect(fit, m, beff) {
                    push(&mut v, st(m), burst_rng(fit, m), false, false);
                }
            }
            if !deff.is_empty() {
                for &d in &drones {
                    if has_effect(fit, d, deff) {
                        for _ in 0..fit.items[d].amount_active {
                            push(&mut v, st(d), (INF, 0.0), true, true);
                        }
                    }
                }
            }
            if let Some((ab, at)) = feff {
                for &(f, e) in &abil {
                    if effect_name(fit, e) == ab {
                        push(&mut v, [fit.g(f, at) * fit.items[f].amount as f64 * res, 0.0], (INF, 0.0), true, false);
                    }
                }
            }
        }
    }
    v
}

/// target-ship resistance attribute per EWAR series (the effects' resistanceID / modules' remoteResistanceID)
fn resist_attr(y: &str) -> &'static str {
    match y {
        "neut_gj_s" => "energyWarfareResistance",
        "web_pct" => "stasisWebifierResistance",
        "ecm_strength" => "ECMResistance",
        "damp_lock_range_pct" => "sensorDampenerResistance",
        "td_optimal_pct" | "gd_range_pct" => "weaponDisruptionResistance",
        _ => "targetPainterResistance",
    }
}

pub fn run(ds: &Dataset, req: &GraphRequest) -> Result<Vec<(String, Vec<Option<f64>>)>, GErr> {
    let fit = build_calc(ds, &req.fit)?;
    // 0.2: target fit -> resist from the target ship's resistance attribute (explicit params.resist wins)
    let tfit = match req.target.as_ref().and_then(|t| t.fit.as_ref()) {
        Some(f) => Some(build_calc(ds, f).map_err(|e| GErr { code: e.code, message: e.message, path: format!("/target/fit{}", e.path) })?),
        None => None,
    };
    let mut out = Vec::new();
    for y in &req.y {
        let res = match (req.p("resist"), &tfit) {
            (Some(r), _) => 1.0 - r.clamp(0.0, 1.0),
            (None, Some(t)) => {
                if y != "neut_gj_s" && t.g(t.ship, "disallowOffensiveModifiers") != 0.0 {
                    0.0
                } else {
                    let a = t.g(t.ship, resist_attr(y));
                    let a = if a == 0.0 { 1.0 } else { a }; // Pyfa `resist or 1`
                    1.0 - (1.0 - a).clamp(0.0, 1.0)
                }
            }
            (None, None) => 1.0,
        };
        let srcs = sources(&fit, y, res);
        let vals = req
            .x
            .values
            .iter()
            .map(|&x| {
                let d = Some(x);
                let lock = in_lock_range(&fit, req.settings.ignore_lock_range, d);
                let dcr = in_drone_range(&fit, req.settings.ignore_drone_control_range, d);
                let live = srcs.iter().filter(|s| !(s.lock && !lock) && !(s.dcr && !dcr));
                Some(match y.as_str() {
                    "neut_gj_s" | "ecm_strength" => live.map(|s| s.s[0] * calc_range_factor(s.opt, s.fall, d, true)).sum(),
                    _ => {
                        let (mut a, mut b) = (Vec::new(), Vec::new());
                        for s in live {
                            let rf = calc_range_factor(s.opt, s.fall, d, true);
                            a.push(1.0 + s.s[0] * rf / 100.0);
                            b.push(1.0 + s.s[1] * rf / 100.0);
                        }
                        let ma = calc_multiplier(&[a]);
                        match y.as_str() {
                            "gd_range_pct" => (1.0 - ma * calc_multiplier(&[b])) * 100.0,
                            "tp_sig_pct" => (ma - 1.0) * 100.0,
                            _ => (1.0 - ma) * 100.0,
                        }
                    }
                })
            })
            .collect();
        out.push((y.clone(), vals));
    }
    Ok(out)
}
