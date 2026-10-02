//! eve-dogma-h CLI — stateless: JSON FitRequest in, JSON FitStats out (contract v1).
use eve_dogma_h::tools;
use eve_dogma_h::{calc, calc_json, Dataset, FitRequest};
use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};
use std::time::Instant;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const USAGE: &str = "eve-dogma-h <command> [--dataset PATH] [args]

Commands:
  calc [FILE]          FitRequest JSON (file or stdin) -> FitStats JSON
  batch                JSONL FitRequests on stdin -> JSONL FitStats on stdout (same order)
  serve-stdio          JSONL RPC {\"id\",\"method\",\"params\"} -> {\"id\",\"result\"}; methods calc, eft_parse {text},
                       eft_export {fit,name?}, search {query,limit?}, type {id}, meta
  eft [FILE]           EFT text (file or stdin) -> FitRequest JSON (--calc: compute it, --skills N: all skills at N)
  search QUERY         search types by name (--limit N)
  type ID|NAME         type with base attributes and effects
  meta                 dataset info
  bench [FILE] [-n N]  time N calculations of one request

Dataset: --dataset PATH, else $EVE_DOGMA_DATASET, else ./dataset.json.gz";

const SHARED_DATASET: &str = "/workspace/exct-eve/data/dataset-3569502.json.gz";

fn load(path: Option<String>) -> Dataset {
    let p = path.or_else(|| std::env::var("EVE_DOGMA_DATASET").ok()).unwrap_or_else(|| {
        if !std::path::Path::new("dataset.json.gz").exists() && std::path::Path::new(SHARED_DATASET).exists() {
            SHARED_DATASET.into()
        } else {
            "dataset.json.gz".into()
        }
    });
    match Dataset::load_path_cached(&p) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(3)
        }
    }
}

fn read_input(file: Option<&String>) -> String {
    let mut s = String::new();
    match file {
        Some(f) if f != "-" => {
            s = std::fs::read_to_string(f).unwrap_or_else(|e| {
                eprintln!("error: {f}: {e}");
                std::process::exit(2)
            })
        }
        _ => {
            std::io::stdin().read_to_string(&mut s).unwrap();
        }
    }
    s
}

