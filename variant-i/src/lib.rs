//! eve-dogma-salsa (EX-CT dogma lab, variant I): EVE Online fitting engine built on the salsa
//! incremental-computation framework. Stateless contract (same request -> same bytes), but a `Session`
//! keeps memoised attribute queries across requests and only recomputes what a change invalidates.
pub mod bincache;
pub mod capsim;
pub mod data;
pub mod eft;
pub mod engine;
pub mod request;
pub mod session;
pub mod spec;
pub mod stats;

pub use data::Dataset;
pub use request::FitRequest;
pub use session::{Session, Workspace};

use serde_json::{json, Value};

/// JSON string in, JSON string out, on a session (incremental across calls).
pub fn calc_json(s: &mut Session, request_json: &str) -> String {
    let v: Value = match serde_json::from_str::<FitRequest>(request_json) {
        Ok(req) => s.calc(&req),
        Err(e) => json!({"error": {"code": "BAD_REQUEST", "message": e.to_string(), "path": ""}}),
    };
    serde_json::to_string(&v).unwrap()
}

/// Same as [`calc_json`] on a [`Workspace`] (one session per hull).
pub fn calc_json_ws(w: &mut Workspace, request_json: &str) -> String {
    let v: Value = match serde_json::from_str::<FitRequest>(request_json) {
        Ok(req) => w.calc(&req),
        Err(e) => json!({"error": {"code": "BAD_REQUEST", "message": e.to_string(), "path": ""}}),
    };
    serde_json::to_string(&v).unwrap()
}
