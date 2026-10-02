//! EFT text import (`eft_parse` RPC / `eft` CLI helper): EFT text -> FitRequest JSON.
//! Same rules and output shape as the reference engine (eve-dogma-rs `eft::parse`, contract 1.4.3):
//! header `[Ship, Name]`, module lines `Module[, Charge][ /offline][ [N]]`, `Name xN` lines are drones / fighters /
//! cargo by category, implants and boosters by category 20, a T3D mode line sets `ship.mode_type_id`, and trailing
//! mutation blocks `[N] Base` / `  Mutaplasmid` / `  attr value, ...`. GPL-3.0-or-later.
use crate::data::Dataset;
use crate::eos::fit::infer_slot;
use crate::eos::stats::slot_name;
use crate::request::SlotReq;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

fn lookup(ds: &Dataset, name: &str) -> Option<u32> {
    ds.types.by_name_ci(name)
}

/// strip a trailing " [N]" mutation reference
fn mut_ref(line: &str) -> (&str, Option<u32>) {
    let l = line.trim_end();
    if l.ends_with(']') {
        if let Some(p) = l.rfind(" [") {
            if let Ok(n) = l[p + 2..l.len() - 1].parse::<u32>() {
                return (l[..p].trim_end(), Some(n));
            }
        }
    }
    (l, None)
}

struct Mut {
    base: u32,
    muta: Option<u32>,
    attrs: Vec<(String, f64)>,
}

impl Mut {
    fn json(&self) -> Value {
        let attrs: Map<String, Value> = self.attrs.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        json!({"base_type_id": self.base, "mutaplasmid_type_id": self.muta, "attributes": attrs})
    }
}

fn is_head(l: &str) -> bool {
    let t = l.trim();
    t.starts_with('[') && t.find(']').map(|e| t[1..e].parse::<u32>().is_ok()).unwrap_or(false)
}

fn parse_mutations(ds: &Dataset, lines: &[&str]) -> Result<(HashMap<u32, Mut>, usize), String> {
    let mut out = HashMap::new();
    let first = lines.iter().position(|l| is_head(l)).unwrap_or(lines.len());
    let mut i = first;
    while i < lines.len() {
        let t = lines[i].trim();
        if !is_head(t) {
            i += 1;
            continue;
        }
        let e = t.find(']').unwrap_or(0);
        let n: u32 = t[1..e].parse().unwrap_or(0);
        let base_name = t[e + 1..].trim();
        let base = lookup(ds, base_name).ok_or(format!("unknown mutated base '{base_name}'"))?;
        let mut m = Mut { base, muta: None, attrs: Vec::new() };
        i += 1;
        while i < lines.len() && !is_head(lines[i]) {
            let l = lines[i].trim();
            i += 1;
            if l.is_empty() {
                continue;
            }
            if m.muta.is_none() {
                m.muta = Some(lookup(ds, l).ok_or(format!("unknown mutaplasmid '{l}'"))?);
                continue;
            }
            for kv in l.split(',') {
                if let Some((k, v)) = kv.trim().rsplit_once(' ') {
                    let aid = ds.attr_id(k.trim());
                    if aid != 0 {
                        if let Ok(v) = v.trim().parse::<f64>() {
                            m.attrs.retain(|x| x.0 != aid.to_string());
                            m.attrs.push((aid.to_string(), v));
                        }
                    }
                }
            }
        }
        out.insert(n, m);
    }
    Ok((out, first))
}

/// resulting (mutated) type id for base + mutaplasmid
fn mutated_type(ds: &Dataset, m: &Mut) -> u32 {
    m.muta
        .and_then(|id| ds.mutaplasmids.get(&id))
        .and_then(|mu| mu.mapping.iter().find(|x| x.0.contains(&m.base)).map(|x| x.1))
        .unwrap_or(m.base)
}

