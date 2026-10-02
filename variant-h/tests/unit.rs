//! Contract and helper behaviour: Python rounding, search ranking, EFT round trip, error reporting, determinism.
mod common;
use common::dataset;
use eve_dogma_h::{calc::py_round2, tools, FitRequest};
use serde_json::{json, Value};

#[test]
fn py_round2_matches_python_round() {
    // Python round(x, 2) rounds the exact binary value half to even
    for (x, want) in [(2.675, 2.67), (0.125, 0.12), (0.375, 0.38), (1.005, 1.0), (797.0, 797.0), (23588.645, 23588.65), (-0.125, -0.12)] {
        assert_eq!(py_round2(x), want, "round({x}, 2)");
    }
}

fn names(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn search_exact_then_prefix_then_substring() {
    let Some(ds) = dataset() else { return };
    let r = tools::search(ds, "hammerhead ii", None, None);
    assert_eq!(names(&r)[0], "Hammerhead II");
    let r = tools::search(ds, "hammer", None, None);
    let n = names(&r);
    let first_sub = n.iter().position(|s| !s.to_lowercase().starts_with("hammer")).unwrap_or(n.len());
    assert!(n[first_sub..].iter().all(|s| !s.to_lowercase().starts_with("hammer")), "prefix hits come first: {n:?}");
    let ids: Vec<u64> = r.as_array().unwrap()[..first_sub].iter().map(|x| x["type_id"].as_u64().unwrap()).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "ties ordered by typeID: {ids:?}");
}

#[test]
fn search_limit_defaults_to_20() {
    let Some(ds) = dataset() else { return };
    assert_eq!(tools::search(ds, "a", None, None).as_array().unwrap().len(), 20);
    assert_eq!(tools::search(ds, "a", Some(5), None).as_array().unwrap().len(), 5);
    assert!(tools::search(ds, "   ", None, None).as_array().unwrap().is_empty());
}

#[test]
fn type_lookup_by_id_and_name() {
    let Some(ds) = dataset() else { return };
    assert_eq!(tools::type_info(ds, "587").unwrap()["name"], "Rifter");
    assert_eq!(tools::type_info(ds, "Rifter").unwrap()["type_id"], 587);
    assert!(tools::type_info(ds, "999999999").is_none());
}

const EFT: &str = "[Rifter, test]\nDamage Control II\nGyrostabilizer II\n\n5MN Microwarpdrive II\nWarp Scrambler II\n\n200mm AutoCannon II, Republic Fleet EMP S\n200mm AutoCannon II, Republic Fleet EMP S\n\nSmall Projectile Burst Aerator I\n\nWarrior II x2\n";

#[test]
fn eft_parse_export_round_trip() {
    let Some(ds) = dataset() else { return };
    let fit = tools::eft_parse(ds, EFT, Some(5)).ok().expect("parses");
    let req: FitRequest = serde_json::from_value(fit["fit"].clone()).or_else(|_| serde_json::from_value(fit.clone())).unwrap();
    let text = tools::eft_export(ds, &req, Some("test"));
    let again: FitRequest = {
        let f = tools::eft_parse(ds, &text, Some(5)).ok().unwrap();
        serde_json::from_value(f["fit"].clone()).or_else(|_| serde_json::from_value(f)).unwrap()
    };
    assert_eq!(tools::eft_export(ds, &again, Some("test")), text);
    assert!(text.starts_with("[Rifter, test]"));
    assert!(text.contains("Warrior II x2"));
}

#[test]
fn bad_json_and_bad_request_are_errors() {
    let Some(ds) = dataset() else { return };
    let v: Value = serde_json::from_str(&eve_dogma_h::calc_json(ds, "{not json")).unwrap();
    assert_eq!(v["error"]["code"], "BAD_JSON");
    let v: Value = serde_json::from_str(&eve_dogma_h::calc_json(ds, r#"{"ship": 5}"#)).unwrap();
    assert!(v.get("error").is_some());
}

#[test]
fn calc_is_deterministic() {
    let Some(ds) = dataset() else { return };
    let req = json!({"ship": {"type_id": 587}, "modules": [{"type_id": 2048, "slot": "low", "state": "active"}]}).to_string();
    let a = eve_dogma_h::calc_json(ds, &req);
    assert_eq!(a, eve_dogma_h::calc_json(ds, &req));
    let v: Value = serde_json::from_str(&a).unwrap();
    assert!(v.get("error").is_none(), "{v}");
}
