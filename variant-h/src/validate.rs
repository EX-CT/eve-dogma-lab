//! Validation system: fitting violations (not errors).
use crate::calc::Calc;
use crate::components::Kind;
use crate::fit::Fit;
use crate::request::{Slot, State};
use rustc_hash::FxHashMap;
use serde_json::{json, Value};

pub fn validate(fit: &Fit, c: &Calc, cpu: f64, pg: f64, calib: f64, bw: f64) -> Vec<Value> {
    let ds = fit.ds;
    let v = fit.views();
    let a = &ds.a;
    let ship = fit.ship;
    let g = |attr: u32| c.get(ship, attr);
    let mut out = Vec::new();
    let mut push = |code: &str, msg: String, idx: Option<usize>| out.push(json!({"code": code, "message": msg, "module_index": idx}));
    if cpu > g(a.cpu_out) + 1e-9 {
        push("CPU_OVERLOAD", format!("CPU used {cpu:.2} > output {:.2}", g(a.cpu_out)), None);
    }
    if pg > g(a.power_out) + 1e-9 {
        push("POWER_OVERLOAD", format!("Powergrid used {pg:.2} > output {:.2}", g(a.power_out)), None);
    }
    if calib > g(a.upgrade_cap) + 1e-9 {
        push("CALIBRATION_OVERLOAD", format!("Calibration used {calib} > {}", g(a.upgrade_cap)), None);
    }
    if bw > g(a.drone_bw) + 1e-9 {
        push("DRONE_BANDWIDTH", format!("Drone bandwidth used {bw} > {}", g(a.drone_bw)), None);
    }
    let modules = &fit.modules;
    for (slot, attr) in [
        (Slot::High, a.hi_slots),
        (Slot::Mid, a.med_slots),
        (Slot::Low, a.low_slots),
        (Slot::Rig, a.rig_slots),
        (Slot::Subsystem, a.max_subsystems),
        (Slot::Service, a.service_slots),
    ] {
        let used = modules.iter().filter(|&&m| v.fitted(m).slot == Some(slot)).count() as f64;
        if used > g(attr) {
            push("SLOTS_EXCEEDED", format!("{slot:?} slots used {used} > {}", g(attr)), None);
        }
    }
    let t = modules.iter().filter(|&&m| v.has_effect(m, ds.e.turret)).count() as f64;
    if t > g(a.turret_slots) {
        push("TURRET_HARDPOINTS", format!("turrets {t} > hardpoints {}", g(a.turret_slots)), None);
    }
    let l = modules.iter().filter(|&&m| v.has_effect(m, ds.e.launcher)).count() as f64;
    if l > g(a.launcher_slots) {
        push("LAUNCHER_HARDPOINTS", format!("launchers {l} > hardpoints {}", g(a.launcher_slots)), None);
    }
    let ship_t = &ds.types[&v.item(ship).type_id];
    let mut fitted_group: FxHashMap<u32, u32> = Default::default();
    let mut fitted_type: FxHashMap<u32, u32> = Default::default();
    let mut active_group: FxHashMap<u32, u32> = Default::default();
    let mut online_group: FxHashMap<u32, u32> = Default::default();
    for &m in modules {
        let it = v.item(m);
        let f = v.fitted(m);
        let st = v.state(m);
        let idx = Some(f.req_index);
        let mt = &ds.types[&it.type_id];
        let name = &mt.name;
        if f.slot.is_none() {
            push("NOT_FITTABLE", format!("{name} is not a fittable module"), idx);
        }
        let gr: Vec<u32> = a.can_fit_group.iter().filter(|&&x| x != 0).filter_map(|x| mt.attr(*x)).map(|v| v as u32).filter(|v| *v != 0).collect();
        let ty: Vec<u32> = a.can_fit_type.iter().filter(|&&x| x != 0).filter_map(|x| mt.attr(*x)).map(|v| v as u32).filter(|v| *v != 0).collect();
        if (!gr.is_empty() || !ty.is_empty()) && !gr.contains(&ship_t.group) && !ty.contains(&ship_t.id) {
            push("SHIP_RESTRICTION", format!("{name} cannot be fitted to {}", ship_t.name), idx);
        }
        if f.slot == Some(Slot::Rig) {
            let rs = mt.attr(a.rig_size).unwrap_or(0.0);
            let srs = g(a.rig_size);
            if rs != 0.0 && rs != srs {
                push("RIG_SIZE", format!("{name} rig size {rs} != ship rig size {srs}"), idx);
            }
        }
        *fitted_group.entry(it.group).or_default() += 1;
        *fitted_type.entry(it.type_id).or_default() += 1;
        if st >= State::Online {
            *online_group.entry(it.group).or_default() += 1;
        }
        if st >= State::Active {
            *active_group.entry(it.group).or_default() += 1;
        }
        let check = |attr: u32, map: &FxHashMap<u32, u32>, key: u32| -> Option<(f64, u32)> {
            let lim = mt.attr(attr)?;
            let n = *map.get(&key).unwrap_or(&0);
            if lim > 0.0 && n as f64 > lim { Some((lim, n)) } else { None }
        };
        if let Some((lim, n)) = check(a.max_group_fitted, &fitted_group, it.group) {
            push("MAX_GROUP_FITTED", format!("{name}: {n} fitted of group, max {lim}"), idx);
        }
        if let Some((lim, n)) = check(a.max_type_fitted, &fitted_type, it.type_id) {
            push("MAX_TYPE_FITTED", format!("{name}: {n} fitted, max {lim}"), idx);
        }
        if let Some((lim, n)) = check(a.max_group_online, &online_group, it.group) {
            push("MAX_GROUP_ONLINE", format!("{name}: {n} online of group, max {lim}"), idx);
        }
        if let Some((lim, n)) = check(a.max_group_active, &active_group, it.group) {
            push("MAX_GROUP_ACTIVE", format!("{name}: {n} active of group, max {lim}"), idx);
        }
        if let Some(ch) = f.charge {
            let ct = &ds.types[&v.item(ch).type_id];
            let cg: Vec<u32> = a.charge_group.iter().filter_map(|x| mt.attr(*x)).map(|v| v as u32).filter(|v| *v != 0).collect();
            if !cg.contains(&ct.group) {
                push("CHARGE_GROUP", format!("{} cannot be loaded into {name}", ct.name), idx);
            }
            if let (Some(x), Some(y)) = (mt.attr(a.charge_size), ct.attr(a.charge_size)) {
                if x != y {
                    push("CHARGE_SIZE", format!("{} size {y} != launcher size {x}", ct.name), idx);
                }
            }
            if ct.volume > mt.capacity && mt.capacity > 0.0 {
                push("CHARGE_CAPACITY", format!("{} does not fit into {name}", ct.name), idx);
            }
        }
    }
    // skills
    let have: FxHashMap<u32, f64> = fit.skills.iter().map(|(_, s, l)| (*s, *l as f64)).collect();
    let mut missing: Vec<(u32, f64, u32)> = Vec::new();
    for &e in &fit.order {
        let it = v.item(e);
        if !matches!(it.kind, Kind::Ship | Kind::Module | Kind::Charge | Kind::Drone | Kind::Fighter | Kind::Implant | Kind::Booster) {
            continue;
        }
        let t = &ds.types[&it.type_id];
        for k in 0..6 {
            let s = t.attr(a.req_skill[k]).unwrap_or(0.0) as u32;
            if s == 0 {
                continue;
            }
            let need = t.attr(a.req_skill_level[k]).unwrap_or(1.0);
            if *have.get(&s).unwrap_or(&0.0) < need && !missing.iter().any(|m| m.0 == s && m.1 >= need) {
                missing.push((s, need, it.type_id));
            }
        }
    }
    for (s, need, by) in missing {
        push("MISSING_SKILL", format!("{} {} required by {}", ds.types.get(&s).map(|t| t.name.as_str()).unwrap_or("?"), need, ds.types[&by].name), None);
    }
    out
}
