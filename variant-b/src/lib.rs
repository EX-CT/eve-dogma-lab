//! eve-dogma: stateless EVE Online fitting engine (EXCT).
//!
//! `calc(dataset, request) -> stats` is a pure function: no I/O, no clocks, no global state.
pub mod capsim;
pub mod data;
pub mod eft;
pub mod engine;
pub mod request;
pub mod stats;

use serde_json::{json, Value};

pub use data::Dataset;
pub use request::FitRequest;

/// Compute full fit statistics for one request.
pub fn calc(ds: &Dataset, req: &FitRequest) -> Value {
    match engine::Fit::build(ds, req) {
        Ok(fit) => fit.compute_stats(req),
        Err(e) => json!({"error": {"code": e.code, "message": e.message, "path": e.path}}),
    }
}

/// Convenience: JSON string in, JSON string out.
pub fn calc_json(ds: &Dataset, request_json: &str) -> String {
    let v = match serde_json::from_str::<FitRequest>(request_json) {
        Ok(req) => calc(ds, &req),
        Err(e) => json!({"error": {"code": "BAD_REQUEST", "message": e.to_string(), "path": ""}}),
    };
    serde_json::to_string(&v).unwrap()
}

/// Compute many independent requests (JSON strings) on `threads` worker threads; results in input order.
pub fn calc_many(ds: &Dataset, requests: &[String], threads: usize) -> Vec<String> {
    let threads = threads.clamp(1, requests.len().max(1));
    if threads == 1 {
        return requests.iter().map(|r| calc_json(ds, r)).collect();
    }
    let mut out: Vec<String> = vec![String::new(); requests.len()];
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<&mut String>> = out.iter_mut().map(std::sync::Mutex::new).collect();
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= requests.len() {
                    break;
                }
                let r = calc_json(ds, &requests[i]);
                **slots[i].lock().unwrap() = r;
            });
        }
    });
    drop(slots);
    out
}
