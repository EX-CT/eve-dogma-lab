//! Request -> response glue (stateless). GPL-3.0-or-later.
use crate::data::Dataset;
use crate::eos::cx::{CommandBonus, Fit, NONE, RT_EARLY, RT_LATE, RT_NORMAL};
use crate::eos::fit::BuildError;
use crate::request::FitRequest;
use crate::jv::Value;
use serde_json::json;

pub fn err(code: &str, message: &str, path: &str) -> Value {
    Value::from(json!({"error": {"code": code, "message": message, "path": path}}))
}

fn be(e: BuildError) -> Value {
    err(e.code, &e.message, &e.path)
}

/// Calculate one FitRequest given as JSON text.
pub fn calc_str(ds: &Dataset, text: &str) -> Value {
    match serde_json::from_str::<FitRequest>(text) {
        Ok(req) => match calc(ds, &req) {
            Ok(v) => v,
            Err(e) => be(e),
        },
        Err(e) => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(_) => err("BAD_REQUEST", &e.to_string(), ""),
            Err(e) => err("BAD_JSON", &e.to_string(), ""),
        },
    }
}

pub fn calc_value(ds: &Dataset, v: serde_json::Value) -> Value {
    let req: FitRequest = match serde_json::from_value(v) {
        Ok(r) => r,
        Err(e) => return err("BAD_REQUEST", &e.to_string(), ""),
    };
    match calc(ds, &req) {
        Ok(v) => v,
        Err(e) => be(e),
    }
}

pub fn calc(ds: &Dataset, req: &FitRequest) -> Result<Value, BuildError> {
    let t0 = std::time::Instant::now();
    let fit = build_calc(ds, req)?;
    let t2 = std::time::Instant::now();
    let out = fit.stats(req);
    if std::env::var_os("EVE_E_PROF").is_some() {
        let t3 = std::time::Instant::now();
        eprintln!("prof build+calc={:?} stats={:?}", t2 - t0, t3 - t2);
    }
    Ok(out)
}

/// Build and fully calculate a fit (command fits, explicit buffs, projected fits): the state `calc` reports on.
pub fn build_calc<'a>(ds: &'a Dataset, req: &FitRequest) -> Result<Fit<'a>, BuildError> {
    // command fits first (Pyfa: commandFits are calculated before the local fit)
    let mut bonuses: Vec<CommandBonus> = Vec::new();
    for (i, b) in req.fleet.booster_fits.iter().enumerate() {
        let mut bf = Fit::build(ds, b).map_err(|mut e| {
            e.path = format!("/fleet/booster_fits/{i}{}", e.path);
            e
        })?;
        for mut cb in bf.calculate_command() {
            cb.thing = NONE;
            match bonuses.iter_mut().find(|x| x.id == cb.id) {
                Some(x) => {
                    if x.value.abs() < cb.value.abs() {
                        *x = cb;
                    }
                }
                None => bonuses.push(cb),
            }
        }
    }
    let mut fit = Fit::build(ds, req)?;
    fit.command_bonuses = bonuses;
    let explicit: Vec<(u32, f64)> = req.fleet.buffs.iter().map(|b| (b.buff_id, b.value)).collect();
    fit.calculate(&explicit);
    // projected fits (Pyfa: after the local calculation, each projected fit runs its own calculation and
    // projects its drones/fighters/modules after every runtime)
    for (i, p) in req.projected.iter().enumerate() {
        if p.kind != "fit" {
            continue;
        }
        let Some(sreq) = &p.fit else { continue };
        let mut sreq = (**sreq).clone();
        sreq.projected.clear();
        let mut sf = Fit::build(ds, &sreq).map_err(|mut e| {
            e.path = format!("/projected/{i}/fit{}", e.path);
            e
        })?;
        sf.command_bonuses.clear();
        let mut mirror = Vec::new();
        for rt in [RT_EARLY, RT_NORMAL, RT_LATE] {
            sf.calc_rt(rt, &[], None);
            fit.project_from(&sf, rt, p.amount.max(1), p.distance_m, &mut mirror);
        }
    }
    Ok(fit)
}

/// `eft_export` RPC: params {fit: FitRequest, name?} -> {text}
pub fn eft_export_value(ds: &Dataset, params: serde_json::Value) -> Value {
    let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
    let Some(fit) = params.get("fit").cloned() else {
        return err("BAD_REQUEST", "missing params.fit", "/params/fit");
    };
    let req: FitRequest = match serde_json::from_value(fit) {
        Ok(r) => r,
        Err(e) => return err("BAD_REQUEST", &e.to_string(), "/params/fit"),
    };
    match crate::eft::export(ds, &req, &name) {
        Ok(t) => Value::from(json!({ "text": t })),
        Err(e) => be(e),
    }
}
