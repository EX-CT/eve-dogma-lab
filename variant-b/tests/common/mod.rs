//! Shared helpers for the integration tests (dataset loading, the tests/cases request corpus).
#![allow(dead_code)]
use eve_dogma::{eft, Dataset, FitRequest};
use serde_json::Value;

/// The dataset from $EVE_DOGMA_DATASET or ./dataset.json.gz; `None` (test skipped) when absent.
pub fn dataset() -> Option<Dataset> {
    let p = std::env::var("EVE_DOGMA_DATASET").unwrap_or_else(|_| "dataset.json.gz".into());
    if !std::path::Path::new(&p).exists() {
        eprintln!("SKIP: dataset not found at {p}");
        return None;
    }
    Some(Dataset::load_path(&p).expect("load dataset"))
}

/// EFT text of tests/fits/<file> parsed into a request with all skills at V.
pub fn eft_request(ds: &Dataset, file: &str) -> FitRequest {
    let text = std::fs::read_to_string(format!("tests/{file}")).unwrap_or_else(|e| panic!("{file}: {e}"));
    let mut req = eft::parse(ds, &text).unwrap_or_else(|e| panic!("{file}: {e}"));
    req.character.skills.default_level = Some(5);
    req
}

/// Every tests/cases/*.json case (`{"eft": "fits/x.eft", "patch": {top-level request keys}}`) as a
/// (name, FitRequest JSON) pair, sorted by name.
pub fn case_requests(ds: &Dataset) -> Vec<(String, String)> {
    let mut files: Vec<_> = std::fs::read_dir("tests/cases").unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    files
        .into_iter()
        .filter(|f| f.extension().is_some_and(|x| x == "json"))
        .map(|f| {
            let case: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
            let mut req = serde_json::to_value(eft_request(ds, case["eft"].as_str().unwrap())).unwrap();
            if let Some(patch) = case.get("patch").and_then(Value::as_object) {
                for (k, x) in patch {
                    req[k] = x.clone();
                }
            }
            (f.file_stem().unwrap().to_string_lossy().into_owned(), req.to_string())
        })
        .collect()
}

/// Sorted list of tests/fits/*.eft file names (relative to tests/).
pub fn fit_files() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tests/fits")
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".eft"))
        .map(|n| format!("fits/{n}"))
        .collect();
    v.sort();
    v
}
