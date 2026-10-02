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

/// Compact JSON with every float rounded to 6 decimals as it is written - the same bytes as
/// `stats::tidy` + `serde_json::to_string`, without the extra pass over the tree.
struct Round6;
impl serde_json::ser::Formatter for Round6 {
    #[inline]
    fn write_f64<W: ?Sized + std::io::Write>(&mut self, w: &mut W, v: f64) -> std::io::Result<()> {
        let r = stats::round6(v);
        if r.is_finite() {
            serde_json::ser::Formatter::write_f64(&mut serde_json::ser::CompactFormatter, w, r)
        } else {
            w.write_all(b"null")
        }
    }
}

/// Serialise an untidied calc result (floats rounded at write time).
fn to_string_round6(v: &Value) -> String {
    use serde::Serialize;
    let mut out = Vec::with_capacity(8192);
    let mut ser = serde_json::Serializer::with_formatter(&mut out, Round6);
    v.serialize(&mut ser).unwrap();
    // serde_json only writes valid UTF-8
    unsafe { String::from_utf8_unchecked(out) }
}

fn calc_untidied(f: impl FnOnce() -> Value) -> String {
    stats::SKIP_TIDY.with(|c| c.set(true));
    let v = f();
    stats::SKIP_TIDY.with(|c| c.set(false));
    to_string_round6(&v)
}

/// JSON string in, JSON string out, on a session (incremental across calls).
pub fn calc_json(s: &mut Session, request_json: &str) -> String {
    match serde_json::from_str::<FitRequest>(request_json) {
        Ok(req) => calc_untidied(|| s.calc(&req)),
        Err(e) => serde_json::to_string(&json!({"error": {"code": "BAD_REQUEST", "message": e.to_string(), "path": ""}})).unwrap(),
    }
}

/// Same as [`calc_json`] on a [`Workspace`] (one session per hull).
pub fn calc_json_ws(w: &mut Workspace, request_json: &str) -> String {
    match serde_json::from_str::<FitRequest>(request_json) {
        Ok(req) => calc_untidied(|| w.calc(&req)),
        Err(e) => serde_json::to_string(&json!({"error": {"code": "BAD_REQUEST", "message": e.to_string(), "path": ""}})).unwrap(),
    }
}
