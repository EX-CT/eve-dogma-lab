//! Shared helpers for the integration tests: one dataset per test binary, bench-style metric extraction.
#![allow(dead_code)]
use eve_dogma_h::Dataset;
use serde_json::Value;
use std::sync::OnceLock;

pub const DEFAULT_DATASET: &str = "/workspace/exct-eve/data/dataset-3569502.json.gz";

/// The dataset (`EVE_DOGMA_DATASET`, else the box default). `None` when it is not available on this machine:
/// the dataset is not part of the repository, so data-driven tests are skipped with a notice in that case.
pub fn dataset() -> Option<&'static Dataset> {
    static DS: OnceLock<Option<Dataset>> = OnceLock::new();
    DS.get_or_init(|| {
        let p = std::env::var("EVE_DOGMA_DATASET").unwrap_or_else(|_| DEFAULT_DATASET.to_string());
        if !std::path::Path::new(&p).exists() {
            eprintln!("dataset {p} not found: skipping data-driven test (set EVE_DOGMA_DATASET)");
            return None;
        }
        Some(Dataset::load_path_cached(&p).expect("dataset loads"))
    })
    .as_ref()
}

/// JSON pointer with the bench's `name[key=value]` array selectors.
pub fn pointer<'a>(doc: &'a Value, ptr: &str) -> Option<&'a Value> {
    let mut cur = doc;
    for part in ptr.trim_matches('/').split('/') {
        if let (true, Some(open)) = (part.ends_with(']'), part.find('[')) {
            let (name, sel) = (&part[..open], &part[open + 1..part.len() - 1]);
            let (key, want) = sel.split_once('=')?;
            cur = cur.get(name)?.as_array()?.iter().find(|e| match e.get(key) {
                Some(Value::String(s)) => s == want,
                Some(v) => v.to_string() == want,
                None => false,
            })?;
        } else if let Some(v) = cur.get(part) {
            cur = v;
        } else {
            let i: usize = part.parse().ok()?;
            cur = cur.get(i)?;
        }
    }
    Some(cur)
}

/// Metric expression (`a+b` sums pointers), like the bench's `metrics.extract`.
pub fn extract(doc: &Value, expr: &str) -> Value {
    if expr.contains('+') {
        let vals: Vec<Option<&Value>> = expr.split('+').map(|p| pointer(doc, p)).collect();
        if vals.iter().all(|v| v.is_none()) {
            return Value::Null;
        }
        return serde_json::json!(vals.iter().map(|v| v.and_then(|x| x.as_f64()).unwrap_or(0.0)).sum::<f64>());
    }
    pointer(doc, expr).cloned().unwrap_or(Value::Null)
}

/// The bench tolerance: |got - want| <= max(1e-3, 1e-4 * |want|); booleans compare by truthiness.
pub fn close(got: &Value, want: &Value) -> bool {
    match (got, want) {
        (Value::Bool(_), _) | (_, Value::Bool(_)) => !got.is_null() && truthy(got) == truthy(want),
        (Value::Null, _) | (_, Value::Null) => got == want,
        _ => match (got.as_f64(), want.as_f64()) {
            (Some(g), Some(w)) => (g - w).abs() <= f64::max(1e-3, 1e-4 * w.abs()),
            _ => got == want,
        },
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|x| x != 0.0).unwrap_or(false),
        Value::Null => false,
        _ => true,
    }
}
