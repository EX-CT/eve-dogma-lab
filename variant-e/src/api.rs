//! Request -> response glue (stateless). GPL-3.0-or-later.
use crate::data::Dataset;
use crate::eos::cx::{CommandBonus, Fit, NONE, RT_EARLY, RT_LATE, RT_NORMAL};
use crate::eos::fit::BuildError;
use crate::request::FitRequest;
use crate::jv::Value;
use serde_json::json;

pub fn err(code: &str, message: &str, path: &str) -> Value {
    Value::from(json!({"error": {"code": code, "message": message, "path": path}}))
}

fn be(e: BuildError) -> Value {
    err(e.code, &e.message, &e.path)
}

/// Calculate one FitRequest given as JSON text.
pub fn calc_str(ds: &Dataset, text: &str) -> Value {
    match serde_json::from_str::<FitRequest>(text) {
        Ok(req) => match calc(ds, &req) {
            Ok(v) => v,
            Err(e) => be(e),
        },
        Err(e) => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(_) => err("BAD_REQUEST", &e.to_string(), ""),
            Err(e) => err("BAD_JSON", &e.to_string(), ""),
        },
    }
}

pub fn calc_value(ds: &Dataset, v: serde_json::Value) -> Value {
    let req: FitRequest = match serde_json::from_value(v) {
        Ok(r) => r,
        Err(e) => return err("BAD_REQUEST", &e.to_string(), ""),
    };
    match calc(ds, &req) {
        Ok(v) => v,
        Err(e) => be(e),
    }
}

pub fn calc(ds: &Dataset, req: &FitRequest) -> Result<Value, BuildError> {
    // command fits first (Pyfa: commandFits are calculated before the local fit)
    let mut bonuses: Vec<CommandBonus> = Vec::new();
    for (i, b) in req.fleet.booster_fits.iter().enumerate() {
        let mut bf = Fit::build(ds, b).map_err(|mut e| {
            e.path = format!("/fleet/booster_fits/{i}{}", e.path);
            e
        })?;
        for mut cb in bf.calculate_command() {
            cb.thing = NONE;
            match bonuses.iter_mut().find(|x| x.id == cb.id) {
                Some(x) => {
                    if x.value.abs() < cb.value.abs() {
                        *x = cb;
                    }
                }
                None => bonuses.push(cb),
            }
        }
    }
    let t0 = std::time::Instant::now();
    let mut fit = Fit::build(ds, req)?;
    let t1 = std::time::Instant::now();
    fit.command_bonuses = bonuses;
    let explicit: Vec<(u32, f64)> = req.fleet.buffs.iter().map(|b| (b.buff_id, b.value)).collect();
    fit.calculate(&explicit);
    // projected fits (Pyfa: after the local calculation, each projected fit runs its own calculation and
    // projects its drones/fighters/modules after every runtime)
    for (i, p) in req.projected.iter().enumerate() {
        if p.kind != "fit" {
            continue;
        }
        let Some(sreq) = &p.fit else { continue };
        let mut sreq = (**sreq).clone();
        sreq.projected.clear();
        let mut sf = Fit::build(ds, &sreq).map_err(|mut e| {
            e.path = format!("/projected/{i}/fit{}", e.path);
            e
        })?;
        sf.command_bonuses.clear();
        let mut mirror = Vec::new();
        for rt in [RT_EARLY, RT_NORMAL, RT_LATE] {
            sf.calc_rt(rt, &[], None);
            fit.project_from(&sf, rt, p.amount.max(1), p.distance_m, &mut mirror);
        }
    }
    let t2 = std::time::Instant::now();
    let out = fit.stats(req);
    if std::env::var_os("EVE_E_PROF").is_some() {
        let t3 = std::time::Instant::now();
        eprintln!("prof build={:?} calc={:?} stats={:?}", t1 - t0, t2 - t1, t3 - t2);
    }
    Ok(out)
}

/// `eft_export` RPC: params {fit: FitRequest, name?} -> {text}
pub fn eft_export_value(ds: &Dataset, params: serde_json::Value) -> Value {
    let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
    let Some(fit) = params.get("fit").cloned() else {
        return err("BAD_REQUEST", "missing params.fit", "/params/fit");
    };
    let req: FitRequest = match serde_json::from_value(fit) {
        Ok(r) => r,
        Err(e) => return err("BAD_REQUEST", &e.to_string(), "/params/fit"),
    };
    match crate::eft::export(ds, &req, &name) {
        Ok(t) => Value::from(json!({ "text": t })),
        Err(e) => be(e),
    }
}

/// `eft_parse` RPC: params {text} -> FitRequest JSON
pub fn eft_parse_value(ds: &Dataset, params: &serde_json::Value) -> Value {
    let text = params.get("text").and_then(|t| t.as_str()).unwrap_or("");
    match crate::eft_parse::parse(ds, text) {
        Ok(v) => Value::from(v),
        Err(e) => err("EFT_PARSE", &e, "/params/text"),
    }
}

