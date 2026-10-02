//! The dataset snapshot (cache file format) must be output-neutral: a dataset loaded from snapshot bytes computes
//! byte-identical FitStats to the dataset parsed from the .json.gz, for every case in tests/cases.
mod common;
use eve_dogma::Dataset;

#[test]
fn snapshot_load_is_output_identical() {
    let Some(_) = common::dataset() else { return };
    let p = std::env::var("EVE_DOGMA_DATASET").unwrap_or_else(|_| "dataset.json.gz".into());
    let parsed = Dataset::load_bytes(&std::fs::read(&p).unwrap()).expect("parse dataset");
    let snap = parsed.snapshot_roundtrip().expect("snapshot round trip");
    let cases = common::case_requests(&parsed);
    assert!(cases.len() > 100, "only {} cases", cases.len());
    for (name, req) in &cases {
        let a = eve_dogma::calc_json(&parsed, req);
        assert!(!a.starts_with("{\"error\""), "{name}: {a}");
        assert_eq!(a, eve_dogma::calc_json(&snap, req), "{name}");
    }
}