pub fn parse(ds: &Dataset, text: &str) -> Result<Value, String> {
    let all: Vec<&str> = text.lines().collect();
    let (muts, first_mut_line) = parse_mutations(ds, &all)?;
    let mut lines = all[..first_mut_line].iter().map(|l| l.trim()).filter(|l| !l.is_empty());
    let header = lines.next().ok_or("empty EFT")?;
    let h = header.trim_start_matches('[').trim_end_matches(']');
    let ship_name = h.split(',').next().unwrap_or("").trim();
    let ship = lookup(ds, ship_name).ok_or(format!("unknown ship '{ship_name}'"))?;
    let mut mode: Option<u32> = None;
    let (mut modules, mut drones, mut fighters, mut implants, mut boosters, mut cargo) = (vec![], vec![], vec![], vec![], vec![], vec![]);
    for line in lines {
        if line.starts_with("[Empty") {
            continue;
        }
        let (line, offline) = match line.strip_suffix("/OFFLINE").or_else(|| line.strip_suffix("/offline")) {
            Some(l) => (l.trim(), true),
            None => (line, false),
        };
        let (line, mref) = mut_ref(line);
        let mutation = match mref {
            Some(n) => Some(muts.get(&n).ok_or(format!("mutation [{n}] not defined"))?),
            None => None,
        };
        let mjson = mutation.map(|m| m.json()).unwrap_or(Value::Null);
        // "Name xN" => drone / fighter / cargo
        if let Some(pos) = line.rfind(" x") {
            if let Ok(n) = line[pos + 2..].trim().parse::<u32>() {
                let name = line[..pos].trim();
                let Some(mut tid) = lookup(ds, name) else { return Err(format!("unknown item '{name}'")) };
                if let Some(m) = mutation {
                    tid = mutated_type(ds, m);
                }
                let cat = ds.types.get(&tid).map(|t| t.category).unwrap_or(0);
                match cat {
                    18 => drones.push(json!({"type_id": tid, "quantity": n, "active": n, "mutation": mjson})),
                    87 => fighters.push(json!({"type_id": tid, "quantity": n, "active": true, "abilities": null})),
                    _ => cargo.push(json!({"type_id": tid, "quantity": n})),
                }
                continue;
            }
        }
        let mut parts = line.splitn(2, ',');
        let name = parts.next().unwrap_or("").trim();
        let charge = parts.next().map(|s| s.trim());
        let Some(mut tid) = lookup(ds, name) else { return Err(format!("unknown item '{name}'")) };
        if let Some(m) = mutation {
            tid = mutated_type(ds, m);
        }
        let Some(t) = ds.types.get(&tid) else { return Err(format!("unknown item '{name}'")) };
        match t.category {
            20 => {
                // boosters carry the boosterness attribute (1087)
                if t.attr(1087).is_some() {
                    boosters.push(json!({"type_id": tid, "side_effects": []}));
                } else {
                    implants.push(json!(tid));
                }
            }
            18 => drones.push(json!({"type_id": tid, "quantity": 1, "active": 1, "mutation": mjson})),
            8 => cargo.push(json!({"type_id": tid, "quantity": 1})),
            _ => {
                if t.group == 1306 {
                    mode = Some(tid); // T3D tactical mode
                    continue;
                }
                let slot = infer_slot(t);
                let charge_type_id = match charge {
                    Some(c) => Some(lookup(ds, c).ok_or(format!("unknown charge '{c}'"))?),
                    None => None,
                };
                let active_capable = t.effects.iter().any(|(e, _)| ds.effects.get(e).map(|x| x.category == 1).unwrap_or(false))
                    || t.attr(6).map(|v| v != 0.0).unwrap_or(false);
                let state = if offline {
                    "offline"
                } else if active_capable && !matches!(slot, Some(SlotReq::Rig) | Some(SlotReq::Subsystem)) {
                    "active"
                } else {
                    "online"
                };
                modules.push(json!({"type_id": tid, "slot": slot.map(slot_name), "state": state, "charge_type_id": charge_type_id,
                                    "mutation": mjson, "spool": null}));
            }
        }
    }
    Ok(json!({
        "schema_version": 1,
        "ship": {"type_id": ship, "mode_type_id": mode},
        "character": {"security_status": null, "skills": {"default_level": null, "levels": {}}},
        "modules": modules, "drones": drones, "fighters": fighters, "implants": implants, "boosters": boosters, "cargo": cargo,
        "fleet": {"booster_fits": [], "buffs": []}, "projected": [],
        "environment": {"effect_type_ids": [], "system_security": null},
        "damage_pattern": null, "target_profile": null, "overrides": [],
        "options": {"cap_sim": {"max_time_s": null, "reload": false, "stagger": false}, "default_spool": null, "factor_reload": false,
                    "include_attributes": null, "nos_no_target_cap": false, "rah": null, "sources": false, "validate": true}
    }))
}