/// Search kinds (interim spec, CONTRACT.md "Search"): published types of these categories.
const SEARCH_CATEGORIES: [(&str, u32); 8] =
    [("ship", 6), ("module", 7), ("charge", 8), ("drone", 18), ("fighter", 87), ("implant", 20), ("subsystem", 32), ("skill", 16)];

fn search_kind(ds: &Dataset, t: &crate::data::TypeInfo) -> Option<&'static str> {
    if t.category == 20 {
        return Some(if ds.group_name(t.group).contains("Booster") { "booster" } else { "implant" });
    }
    SEARCH_CATEGORIES.iter().find(|(_, c)| *c == t.category).map(|(n, _)| *n)
}

/// `search` RPC: params {query, limit? (20), kinds?} -> [{type_id, name, name_zh, group, category_id, kind, slot,
/// meta_level, match}], ranked exact > prefix > substring (case-insensitive, English or Chinese), ties by type id.
pub fn search_value(ds: &Dataset, params: &serde_json::Value) -> Value {
    let q = params.get("query").and_then(|q| q.as_str()).unwrap_or("").trim().to_lowercase();
    let limit = params.get("limit").and_then(|l| l.as_u64()).unwrap_or(20) as usize;
    let kinds: Option<Vec<String>> =
        params.get("kinds").and_then(|k| k.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect());
    let rank = |t: &crate::data::TypeInfo| -> Option<u8> {
        let en = t.name.to_lowercase();
        let zh = t.name_zh.as_deref().map(|z| z.to_lowercase()).unwrap_or_default();
        let hit = |f: &dyn Fn(&str) -> bool| f(&en) || (!zh.is_empty() && f(&zh));
        if hit(&|s| s == q) {
            Some(0)
        } else if hit(&|s| s.starts_with(&q)) {
            Some(1)
        } else if hit(&|s| s.contains(&q)) {
            Some(2)
        } else {
            None
        }
    };
    let mut hits: Vec<(u8, &crate::data::TypeInfo, &'static str)> = ds
        .types
        .iter()
        .filter(|t| t.published)
        .filter_map(|t| {
            let k = search_kind(ds, t)?;
            if let Some(ks) = &kinds {
                if !ks.iter().any(|x| x == k) {
                    return None;
                }
            }
            Some((rank(t)?, t, k))
        })
        .collect();
    hits.sort_by_key(|h| (h.0, h.1.id));
    Value::from(serde_json::Value::Array(
        hits.into_iter()
            .take(limit)
            .map(|(r, t, k)| {
                json!({"type_id": t.id, "name": t.name, "name_zh": t.name_zh, "kind": k, "match": (["exact", "prefix", "substring"][r as usize]),
                       "group": ds.group_names.get(&t.group), "category_id": t.category, "meta_level": t.meta_level,
                       "slot": crate::eos::fit::infer_slot(t).map(crate::eos::stats::slot_name)})
            })
            .collect(),
    ))
}

/// `type` RPC: params {id: type id or name} -> type info with attributes (by name) and effects
pub fn type_value(ds: &Dataset, params: &serde_json::Value) -> Value {
    let key = match params.get("id") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(v) if !v.is_null() => v.to_string(),
        _ => return err("BAD_REQUEST", "missing params.id", "/params/id"),
    };
    let id = key.trim().parse::<u32>().ok().or_else(|| ds.types.by_name_ci(&key));
    let Some(t) = id.and_then(|i| ds.types.get(&i)) else {
        return err("UNKNOWN_TYPE", &key, "/params/id");
    };
    let attrs: serde_json::Map<String, serde_json::Value> =
        t.attrs.iter().map(|(a, v)| (ds.attrs.get(a).map(|x| x.name.clone()).unwrap_or(a.to_string()), json!(v))).collect();
    let effects: Vec<serde_json::Value> =
        t.effects.iter().map(|(e, d)| json!({"id": e, "name": ds.effects.get(e).map(|x| x.name.clone()), "default": d})).collect();
    let a = |id: u32| t.attr(id).unwrap_or(0.0);
    Value::from(json!({"type_id": t.id, "name": t.name, "name_zh": t.name_zh, "group": ds.group_names.get(&t.group), "group_id": t.group,
        "category_id": t.category, "published": t.published, "mass": a(crate::data::A_MASS), "volume": a(crate::data::A_VOLUME),
        "capacity": a(crate::data::A_CAPACITY), "meta_level": t.meta_level, "slot": crate::eos::fit::infer_slot(t).map(crate::eos::stats::slot_name),
        "attributes": attrs, "effects": effects}))
}
