//! Non-calculation helpers of the contract: type search, type info, EFT import/export.
use crate::data::{Dataset, TypeInfo};
use crate::fit::infer_slot;
use crate::request::{FitRequest, Slot, State};
use serde_json::{json, Map, Value};

/// categories offered by `search` (contract v1.4.1): ship, module, charge, skill, drone, implant/booster,
/// subsystem, fighter
const SEARCH_CATEGORIES: &[u32] = &[6, 7, 8, 16, 18, 20, 32, 87];

fn group_name(ds: &Dataset, t: &TypeInfo) -> Value {
    ds.group_names.get(&t.group).map(|g| json!(g)).unwrap_or(Value::Null)
}

fn slot_name(t: &TypeInfo) -> Value {
    if !matches!(t.category, 7 | 32 | 66) {
        return Value::Null;
    }
    infer_slot(t).map(|s| serde_json::to_value(s).unwrap_or(Value::Null)).unwrap_or(Value::Null)
}

/// Case-insensitive search over English and Chinese names (contract v1.4.1): exact > prefix > substring (best of
/// the two names), ties by type id ascending; published types only; default limit 20.
pub fn search(ds: &Dataset, query: &str, limit: Option<usize>) -> Value {
    let q = query.trim().to_lowercase();
    let limit = limit.unwrap_or(20);
    if q.is_empty() {
        return json!([]);
    }
    let mut hits: Vec<(u8, u32)> = Vec::new();
    for t in ds.types.values() {
        if !SEARCH_CATEGORIES.contains(&t.category) || !t.published || t.name.is_empty() {
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
        let best = [rank(&t.name), t.name_zh.as_deref().and_then(rank)].into_iter().flatten().min();
        if let Some(r) = best {
            hits.push((r, t.id));
        }
    }
    hits.sort();
    hits.truncate(limit);
    Value::Array(
        hits.into_iter()
            .map(|(_, id)| {
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
    // Section semantics (as Pyfa's importer): a blank-line-separated section made only of drones is the drone
    // bay (likewise fighters); in a mixed section a drone/fighter goes to its bay only when no pure bay exists.
    let body = &lines[hi + 1..body_end];
    let mut sec_of = vec![0usize; body.len()];
    let mut sec_kinds: Vec<Vec<u32>> = vec![Vec::new()]; // per section: category of each line (0 = no quantity)
    for (j, raw) in body.iter().enumerate() {
        let l = raw.trim();
        if l.is_empty() {
            if !sec_kinds.last().unwrap().is_empty() {
                sec_kinds.push(Vec::new());
            }
            continue;
        }
        sec_of[j] = sec_kinds.len() - 1;
        let (l0, _) = split_ref(l);
        let (l1, qty) = split_qty(l0);
        let cat = ds.type_by_name(l1.split(',').next().unwrap_or("").trim()).map(|t| ds.types[&t].category).unwrap_or(0);
        sec_kinds.last_mut().unwrap().push(if qty.is_some() { cat } else { 0 });
    }
    let pure = |k: &Vec<u32>, c: u32| !k.is_empty() && k.iter().all(|&x| x == c);
    let has_bay = |c: u32| sec_kinds.iter().any(|k| pure(k, c));
    let (drone_bay, fighter_bay) = (has_bay(18), has_bay(87));
    for (j, raw) in body.iter().enumerate() {
        let l = raw.trim();
        if l.is_empty() || (l.starts_with("[Empty ") && l.ends_with(']')) {
            continue;
        }
        let sk = &sec_kinds[sec_of[j]];
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
                18 if pure(sk, 18) || !drone_bay => drones.push(json!({"type_id": tid, "quantity": q, "active": q, "mutation": mutation})),
                87 if pure(sk, 87) || !fighter_bay => fighters.push(json!({"type_id": tid, "quantity": q, "active": true, "abilities": null})),
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

/// Market groups of the drone ordering used by Pyfa's EFT exporter (light scout, light hybrid, medium scout,
/// medium hybrid, heavy attack, heavy hybrid, sentry, combat utility, ewar, logistic, mining, salvage).
const DRONE_MARKET_ORDER: &[&[u32]] = &[
    &[837, 1531], &[3881], &[838, 1532], &[3882], &[359, 839], &[3883], &[911, 1533], &[843, 1586], &[841, 1029],
    &[842, 1030], &[158, 358], &[1643, 1646],
];
const FIGHTER_GROUP_ORDER: &[&str] = &[
    "Light Fighter", "Structure Light Fighter", "Heavy Fighter", "Structure Heavy Fighter", "Support Fighter",
    "Structure Support Fighter",
];

/// Python `repr(float)`-style number: shortest round-trip digits, always with a decimal point.
fn py_float(v: f64) -> String {
    let s = format!("{v}");
    if v.is_finite() && !s.contains('.') && !s.contains('e') {
        format!("{s}.0")
    } else {
        s
    }
}

/// FitRequest -> EFT text, laid out like Pyfa's EFT exporter: header, then sections separated by two blank
/// lines (racks low/mid/high/rig/subsystem/service separated by one; drones then fighters; implants then
/// boosters; cargo; mutation details). No trailing newline, no T3D mode line (Pyfa writes none).
pub fn eft_export(ds: &Dataset, req: &FitRequest, name: Option<&str>) -> String {
    let tn = |id: u32| ds.types.get(&id).map(|t| t.name.clone()).unwrap_or_else(|| format!("#{id}"));
    let header = format!("[{}, {}]", tn(req.ship.type_id), name.unwrap_or("EXCT fit"));
    let mut muta: Vec<String> = Vec::new();
    let mut mref = |m: &Option<crate::request::Mutation>| -> String {
        let Some(m) = m else { return String::new() };
        let Some(mp) = m.mutaplasmid_type_id else { return String::new() };
        let n = muta.len() + 1;
        // Pyfa lists every attribute the mutaplasmid rolls: the given value (clamped into the roll range) or
        // the base item's value
        let base_t = ds.types.get(&m.base_type_id);
        let mut attrs: Vec<(String, f64)> = Vec::new();
        if let Some(mu) = ds.mutaplasmids.get(&mp) {
            for (k, (lo, hi)) in &mu.attrs {
                let Ok(aid) = k.parse::<u32>() else { continue };
                let Some(bv) = base_t.and_then(|t| t.attr(aid)) else { continue };
                let an = ds.attrs.get(&aid).map(|a| a.name.clone()).unwrap_or_else(|| k.clone());
                let v = match m.attributes.get(k) {
                    None => bv,
                    Some(_) if bv == 0.0 => 0.0,
                    Some(&v) => {
                        let (lo, hi) = ((lo * 1000.0).round() / 1000.0, (hi * 1000.0).round() / 1000.0);
                        let r = v / bv;
                        if lo <= r && r <= hi {
                            v
                        } else {
                            let (x, y) = (lo * bv, hi * bv);
                            v.clamp(x.min(y), x.max(y))
                        }
                    }
                };
                attrs.push((an, v));
            }
        }
        attrs.sort_by(|a, b| a.0.cmp(&b.0));
        let line: Vec<String> = attrs.iter().map(|(a, v)| format!("{a} {}", py_float(*v))).collect();
        muta.push(format!("[{n}] {}\n  {}\n  {}", tn(m.base_type_id), tn(mp), line.join(", ")));
        format!(" [{n}]")
    };
    let mut sections: Vec<String> = Vec::new();
    let mut racks: Vec<String> = Vec::new();
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
                l.push_str(" /offline");
            }
            l.push_str(&mref(&m.mutation));
            lines.push(l);
        }
        if !lines.is_empty() {
            racks.push(lines.join("\n"));
        }
    }
    if !racks.is_empty() {
        sections.push(racks.join("\n\n"));
    }
    // drones: market-group order, plain before mutated, then name
    let drone_key = |d: &crate::request::DroneReq| {
        let base = d.mutation.as_ref().map(|x| x.base_type_id).unwrap_or(d.type_id);
        let mg = ds.types.get(&base).and_then(|t| t.market_group);
        let ord = mg.and_then(|g| DRONE_MARKET_ORDER.iter().position(|s| s.contains(&g))).unwrap_or(usize::MAX);
        let mutated = d.mutation.as_ref().map(|m| m.mutaplasmid_type_id.is_some()).unwrap_or(false);
        let full = match d.mutation.as_ref().and_then(|m| m.mutaplasmid_type_id) {
            Some(mp) => format!("{} {}", tn(mp), tn(base)),
            None => tn(d.type_id),
        };
        (ord, mutated, full)
    };
    let mut drones: Vec<&crate::request::DroneReq> = req.drones.iter().collect();
    drones.sort_by_cached_key(|d| drone_key(d));
    let mut minions = Vec::new();
    let dl: Vec<String> = drones
        .iter()
        .map(|d| format!("{} x{}{}", tn(d.mutation.as_ref().map(|x| x.base_type_id).unwrap_or(d.type_id)), d.quantity, mref(&d.mutation)))
        .collect();
    if !dl.is_empty() {
        minions.push(dl.join("\n"));
    }
    let mut fighters: Vec<_> = req.fighters.iter().collect();
    fighters.sort_by_cached_key(|f| {
        let g = ds.types.get(&f.type_id).and_then(|t| ds.group_names.get(&t.group)).cloned().unwrap_or_default();
        (FIGHTER_GROUP_ORDER.iter().position(|x| *x == g).unwrap_or(usize::MAX), tn(f.type_id))
    });
    let fl: Vec<String> = fighters
        .iter()
        .map(|f| {
            // Pyfa stores a full (or over-full) squadron as "max size"
            let max = ds.types.get(&f.type_id).and_then(|t| t.attr(ds.a.fighter_sq_max)).unwrap_or(1.0) as u32;
            let q = f.quantity.map(|q| q.min(max)).unwrap_or(max);
            format!("{} x{}", tn(f.type_id), q)
        })
        .collect();
    if !fl.is_empty() {
        minions.push(fl.join("\n"));
    }
    if !minions.is_empty() {
        sections.push(minions.join("\n\n"));
    }
    let slot_of = |id: u32, attr: &str| ds.types.get(&id).and_then(|t| t.attr(ds.attr_id(attr))).unwrap_or(0.0);
    let mut imps: Vec<u32> = req.implants.clone();
    imps.sort_by(|a, b| slot_of(*a, "implantness").total_cmp(&slot_of(*b, "implantness")));
    let mut boos: Vec<u32> = req.boosters.iter().map(|b| b.type_id).collect();
    boos.sort_by(|a, b| slot_of(*a, "boosterness").total_cmp(&slot_of(*b, "boosterness")));
    let mut chars = Vec::new();
    if !imps.is_empty() {
        chars.push(imps.iter().map(|&i| tn(i)).collect::<Vec<_>>().join("\n"));
    }
    if !boos.is_empty() {
        chars.push(boos.iter().map(|&i| tn(i)).collect::<Vec<_>>().join("\n"));
    }
    if !chars.is_empty() {
        sections.push(chars.join("\n\n"));
    }
    let mut cargo: Vec<_> = req.cargo.iter().collect();
    cargo.sort_by_cached_key(|c| {
        let t = ds.types.get(&c.type_id);
        let cat = t.and_then(|t| ds.category_names.get(&t.category)).cloned().unwrap_or_default();
        let grp = t.and_then(|t| ds.group_names.get(&t.group)).cloned().unwrap_or_default();
        (cat, grp, tn(c.type_id))
    });
    if !cargo.is_empty() {
        sections.push(cargo.iter().map(|c| format!("{} x{}", tn(c.type_id), c.quantity)).collect::<Vec<_>>().join("\n"));
    }
    if !muta.is_empty() {
        sections.push(muta.join("\n"));
    }
    format!("{header}\n\n{}", sections.join("\n\n\n"))
}
