//! Non-calculation helpers of the contract: type search, type info, EFT import/export.
use crate::data::{Dataset, TypeInfo};
use crate::fit::infer_slot;
use crate::request::{FitRequest, Slot, State};
use serde_json::{json, Map, Value};

/// categories offered by `search` (things that go into or affect a fit)
const SEARCH_CATEGORIES: &[u32] = &[2, 6, 7, 8, 16, 18, 20, 32, 87];

fn group_name(ds: &Dataset, t: &TypeInfo) -> Value {
    ds.group_names.get(&t.group).map(|g| json!(g)).unwrap_or(Value::Null)
}

fn slot_name(t: &TypeInfo) -> Value {
    if !matches!(t.category, 7 | 32 | 66) {
        return Value::Null;
    }
    infer_slot(t).map(|s| serde_json::to_value(s).unwrap_or(Value::Null)).unwrap_or(Value::Null)
}

/// Case-insensitive search over English and Chinese names. Exact matches come first, then prefix matches, then
/// substring matches; ties go to shorter names, then by name and type id.
pub fn search(ds: &Dataset, query: &str, limit: Option<usize>) -> Value {
    let q = query.trim().to_lowercase();
    let limit = limit.unwrap_or(20);
    if q.is_empty() {
        return json!([]);
    }
    let mut hits: Vec<(u8, usize, &str, u32)> = Vec::new();
    for t in ds.types.values() {
        let beacon = t.category == 2 && ds.group_names.get(&t.group).map(|g| g == "Effect Beacon").unwrap_or(false);
        let mutaplasmid = ds.mutaplasmids.contains_key(&t.id);
        let wanted = beacon || mutaplasmid || (t.category != 2 && SEARCH_CATEGORIES.contains(&t.category) && t.published);
        if !wanted || t.name.is_empty() {
            continue;
        }
        let rank = |name: &str| -> Option<u8> {
            let n = name.to_lowercase();
            if n == q {
                Some(0)
            } else if n.starts_with(&q) {
                Some(1)
            } else if n.contains(&q) {
                Some(2)
            } else {
                None
            }
        };
        // rank from whichever name matched best; ties ordered by the English name's length
        let en = rank(&t.name).map(|r| (r, t.name.chars().count()));
        let zh = t.name_zh.as_deref().and_then(|z| rank(z).map(|r| (r, t.name.chars().count())));
        let best = match (en, zh) {
            (Some(a), Some(b)) => Some(if b.0 < a.0 { b } else { a }),
            (a, b) => a.or(b),
        };
        if let Some((r, len)) = best {
            hits.push((r, len, &t.name, t.id));
        }
    }
    hits.sort();
    hits.truncate(limit);
    Value::Array(
        hits.into_iter()
            .map(|(_, _, _, id)| {
                let t = &ds.types[&id];
                json!({"type_id": id, "name": t.name, "name_zh": t.name_zh, "group": group_name(ds, t),
                       "category_id": t.category, "meta_level": t.meta_level, "slot": slot_name(t)})
            })
            .collect(),
    )
}

/// Type by id or (case-insensitive) name, with base attributes by name and effects.
pub fn type_info(ds: &Dataset, key: &str) -> Option<Value> {
    let key = key.trim();
    let id = key.parse::<u32>().ok().filter(|i| ds.types.contains_key(i)).or_else(|| ds.type_by_name(key))?;
    let t = &ds.types[&id];
    let mut attrs = Map::new();
    for &(a, v) in &t.attrs {
        if matches!(a, 4 | 38 | 161 | 162) {
            continue; // type-level fields, reported at top level
        }
        let name = ds.attrs.get(&a).map(|x| x.name.clone()).unwrap_or_else(|| a.to_string());
        attrs.insert(name, json!(v));
    }
    let effects: Vec<Value> = t
        .effects
        .iter()
        .map(|&(e, d)| json!({"id": e, "name": ds.effects.get(&e).map(|x| x.name.as_str()).unwrap_or(""), "default": d}))
        .collect();
    Some(json!({"type_id": id, "name": t.name, "name_zh": t.name_zh, "group": group_name(ds, t), "group_id": t.group,
                "category_id": t.category, "published": t.published, "mass": t.mass, "volume": t.volume,
                "capacity": t.capacity, "slot": slot_name(t), "attributes": Value::Object(attrs), "effects": effects}))
}

#[derive(Debug)]
pub struct EftError(pub String);

struct MutaBlock {
    base: u32,
    mutaplasmid: Option<u32>,
    attributes: Map<String, Value>,
}

fn lookup(ds: &Dataset, name: &str) -> Result<u32, EftError> {
    ds.type_by_name(name).ok_or_else(|| EftError(format!("unknown item '{}'", name.trim())))
}

/// split a trailing " [N]" mutation reference off a line
fn split_ref(line: &str) -> (&str, Option<u32>) {
    let l = line.trim_end();
    if l.ends_with(']') {
        if let Some(i) = l.rfind(" [") {
            if let Ok(n) = l[i + 2..l.len() - 1].parse::<u32>() {
                return (l[..i].trim_end(), Some(n));
            }
        }
    }
    (l, None)
}

