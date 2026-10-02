//! Graphs (EXCT CONTRACT-GRAPHS 0.1): Pyfa's graph subsystem re-implemented on top of the engine.
//! One GraphRequest in -> one GraphResult out; every sample point is evaluated exactly (Pyfa `getPoint`).
//! GPL-3.0-or-later.
pub mod common;
pub mod cycles;
pub mod simple;
pub mod ewar;
pub mod rr;
pub mod damage;
pub mod app;

use crate::data::Dataset;
use crate::eos::fit::BuildError;
use crate::request::FitRequest;
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Deserialize)]
pub struct Axis {
    pub axis: String,
    pub values: Vec<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    #[serde(default = "yes")]
    pub ignore_resists: bool,
    #[serde(default = "yes")]
    pub apply_projected: bool,
    #[serde(default = "yes")]
    pub ignore_lock_range: bool,
    #[serde(default)]
    pub ignore_drone_control_range: bool,
    #[serde(default = "auto")]
    pub mobile_drone_mode: String,
}
fn yes() -> bool {
    true
}
fn auto() -> String {
    "auto".into()
}
impl Default for Settings {
    fn default() -> Self {
        Settings { ignore_resists: true, apply_projected: true, ignore_lock_range: true, ignore_drone_control_range: false, mobile_drone_mode: auto() }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProfileReq {
    #[serde(default)]
    pub em: f64,
    #[serde(default)]
    pub thermal: f64,
    #[serde(default)]
    pub kinetic: f64,
    #[serde(default)]
    pub explosive: f64,
    #[serde(default)]
    pub max_velocity: Option<f64>,
    #[serde(default)]
    pub signature_radius: Option<f64>,
    #[serde(default)]
    pub radius: Option<f64>,
    #[serde(default)]
    pub hp: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TargetReq {
    #[serde(default)]
    pub profile: Option<ProfileReq>,
    #[serde(default)]
    pub fit: Option<Box<FitRequest>>,
    #[serde(default)]
    pub resist_mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraphRequest {
    pub graph: String,
    pub fit: FitRequest,
    #[serde(default)]
    pub target: Option<TargetReq>,
    pub x: Axis,
    pub y: Vec<String>,
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(default)]
    pub settings: Settings,
}

impl GraphRequest {
    pub fn p(&self, k: &str) -> Option<f64> {
        self.params.get(k).and_then(|v| v.as_f64())
    }
    pub fn pd(&self, k: &str, d: f64) -> f64 {
        self.p(k).unwrap_or(d)
    }
    pub fn pb(&self, k: &str, d: bool) -> bool {
        self.params.get(k).and_then(|v| v.as_bool()).unwrap_or(d)
    }
    pub fn ps(&self, k: &str) -> Option<&str> {
        self.params.get(k).and_then(|v| v.as_str())
    }
}

pub struct GErr {
    pub code: &'static str,
    pub message: String,
    pub path: String,
}
impl From<BuildError> for GErr {
    fn from(e: BuildError) -> Self {
        GErr { code: e.code, message: e.message, path: format!("/fit{}", e.path) }
    }
}
pub fn gerr(code: &'static str, message: impl Into<String>, path: &str) -> GErr {
    GErr { code, message: message.into(), path: path.into() }
}

/// Series: one Option<f64> per x; plus informational extra series (charge ids).
pub struct Out {
    pub series: Vec<(String, Vec<Value>)>,
}

fn fnum(v: Option<f64>) -> Value {
    match v {
        Some(x) if x.is_finite() => json!(x),
        _ => Value::Null,
    }
}

pub fn in_range(x: f64, lo: f64, hi: f64) -> bool {
    lo <= x && x <= hi
}

const GRAPHS: &[(&str, &[&str], &[&str])] = &[
    ("application_profile", &["distance_m"], &["dps", "volley"]),
    ("damage", &["distance_m", "time_s", "tgt_speed_mps", "tgt_sig_m"], &["dps", "volley", "damage"]),
    ("ewar", &["distance_m"], &["neut_gj_s", "web_pct", "ecm_strength", "damp_lock_range_pct", "td_optimal_pct", "gd_range_pct", "tp_sig_pct"]),
    ("remote_reps", &["distance_m", "time_s"], &["rps", "total"]),
    ("capacitor", &["time_s", "cap_pct"], &["cap_gj", "cap_regen_gj_s"]),
    ("shield_regen", &["time_s", "shield_pct"], &["shield_hp", "shield_regen_hp_s"]),
    ("mobility", &["time_s"], &["speed_mps", "distance_m", "momentum_kg_mps", "bump_speed_mps", "bump_distance_m"]),
    ("warp_time", &["distance_m"], &["time_s"]),
    ("lock_time", &["tgt_sig_m"], &["time_s"]),
];

pub fn run(ds: &Dataset, req: &GraphRequest) -> Result<Value, GErr> {
    let Some(g) = GRAPHS.iter().find(|g| g.0 == req.graph) else {
        return Err(gerr("UNKNOWN_GRAPH", req.graph.clone(), "/graph"));
    };
    if !g.1.contains(&req.x.axis.as_str()) {
        return Err(gerr("BAD_AXIS", format!("x axis {} not valid for {}", req.x.axis, req.graph), "/x/axis"));
    }
    for (i, y) in req.y.iter().enumerate() {
        if !g.2.contains(&y.as_str()) {
            return Err(gerr("BAD_AXIS", format!("y {y} not valid for {}", req.graph), &format!("/y/{i}")));
        }
    }
    let mut series: Map<String, Value> = Map::new();
    let mut put = |name: String, v: Vec<Option<f64>>| {
        series.insert(name, Value::Array(v.into_iter().map(fnum).collect()));
    };
    match req.graph.as_str() {
        "mobility" | "warp_time" | "lock_time" | "capacitor" | "shield_regen" => {
            for (k, v) in simple::run(ds, req)? {
                put(k, v);
            }
        }
        "ewar" => {
            for (k, v) in ewar::run(ds, req)? {
                put(k, v);
            }
        }
        "remote_reps" => {
            for (k, v) in rr::run(ds, req)? {
                put(k, v);
            }
        }
        "damage" => {
            for (k, v) in damage::run(ds, req)? {
                put(k, v);
            }
        }
        "application_profile" => {
            let (vals, ids) = app::run(ds, req)?;
            for (k, v) in vals {
                put(k, v);
            }
            for (k, v) in ids {
                series.insert(k, Value::Array(v.into_iter().map(|c| c.map(|c| json!(c)).unwrap_or(Value::Null)).collect()));
            }
        }
        _ => unreachable!(),
    }
    Ok(json!({"graph": req.graph, "x_axis": req.x.axis, "x": req.x.values, "series": series}))
}

fn err_value(e: GErr) -> Value {
    json!({"error": {"code": e.code, "message": e.message, "path": e.path}})
}

pub fn graph_value(ds: &Dataset, v: Value) -> Value {
    match serde_json::from_value::<GraphRequest>(v) {
        Ok(r) => run(ds, &r).unwrap_or_else(err_value),
        Err(e) => err_value(gerr("BAD_REQUEST", e.to_string(), "")),
    }
}

pub fn graph_str(ds: &Dataset, text: &str) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(v) => graph_value(ds, v),
        Err(e) => err_value(gerr("BAD_JSON", e.to_string(), "")),
    }
}
