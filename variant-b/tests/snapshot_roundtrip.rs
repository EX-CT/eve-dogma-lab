//! The dataset snapshot (cache file format) must be output-neutral: a dataset loaded from snapshot bytes computes
//! byte-identical FitStats to the dataset parsed from the .json.gz, for every case in tests/cases.
use eve_dogma::Dataset;

#[test]
fn snapshot_load_is_output_identical() {
    let p = std::env::var("EVE_DOGMA_DATASET").unwrap_or_else(|_| "dataset.json.gz".into());
    if !std::path::Path::new(&p).exists() {
        eprintln!("SKIP: dataset not found at {p}");
        return;
    }
    let bytes = std::fs::read(&p).unwrap();
    let parsed = Dataset::load_bytes(&bytes).expect("parse dataset");
    let snap = parsed.snapshot_roundtrip().expect("snapshot round trip");
    let mut files: Vec<_> = std::fs::read_dir("tests/cases").unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    let mut n = 0;
    for f in files {
        if f.extension().map_or(true, |x| x != "json") {
            continue;
        }
        let req = std::fs::read_to_string(&f).unwrap();
        let a = eve_dogma::calc_json(&parsed, &req);
        let b = eve_dogma::calc_json(&snap, &req);
        assert_eq!(a, b, "{}", f.display());
        n += 1;
    }
    assert!(n > 100, "only {n} cases");
}