fn meta(ds: &Dataset) -> Value {
    json!({"engine": concat!("eve-dogma-h ", env!("CARGO_PKG_VERSION")), "schema_version": 1, "sde_build": ds.build,
           "dataset_sha256": ds.sha256, "types": ds.types.len(), "attributes": ds.attrs.len(), "effects": ds.effects.len()})
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut take = |f: &str| -> Option<String> {
        let p = args.iter().position(|a| a == f)?;
        let v = args.get(p + 1).cloned();
        args.drain(p..(p + 2).min(args.len()));
        v
    };
    let dataset = take("--dataset");
    let n: usize = take("-n").and_then(|v| v.parse().ok()).unwrap_or(1000);
    let skills: Option<u8> = take("--skills").and_then(|v| v.parse().ok());
    let limit: Option<usize> = take("--limit").and_then(|v| v.parse().ok());
    let do_calc = if let Some(p) = args.iter().position(|a| a == "--calc") {
        args.remove(p);
        true
    } else {
        false
    };
    let cmd = args.first().cloned().unwrap_or_default();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    match cmd.as_str() {
        "calc" => {
            let s = read_input(args.get(1));
            let ds = load(dataset);
            let res = calc_json(&ds, &s);
            writeln!(out, "{res}").unwrap();
            out.flush().unwrap();
            if res.starts_with("{\"error\"") {
                std::process::exit(2);
            }
        }
        "batch" => {
            let ds = load(dataset);
            for line in std::io::stdin().lock().lines() {
                let line = line.unwrap();
                if line.trim().is_empty() {
                    continue;
                }
                writeln!(out, "{}", calc_json(&ds, &line)).unwrap();
            }
        }
        "serve-stdio" => {
            let ds = load(dataset);
            for line in std::io::stdin().lock().lines() {
                let line = line.unwrap();
                if line.trim().is_empty() {
                    continue;
                }
                let resp = match serde_json::from_str::<Value>(&line) {
                    Err(e) => json!({"id": null, "error": {"code": "BAD_JSON", "message": e.to_string()}}),
                    Ok(v) => {
                        let id = v.get("id").cloned().unwrap_or(Value::Null);
                        let p = v.get("params").cloned().unwrap_or(Value::Null);
                        let result = match v.get("method").and_then(|m| m.as_str()).unwrap_or("calc") {
                            "calc" => match serde_json::from_value::<FitRequest>(p) {
                                Ok(r) => calc(&ds, &r),
                                Err(e) => json!({"error": {"code": "BAD_REQUEST", "message": e.to_string()}}),
                            },
                            "meta" => meta(&ds),
                            "eft_parse" => {
                                let text = p.get("text").and_then(|t| t.as_str()).unwrap_or("");
                                let sk = p.get("skills").and_then(|t| t.as_u64()).map(|x| x.min(5) as u8);
                                tools::eft_parse(&ds, text, sk).unwrap_or_else(|e| json!({"error": {"code": "EFT_PARSE", "message": e.0}}))
                            }
                            "eft_export" => {
                                let name = p.get("name").and_then(|t| t.as_str()).map(str::to_string);
                                match serde_json::from_value::<FitRequest>(p.get("fit").cloned().unwrap_or(Value::Null)) {
                                    Ok(r) => json!({"text": tools::eft_export(&ds, &r, name.as_deref())}),
                                    Err(e) => json!({"error": {"code": "BAD_REQUEST", "message": e.to_string()}}),
                                }
                            }
                            "search" => {
                                let q = p.get("query").and_then(|t| t.as_str()).unwrap_or("");
                                {
                                    let kinds: Option<Vec<String>> = p.get("kinds").and_then(|k| k.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect());
                                    tools::search(&ds, q, p.get("limit").and_then(|l| l.as_u64()).map(|l| l as usize), kinds.as_deref())
                                }
                            }
                            "type" => {
                                let key = match p.get("id") {
                                    Some(Value::String(s)) => s.clone(),
                                    Some(v) => v.to_string(),
                                    None => String::new(),
                                };
                                tools::type_info(&ds, &key)
                                    .unwrap_or_else(|| json!({"error": {"code": "UNKNOWN_TYPE", "message": format!("unknown type '{key}'")}}))
                            }
                            m => json!({"error": {"code": "UNKNOWN_METHOD", "message": m}}),
                        };
                        json!({"id": id, "result": result})
                    }
                };
                writeln!(out, "{}", serde_json::to_string(&resp).unwrap()).unwrap();
                out.flush().unwrap();
            }
        }
        "eft" => {
            let text = read_input(args.get(1));
            let ds = load(dataset);
            match tools::eft_parse(&ds, &text, skills) {
                Ok(req) => {
                    let res = if do_calc {
                        match serde_json::from_value::<FitRequest>(req) {
                            Ok(r) => calc(&ds, &r),
                            Err(e) => json!({"error": {"code": "BAD_REQUEST", "message": e.to_string()}}),
                        }
                    } else {
                        req
                    };
                    writeln!(out, "{}", serde_json::to_string_pretty(&res).unwrap()).unwrap();
                }
                Err(e) => {
                    writeln!(out, "{}", json!({"error": {"code": "EFT_PARSE", "message": e.0}})).unwrap();
                    out.flush().unwrap();
                    std::process::exit(2);
                }
            }
        }
        "search" => {
            let ds = load(dataset);
            let q = args[1..].join(" ");
            writeln!(out, "{}", serde_json::to_string_pretty(&tools::search(&ds, &q, limit, None)).unwrap()).unwrap();
        }
        "type" => {
            let ds = load(dataset);
            let key = args[1..].join(" ");
            match tools::type_info(&ds, &key) {
                Some(v) => writeln!(out, "{}", serde_json::to_string_pretty(&v).unwrap()).unwrap(),
                None => {
                    writeln!(out, "{}", json!({"error": {"code": "UNKNOWN_TYPE", "message": format!("unknown type '{key}'")}})).unwrap();
                    out.flush().unwrap();
                    std::process::exit(2);
                }
            }
        }
        "meta" => {
            let t0 = Instant::now();
            let ds = load(dataset);
            let mut m = meta(&ds);
            m["load_ms"] = json!(t0.elapsed().as_secs_f64() * 1000.0);
            writeln!(out, "{}", serde_json::to_string_pretty(&m).unwrap()).unwrap();
        }
        "bench" => {
            let t0 = Instant::now();
            let ds = load(dataset);
            let load_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let s = read_input(args.get(1));
            let req: FitRequest = serde_json::from_str(&s).expect("bad request");
            let _ = calc(&ds, &req);
            let t1 = Instant::now();
            for _ in 0..n {
                std::hint::black_box(calc(&ds, &req));
            }
            let el = t1.elapsed().as_secs_f64();
            writeln!(out, "{}", json!({"dataset_load_ms": load_ms, "iterations": n, "total_s": el, "per_calc_us": el / n as f64 * 1e6})).unwrap();
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(if cmd.is_empty() || cmd == "help" || cmd == "--help" { 0 } else { 2 });
        }
    }
}