/// split a trailing " xN" quantity
fn split_qty(line: &str) -> (&str, Option<u32>) {
    if let Some(i) = line.rfind(" x") {
        if let Ok(n) = line[i + 2..].trim().parse::<u32>() {
            return (line[..i].trim_end(), Some(n));
        }
    }
    (line, None)
}

/// EFT text -> FitRequest (as JSON with every contract field). `skills`: default skill level for all skills.
pub fn eft_parse(ds: &Dataset, text: &str, skills: Option<u8>) -> Result<Value, EftError> {
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    let mut it = lines.iter().map(|l| l.trim()).enumerate().skip_while(|(_, l)| l.is_empty());
    let (hi, header) = it.next().ok_or_else(|| EftError("empty EFT text".into()))?;
    let inner = header.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(header);
    let ship_name = inner.split(',').next().unwrap_or("").trim();
    let ship = ds
        .type_by_name(ship_name)
        .filter(|id| matches!(ds.types[id].category, 6 | 65))
        .ok_or_else(|| EftError(format!("unknown ship '{ship_name}'")))?;

    // mutation blocks: "[N] Base type" / "  Mutaplasmid" / "  attr v, attr v"
    let mut blocks: Vec<(u32, MutaBlock)> = Vec::new();
    let mut body_end = lines.len();
    let mut i = hi + 1;
    while i < lines.len() {
        let l = lines[i].trim();
        let is_block = l.starts_with('[') && l[1..].split(']').next().map(|n| n.parse::<u32>().is_ok()).unwrap_or(false);
        if !is_block {
            i += 1;
            continue;
        }
        body_end = body_end.min(i);
        let close = l.find(']').unwrap();
        let n: u32 = l[1..close].parse().unwrap();
        let base = lookup(ds, &l[close + 1..])?;
        let mut mb = MutaBlock { base, mutaplasmid: None, attributes: Map::new() };
        i += 1;
        while i < lines.len() && lines[i].starts_with(|c: char| c.is_whitespace()) && !lines[i].trim().is_empty() {
            let x = lines[i].trim();
            // first indented line: the mutaplasmid; then "attr value, attr value" lines
            let first = mb.mutaplasmid.is_none() && mb.attributes.is_empty();
            if first && ds.type_by_name(x).is_some() {
                mb.mutaplasmid = ds.type_by_name(x);
            } else {
                for part in x.split(',') {
                    let mut w = part.split_whitespace();
                    if let (Some(an), Some(v)) = (w.next(), w.next()) {
                        let aid = ds.attr_id(an);
                        if let (true, Ok(v)) = (aid != 0, v.parse::<f64>()) {
                            mb.attributes.insert(aid.to_string(), json!(v));
                        }
                    }
                }
            }
            i += 1;
        }
        blocks.push((n, mb));
    }
    let mutation_of = |r: Option<u32>| -> Option<(u32, Value)> {
        let (_, mb) = blocks.iter().find(|(n, _)| Some(*n) == r)?;
        let out = mb
            .mutaplasmid
            .and_then(|m| ds.mutaplasmids.get(&m))
            .and_then(|mi| mi.mapping.iter().find(|mp| mp.inputs.contains(&mb.base)).map(|mp| mp.output))
            .unwrap_or(mb.base);
        Some((out, json!({"base_type_id": mb.base, "mutaplasmid_type_id": mb.mutaplasmid, "attributes": mb.attributes})))
    };

    let (mut modules, mut drones, mut fighters, mut implants, mut boosters, mut cargo) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut mode: Option<u32> = None;
    for raw in &lines[hi + 1..body_end] {
        let l = raw.trim();
        if l.is_empty() || (l.starts_with("[Empty ") && l.ends_with(']')) {
            continue;
        }
        let (l, r) = split_ref(l);
        let (l, offline) = match l.to_lowercase().rfind(" /offline") {
            Some(p) if p + 9 == l.len() => (l[..p].trim_end(), true),
            _ => (l, false),
        };
        let (l, qty) = split_qty(l);
        let (name, charge) = match l.split_once(',') {
            Some((a, b)) => (a.trim(), Some(b.trim())),
            None => (l.trim(), None),
        };
        let mut tid = lookup(ds, name)?;
        let mut mutation = Value::Null;
        if let Some((out, m)) = mutation_of(r) {
            tid = out;
            mutation = m;
        }
        let t = &ds.types[&tid];
        if let Some(q) = qty {
            match t.category {
                18 => drones.push(json!({"type_id": tid, "quantity": q, "active": q, "mutation": mutation})),
                87 => fighters.push(json!({"type_id": tid, "quantity": q, "active": true, "abilities": null})),
                _ => cargo.push(json!({"type_id": tid, "quantity": q})),
            }
            continue;
        }
        if t.group == 1306 {
            mode = Some(tid); // tactical destroyer mode line
            continue;
        }
        match t.category {
            7 | 32 | 66 => {
                let slot = if t.category == 32 { Some(Slot::Subsystem) } else { infer_slot(t) };
                let passive_slot = matches!(slot, Some(Slot::Rig) | Some(Slot::Subsystem));
                let state = if offline { State::Offline } else if passive_slot { State::Online } else { State::Active };
                let charge_id = match charge {
                    Some(c) if !c.is_empty() => Some(lookup(ds, c)?),
                    _ => None,
                };
                modules.push(json!({"type_id": tid, "slot": slot, "state": state, "charge_type_id": charge_id,
                                    "mutation": mutation, "spool": null}));
            }
            20 => {
                if ds.group_names.get(&t.group).map(|g| g == "Booster").unwrap_or(false) {
                    boosters.push(json!({"type_id": tid, "side_effects": []}));
                } else {
                    implants.push(json!(tid));
                }
            }
            18 => drones.push(json!({"type_id": tid, "quantity": 1, "active": 1, "mutation": mutation})),
            87 => fighters.push(json!({"type_id": tid, "quantity": null, "active": true, "abilities": null})),
            _ => cargo.push(json!({"type_id": tid, "quantity": 1})),
        }
    }
    let req = json!({
        "schema_version": 1,
        "ship": {"type_id": ship, "mode_type_id": mode},
        "character": {"skills": {"default_level": skills, "levels": {}}},
        "modules": modules, "drones": drones, "fighters": fighters, "implants": implants, "boosters": boosters, "cargo": cargo,
    });
    let parsed: FitRequest = serde_json::from_value(req).map_err(|e| EftError(format!("internal: {e}")))?;
    serde_json::to_value(parsed).map_err(|e| EftError(format!("internal: {e}")))
}

