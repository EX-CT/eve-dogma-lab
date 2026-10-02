//! Request -> response glue (stateless). GPL-3.0-or-later.
use crate::data::Dataset;
use crate::eos::cx::{CommandBonus, Fit, NONE};
use crate::eos::fit::BuildError;
use crate::request::FitRequest;
use serde_json::{Value, json};

pub fn err(code: &str, message: &str, path: &str) -> Value {
    json!({"error": {"code": code, "message": message, "path": path}})
}

fn be(e: BuildError) -> Value {
    err(e.code, &e.message, &e.path)
}

/// Calculate one FitRequest given as JSON text.
pub fn calc_str(ds: &Dataset, text: &str) -> Value {
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return err("BAD_JSON", &e.to_string(), ""),
    };
    calc_value(ds, v)
}

pub fn calc_value(ds: &Dataset, v: Value) -> Value {
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
    Ok(fit.stats(req))
}
