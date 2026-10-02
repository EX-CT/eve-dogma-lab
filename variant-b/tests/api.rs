//! Library API behaviour: error responses, output shape, determinism, threading, skills and EFT round trips.
mod common;
use eve_dogma::{calc_json, calc_many, eft};
use serde_json::Value;

fn calc(ds: &eve_dogma::Dataset, req: &str) -> Value {
    serde_json::from_str(&calc_json(ds, req)).expect("calc_json returns JSON")
}

fn error_code(v: &Value) -> Option<(&str, &str)> {
    let e = v.get("error")?;
    Some((e["code"].as_str()?, e["path"].as_str()?))
}

const RIFTER: u32 = 587;

#[test]
fn malformed_json_is_bad_request() {
    let Some(ds) = common::dataset() else { return };
    assert_eq!(error_code(&calc(&ds, "nope")), Some(("BAD_REQUEST", "")));
    assert_eq!(error_code(&calc(&ds, "{}")).map(|e| e.0), Some("BAD_REQUEST"), "ship is required");
}

#[test]
fn unknown_types_are_reported_with_a_path() {
    let Some(ds) = common::dataset() else { return };
    let v = calc(&ds, r#"{"ship":{"type_id":1}}"#);
    assert_eq!(error_code(&v), Some(("UNKNOWN_TYPE", "/ship/type_id")));
    let v = calc(&ds, &format!(r#"{{"ship":{{"type_id":{RIFTER}}},"modules":[{{"type_id":2}}]}}"#));
    assert_eq!(error_code(&v), Some(("UNKNOWN_TYPE", "/modules/0")));
}

#[test]
fn bare_hull_has_every_section() {
    let Some(ds) = common::dataset() else { return };
    let v = calc(&ds, &format!(r#"{{"ship":{{"type_id":{RIFTER}}},"character":{{"skills":{{"default_level":5}}}}}}"#));
    for k in ["capacitor", "defense", "drones", "meta", "modules", "navigation", "offense", "resources", "ship", "targeting", "violations"] {
        assert!(v.get(k).is_some(), "missing section {k}");
    }
    assert_eq!(v["ship"]["type_id"], RIFTER);
    assert_eq!(v["ship"]["name"], "Rifter");
    assert_eq!(v["modules"].as_array().unwrap().len(), 0);
    assert_eq!(v["violations"].as_array().unwrap().len(), 0);
    assert_eq!(v["capacitor"]["stable"], true);
    assert!(v["meta"]["sde_build"].as_u64().is_some());
}

#[test]
fn skills_raise_hull_stats() {
    let Some(ds) = common::dataset() else { return };
    let at = |lvl: u8| calc(&ds, &format!(r#"{{"ship":{{"type_id":{RIFTER}}},"character":{{"skills":{{"default_level":{lvl}}}}}}}"#));
    let (s0, s5) = (at(0), at(5));
    let f = |v: &Value, p: &str| v.pointer(p).and_then(Value::as_f64).unwrap_or_else(|| panic!("{p}"));
    // Navigation V (+25 % velocity) and CPU Management V (+25 % CPU) on the bare hull
    assert!((f(&s5, "/navigation/max_velocity") / f(&s0, "/navigation/max_velocity") - 1.25).abs() < 1e-9);
    assert!((f(&s5, "/resources/cpu/total") / f(&s0, "/resources/cpu/total") - 1.25).abs() < 1e-9);
    assert!(f(&s5, "/defense/ehp/total") > f(&s0, "/defense/ehp/total"));
}

#[test]
fn every_case_computes_without_error() {
    let Some(ds) = common::dataset() else { return };
    for (name, req) in common::case_requests(&ds) {
        let out = calc_json(&ds, &req);
        assert!(!out.starts_with("{\"error\""), "{name}: {out}");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v.pointer("/defense/ehp/total").and_then(Value::as_f64).is_some_and(|x| x > 0.0), "{name}: ehp");
    }
}

#[test]
fn calc_many_matches_sequential_in_order() {
    let Some(ds) = common::dataset() else { return };
    let reqs: Vec<String> = common::case_requests(&ds).into_iter().map(|c| c.1).collect();
    let seq: Vec<String> = reqs.iter().map(|r| calc_json(&ds, r)).collect();
    assert_eq!(calc_many(&ds, &reqs, 1), seq);
    assert_eq!(calc_many(&ds, &reqs, 4), seq);
}

#[test]
fn results_do_not_depend_on_previous_calls() {
    let Some(ds) = common::dataset() else { return };
    let cases = common::case_requests(&ds);
    let first: Vec<String> = cases.iter().map(|c| calc_json(&ds, &c.1)).collect();
    // reverse order: any state leaking from one calc into the next would show up here
    for (i, c) in cases.iter().enumerate().rev() {
        assert_eq!(calc_json(&ds, &c.1), first[i], "{}", c.0);
    }
}

#[test]
fn eft_export_parses_back_to_the_same_fit() {
    let Some(ds) = common::dataset() else { return };
    let key = |r: &eve_dogma::FitRequest| {
        let mut m: Vec<String> = r.modules.iter().map(|m| format!("{}/{:?}/{:?}/{}", m.type_id, m.charge_type_id, m.state, m.mutation.is_some())).collect();
        m.sort();
        let mut d: Vec<String> = r.drones.iter().map(|d| format!("{}x{}", d.type_id, d.quantity)).collect();
        d.sort();
        (r.ship.type_id, m, d, r.implants.clone(), r.boosters.len(), r.cargo.len())
    };
    let files = common::fit_files();
    assert!(files.len() > 100);
    for f in files {
        let req = common::eft_request(&ds, &f);
        let back = eft::parse(&ds, &eft::export(&ds, &req, "x")).unwrap_or_else(|e| panic!("{f}: {e}"));
        assert_eq!(key(&back), key(&req), "{f}");
    }
}