/// FitRequest -> EFT text (low, mid, high, rig, subsystem, service racks; drones; fighters; implants + boosters; cargo).
pub fn eft_export(ds: &Dataset, req: &FitRequest, name: Option<&str>) -> String {
    let tn = |id: u32| ds.types.get(&id).map(|t| t.name.clone()).unwrap_or_else(|| format!("#{id}"));
    let mut out = format!("[{}, {}]\n", tn(req.ship.type_id), name.unwrap_or("EXCT fit"));
    let mut sections: Vec<Vec<String>> = Vec::new();
    let mut muta: Vec<String> = Vec::new();
    let mut mref = |m: &Option<crate::request::Mutation>| -> String {
        let Some(m) = m else { return String::new() };
        let n = muta.len() / 3 + 1;
        let attrs: Vec<String> = m
            .attributes
            .iter()
            .map(|(k, v)| {
                let an = k.parse::<u32>().ok().and_then(|a| ds.attrs.get(&a)).map(|a| a.name.clone()).unwrap_or_else(|| k.clone());
                format!("{an} {v}")
            })
            .collect();
        muta.push(format!("[{n}] {}", tn(m.base_type_id)));
        muta.push(format!("  {}", m.mutaplasmid_type_id.map(tn).unwrap_or_default()));
        muta.push(format!("  {}", attrs.join(", ")));
        format!(" [{n}]")
    };
    for rack in [Slot::Low, Slot::Mid, Slot::High, Slot::Rig, Slot::Subsystem, Slot::Service] {
        let mut lines = Vec::new();
        for m in &req.modules {
            let slot = m.slot.or_else(|| ds.types.get(&m.type_id).and_then(infer_slot));
            if slot != Some(rack) {
                continue;
            }
            let mut l = tn(m.mutation.as_ref().map(|x| x.base_type_id).unwrap_or(m.type_id));
            if let Some(c) = m.charge_type_id {
                l.push_str(&format!(", {}", tn(c)));
            }
            if m.state == Some(State::Offline) {
                l.push_str(" /OFFLINE");
            }
            l.push_str(&mref(&m.mutation));
            lines.push(l);
        }
        sections.push(lines);
    }
    sections.push(
        req.drones
            .iter()
            .map(|d| format!("{} x{}{}", tn(d.mutation.as_ref().map(|x| x.base_type_id).unwrap_or(d.type_id)), d.quantity.max(1), mref(&d.mutation)))
            .collect(),
    );
    sections.push(
        req.fighters
            .iter()
            .map(|f| {
                let q = f.quantity.unwrap_or_else(|| ds.types.get(&f.type_id).and_then(|t| t.attr(ds.a.fighter_sq_max)).unwrap_or(1.0) as u32);
                format!("{} x{}", tn(f.type_id), q)
            })
            .collect(),
    );
    let mut ib: Vec<String> = req.implants.iter().map(|&i| tn(i)).collect();
    ib.extend(req.boosters.iter().map(|b| tn(b.type_id)));
    sections.push(ib);
    sections.push(req.cargo.iter().map(|c| format!("{} x{}", tn(c.type_id), c.quantity)).collect());
    if let Some(m) = req.ship.mode_type_id {
        sections.push(vec![tn(m)]);
    }
    let body: Vec<String> = sections.into_iter().filter(|s| !s.is_empty()).map(|s| s.join("\n") + "\n").collect();
    out.push_str(&body.join("\n"));
    if !muta.is_empty() {
        out.push('\n');
        out.push_str(&muta.join("\n"));
        out.push('\n');
    }
    out
}
