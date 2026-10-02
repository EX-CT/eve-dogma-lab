//! eve-dogma-e CLI: stateless FitRequest JSON in, FitStats JSON out. GPL-3.0-or-later.
use eve_dogma_e::api;
use eve_dogma_e::data::Dataset;
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};

const USAGE: &str = "eve-dogma-e <command> [--dataset PATH] [args]

Commands:
  calc [FILE]     FitRequest JSON (file or stdin) -> FitStats JSON (default when no command is given)
  batch           JSONL FitRequests on stdin -> JSONL FitStats on stdout
  serve-stdio     JSONL RPC {\"id\",\"method\":\"calc|meta\",\"params\"} -> {\"id\",\"result\"}
  meta            dataset info

Dataset: --dataset PATH, or $EVE_DOGMA_DATASET, or ./dataset.json.gz";

fn load(path: Option<String>) -> Result<Dataset, String> {
    let p = path.or_else(|| std::env::var("EVE_DOGMA_DATASET").ok()).unwrap_or_else(|| "dataset.json.gz".into());
    Dataset::load(&p)
}

fn meta(ds: &Dataset) -> Value {
    json!({"engine": concat!("eve-dogma-e ", env!("CARGO_PKG_VERSION"), " (pyfa port)"), "schema_version": 1,
           "sde_build": ds.build, "dataset_sha256": ds.sha256, "types": ds.types.len()})
}

fn out<T: serde::Serialize>(v: &T) {
    let mut buf = serde_json::to_vec(v).unwrap_or_default();
    buf.push(b'\n');
    let _ = std::io::stdout().lock().write_all(&buf);
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut dataset = None;
    if let Some(i) = args.iter().position(|a| a == "--dataset") {
        if i + 1 < args.len() {
            dataset = Some(args[i + 1].clone());
            args.drain(i..=i + 1);
        } else {
            args.remove(i);
        }
    }
    if let Some(i) = args.iter().position(|a| a.starts_with("--dataset=")) {
        dataset = Some(args[i]["--dataset=".len()..].to_string());
        args.remove(i);
    }
    let cmd = args.first().cloned().unwrap_or_else(|| "calc".into());
    if cmd == "-h" || cmd == "--help" || cmd == "help" {
        println!("{USAGE}");
        return;
    }
    let ds = match load(dataset) {
        Ok(d) => d,
        Err(e) => {
            out(&api::err("DATASET", &e, ""));
            std::process::exit(3);
        }
    };
    match cmd.as_str() {
        "calc" => {
            let mut s = String::new();
            match args.get(1) {
                Some(f) if f != "-" => match std::fs::read_to_string(f) {
                    Ok(t) => s = t,
                    Err(e) => {
                        out(&api::err("IO", &format!("{f}: {e}"), ""));
                        std::process::exit(2);
                    }
                },
                _ => {
                    let _ = std::io::stdin().read_to_string(&mut s);
                }
            }
            let v = api::calc_str(&ds, &s);
            out(&v);
            if v.get("error").is_some() {
                std::process::exit(1);
            }
        }
        "batch" => {
            let stdin = std::io::stdin();
            let mut w = std::io::BufWriter::with_capacity(1 << 16, std::io::stdout().lock());
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                let v = api::calc_str(&ds, &line);
                let _ = serde_json::to_writer(&mut w, &v);
                let _ = w.write_all(b"\n");
            }
            let _ = w.flush();
        }
        "serve-stdio" => {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                let r: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(e) => {
                        out(&json!({"id": null, "error": {"code": "BAD_JSON", "message": e.to_string(), "path": ""}}));
                        continue;
                    }
                };
                let id = r.get("id").cloned().unwrap_or(Value::Null);
                let params = r.get("params").cloned().unwrap_or(Value::Null);
                let res = match r.get("method").and_then(|m| m.as_str()) {
                    Some("calc") => api::calc_value(&ds, params),
                    Some("meta") => eve_dogma_e::jv::Value::from(meta(&ds)),
                    m => api::err("UNKNOWN_METHOD", &format!("{m:?}"), "/method"),
                };
                if let Some(e) = res.get("error") {
                    out(&json!({"id": id, "error": e}));
                } else {
                    out(&json!({"id": id, "result": res}));
                }
            }
        }
        "meta" => out(&meta(&ds)),
        other => {
            out(&api::err("UNKNOWN_COMMAND", other, ""));
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}
