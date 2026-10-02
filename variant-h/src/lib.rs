//! eve-dogma-h — Variant H of the EXCT dogma engine: an Entity-Component-System design on `hecs`.
//!
//! `calc(dataset, request) -> stats` is a pure function: one fresh `hecs::World` per request, a fixed
//! schedule of systems (see `fit.rs`), then read-only output systems (`stats.rs`, `validate.rs`).
pub mod calc;
pub mod capsim;
pub mod components;
pub mod data;
pub mod fit;
pub mod ids;
pub mod request;
pub mod sha256;
pub mod stats;
pub mod validate;

use serde_json::{json, Value};

pub use data::Dataset;
pub use request::FitRequest;

pub fn calc(ds: &Dataset, req: &FitRequest) -> Value {
    match fit::Fit::run(ds, req) {
        Ok(fit) => stats::compute(&fit, req),
        Err(e) => json!({"error": {"code": e.code, "message": e.message, "path": e.path}}),
    }
}

pub fn calc_json(ds: &Dataset, request_json: &str) -> String {
    let v = match serde_json::from_str::<Value>(request_json) {
        Err(e) => json!({"error": {"code": "BAD_JSON", "message": e.to_string(), "path": ""}}),
        Ok(raw) => match serde_json::from_value::<FitRequest>(raw) {
            Ok(req) => calc(ds, &req),
            Err(e) => json!({"error": {"code": "BAD_REQUEST", "message": e.to_string(), "path": ""}}),
        },
    };
    serde_json::to_string(&v).unwrap()
}
