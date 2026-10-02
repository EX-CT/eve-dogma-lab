//! Pyfa parity regression tests: every bench 1.8.0 case (0969967, 326 cases, 21 051 Pyfa-expected values with the
//! bench's known divergences removed) plus the staged pending-1.9.0 cases, checked at the bench tolerance.
//! Fixtures: `tests/fixtures/*.jsonl.gz`, one `{case, request, expect: [[metric, pointer, value]]}` per line.
mod common;
use common::{close, dataset, extract};
use serde_json::Value;
use std::io::Read;

fn fixture(name: &str) -> Vec<Value> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let mut s = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&path).expect("fixture")).read_to_string(&mut s).unwrap();
    s.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

/// Run every case of `file` whose name starts with `family_` (or every case for "*") and report all mismatches.
fn check(file: &str, family: &str) {
    let Some(ds) = dataset() else { return };
    let cases: Vec<Value> = fixture(file)
        .into_iter()
        .filter(|c| family == "*" || c["case"].as_str().unwrap().split('_').next() == Some(family))
        .collect();
    assert!(!cases.is_empty(), "no cases for family {family}");
    let (mut bad, mut n) = (Vec::new(), 0);
    for c in &cases {
        let out: Value = serde_json::from_str(&eve_dogma_h::calc_json(ds, &c["request"].to_string())).unwrap();
        assert!(out.get("error").is_none(), "{}: engine error {}", c["case"], out["error"]);
        for e in c["expect"].as_array().unwrap() {
            n += 1;
            let got = extract(&out, e[1].as_str().unwrap());
            if !close(&got, &e[2]) {
                bad.push(format!("{} {}: got {} want {}", c["case"].as_str().unwrap(), e[0].as_str().unwrap(), got, e[2]));
            }
        }
    }
    assert!(bad.is_empty(), "{}/{n} values differ from Pyfa:\n{}", bad.len(), bad.join("\n"));
}

macro_rules! bench_families {
    ($($name:ident => $family:literal),* $(,)?) => {
        $(#[test] fn $name() { check("bench-1.8.0.jsonl.gz", $family); })*
    };
}

bench_families! {
    aoe_burst_projectors => "aoe", boosters => "booster", aoe_clouds => "cloud", damage_patterns => "dmgpattern",
    drones => "drones", ecm => "ecm", system_effects => "env", esf_scenarios => "esf", exct_fits => "exct",
    fighters => "fighters", fleet_boosts => "fleet", implants => "implants", incursion => "incursion",
    projected_fighters => "pfighter", projected_modules => "proj", projected_fits => "projfit", reload => "reload",
    skills_level0 => "skills0", skills_level2 => "skills2", skills_level3 => "skills3", skills_level4 => "skills4",
    standup_modules => "standup", sustained_tank => "sustain", weather => "weather",
}

#[test]
fn bench_corpus_is_complete() {
    let cases = fixture("bench-1.8.0.jsonl.gz");
    assert_eq!(cases.len(), 326);
    assert_eq!(cases.iter().map(|c| c["expect"].as_array().unwrap().len()).sum::<usize>(), 21051);
}

/// Breacher pods (strongest pod only, `pure` damage), overheat in fit order (Tengu subsystem listed after the
/// hardener), neut/nos vs MWD signature, EWAR drone cycle time.
#[test]
fn pending_1_9_0_cases() {
    check("pending-1.9.0.jsonl.gz", "*");
}
