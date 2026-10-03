//! `ecm_burst` graph (CONTRACT-GRAPHS 0.2; Pyfa's hidden "ECM Burst + Scanres Damps", `fitEcmBurstScanresDamps`):
//! enemy lock time / lock uptime vs enemy scan resolution under the source's scan-res damps, and the damage the
//! source deals before dying while it ECM-bursts every 30 s. GPL-3.0-or-later.
use super::common::*;
use super::{GErr, GraphRequest};
use crate::api::build_calc;
use crate::data::Dataset;
use crate::eos::cx::Fit;

const BURST: f64 = 30.0;
const DRONE_LOCK: f64 = 2.0;

/// stacking-penalised scan resolution multiplier of the source's damps (one 'default' group, range ignored)
fn damp_mult(fit: &Fit) -> f64 {
    let mut g = Vec::new();
    for m in active_modules(fit) {
        for e in ["remoteSensorDampFalloff", "structureModuleEffectRemoteSensorDampener", "doomsdayAOEDamp"] {
            if has_effect(fit, m, e) {
                g.push(1.0 + fit.g(m, "scanResolutionBonus") / 100.0);
            }
        }
    }
    for d in active_drones(fit) {
        if has_effect(fit, d, "remoteSensorDampEntity") {
            for _ in 0..fit.items[d].amount_active {
                g.push(1.0 + fit.g(d, "scanResolutionBonus") / 100.0);
            }
        }
    }
    calc_multiplier(&[g])
}

/// eos.calc calculateLockTime
fn lock_time(scan_res: f64, sig: f64) -> Option<f64> {
    if scan_res == 0.0 || sig == 0.0 {
        return None;
    }
    Some((40000.0 / scan_res / sig.asinh().powi(2)).min(1800.0))
}

fn num(v: Option<&crate::jv::Value>) -> f64 {
    match v {
        Some(crate::jv::Value::F(x)) => *x,
        Some(crate::jv::Value::U(x)) => *x as f64,
        Some(crate::jv::Value::I(x)) => *x as f64,
        _ => 0.0,
    }
}

#[allow(clippy::too_many_arguments)]
fn inflicted(sig: f64, wdps: f64, ddps: f64, ehp: f64, scan_res: f64, tgt_dps: f64, adj: f64, limit: f64) -> Option<f64> {
    let lt = lock_time(scan_res, sig)?;
    let up = (BURST - lt - adj).max(0.0);
    let down = BURST - up;
    let mut dmg = 0.0;
    let mut rem = ehp;
    let n = if limit.is_finite() && limit > 0.0 { limit.trunc() as i64 } else { 0 };
    for _ in 0..n {
        let alive = down + up.min(rem / tgt_dps);
        rem -= up * tgt_dps;
        dmg += alive * wdps;
        dmg += (alive - DRONE_LOCK - 1.0).max(0.0) * ddps;
        if rem <= 0.0 {
            break;
        }
    }
    Some(dmg)
}

pub fn run(ds: &Dataset, req: &GraphRequest) -> Result<Vec<(String, Vec<Option<f64>>)>, GErr> {
    let fit = build_calc(ds, &req.fit)?;
    let mult = if req.pb("apply_damps", true) { damp_mult(&fit) } else { 1.0 };
    let sig = fit.g(fit.ship, "signatureRadius");
    let p_scan = req.pd("tgt_scan_res_mm", 700.0);
    let p_dps = req.pd("tgt_dps", 200.0);
    let adj = req.pd("uptime_adj_s", 1.0);
    let limit = req.pd("uptime_amount_limit", 3.0);
    let axis = req.x.axis.as_str();
    let mut dmg_inputs = None;
    let mut out = Vec::new();
    for y in &req.y {
        let mut vals = Vec::new();
        for &x in &req.x.values {
            let scan = if axis == "tgt_scan_res_mm" { x } else { p_scan };
            if !(scan >= 1.0) {
                vals.push(None); // Pyfa limiter (1, inf) on the scan-res axis; scan res < 1 as a param likewise
                continue;
            }
            let v = match y.as_str() {
                "tgt_lock_time_s" => lock_time(scan * mult, sig),
                "tgt_lock_uptime_s" => lock_time(scan * mult, sig).map(|t| (BURST - t).max(0.0)),
                _ => {
                    let tdps = if axis == "tgt_dps" { x } else { p_dps };
                    if !(tdps > 0.0) {
                        None
                    } else {
                        let (w, d, e) = *dmg_inputs.get_or_insert_with(|| {
                            let st = fit.stats(&req.fit);
                            let tot = st.get("offense").and_then(|o| o.get("total"));
                            let w = num(tot.and_then(|t| t.get("weapon_dps")));
                            let d = if req.pb("apply_drones", true) {
                                num(tot.and_then(|t| t.get("drone_dps"))) + num(tot.and_then(|t| t.get("fighter_dps")))
                            } else {
                                0.0
                            };
                            let e = num(st.get("defense").and_then(|v| v.get("ehp")).and_then(|v| v.get("total")));
                            (w, d, e)
                        });
                        inflicted(sig, w, d, e, scan * mult, tdps, adj, limit)
                    }
                }
            };
            vals.push(v);
        }
        out.push((y.clone(), vals));
    }
    Ok(out)
}
