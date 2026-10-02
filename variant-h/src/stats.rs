//! Output systems: read-only passes over the evaluated world producing FitStats (contract v1).
use crate::calc::Calc;
use crate::capsim::{self, Drain};
use crate::components::*;
use crate::fit::Fit;
use crate::views::Views;
use crate::request::{FitRequest, Resists, Slot, Spool, SpoolType, State};
use hecs::Entity;
use serde_json::{json, Map, Value};

pub fn range_factor(optimal: f64, falloff: f64, distance: Option<f64>, restricted: bool) -> f64 {
    let Some(d) = distance else { return 1.0 };
    if falloff > 0.0 {
        if restricted && d > optimal + 3.0 * falloff {
            return 0.0;
        }
        0.5f64.powf(((d - optimal).max(0.0) / falloff).powi(2))
    } else if d <= optimal {
        1.0
    } else {
        0.0
    }
}

pub fn lock_time(scan_res: f64, sig: f64) -> Option<f64> {
    if scan_res <= 0.0 || sig <= 0.0 {
        return None;
    }
    Some((40000.0 / scan_res / sig.asinh().powi(2)).min(1800.0))
}

fn float_unerr(v: f64) -> f64 {
    (v * 1e9).round() / 1e9
}

/// Spool-up (Pyfa calculateSpoolup semantics) -> bonus value
pub fn spoolup(max: f64, step: f64, cycle_s: f64, spool: Spool) -> f64 {
    if max == 0.0 || step == 0.0 {
        return 0.0;
    }
    let full = float_unerr(max / step).ceil();
    let cycles = match spool.kind {
        SpoolType::SpoolScale => float_unerr(max * spool.amount / step).ceil(),
        SpoolType::CycleScale => (spool.amount * full).round(),
        SpoolType::Time => float_unerr(spool.amount / cycle_s).floor().min(full),
        SpoolType::Cycles => spool.amount.floor().min(full),
    };
    (cycles * step).min(max)
}

#[derive(Default, Clone, Copy)]
struct Dmg([f64; 4]);
impl Dmg {
    fn total(&self) -> f64 {
        self.0.iter().sum()
    }
    fn scale(&self, k: f64) -> Dmg {
        Dmg(self.0.map(|x| x * k))
    }
    fn add(&mut self, o: &Dmg) {
        for k in 0..4 {
            self.0[k] += o.0[k];
        }
    }
    fn vs(&self, r: &Resists) -> f64 {
        self.0[0] * (1.0 - r.em) + self.0[1] * (1.0 - r.thermal) + self.0[2] * (1.0 - r.kinetic) + self.0[3] * (1.0 - r.explosive)
    }
    fn json(&self) -> Value {
        json!({"em": self.0[0], "thermal": self.0[1], "kinetic": self.0[2], "explosive": self.0[3], "total": self.total()})
    }
}

fn round6(v: f64) -> f64 {
    if v.is_finite() { (v * 1e6).round() / 1e6 } else { v }
}

/// round every float leaf to 6 decimals in place (non-finite -> null, like serde_json's f64 conversion)
fn tidy(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if n.is_f64() {
                if let Some(f) = n.as_f64() {
                    *v = json!(round6(f));
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(tidy),
        Value::Object(o) => o.values_mut().for_each(tidy),
        _ => {}
    }
}

struct Ctx<'f, 'a> {
    fit: &'f Fit<'a>,
    c: Calc<'f>,
    v: Views<'f>,
}

impl<'f, 'a> Ctx<'f, 'a> {
    #[inline]
    fn g(&self, e: Entity, attr: u32) -> f64 {
        self.c.get(e, attr)
    }
    fn type_name(&self, e: Entity) -> &str {
        self.v.type_name(e)
    }
    fn raw_cycle_ms(&self, e: Entity) -> f64 {
        let a = &self.fit.ds.a;
        let mut v: f64 = self.g(e, a.speed).max(self.g(e, a.duration));
        for &x in &a.extra_durations {
            if x != 0 {
                v = v.max(self.g(e, x));
            }
        }
        v
    }
    fn charge(&self, e: Entity) -> Option<Entity> {
        self.v.charge(e)
    }
    fn num_charges(&self, e: Entity) -> u32 {
        let Some(c) = self.charge(e) else { return 0 };
        let vol = self.g(c, self.fit.ds.a.volume);
        let cap = self.c.base(e, self.fit.ds.a.capacity);
        if vol <= 0.0 { 0 } else { float_unerr(cap / vol).floor() as u32 }
    }
    fn num_shots(&self, e: Entity) -> u32 {
        let a = &self.fit.ds.a;
        let Some(c) = self.charge(e) else { return 0 };
        let n = self.num_charges(e);
        if n > 0 && self.c.has(e, a.charge_rate) {
            let r = self.g(e, a.charge_rate);
            return if r > 0.0 { (n as f64 / r).floor() as u32 } else { 0 };
        }
        if n > 0 && self.c.has(c, a.crystals_get_damaged) {
            if self.g(c, a.crystals_get_damaged) == 1.0 {
                let hp = self.g(c, a.hp);
                let chance = self.g(c, a.crystal_vol_chance);
                let dmg = self.g(c, a.crystal_vol_damage);
                if dmg * chance > 0.0 {
                    return ((n as f64 * hp) / (dmg * chance)).floor() as u32;
                }
            }
            return 0;
        }
        0
    }
    fn avg_cycle_ms(&self, e: Entity, factor_reload: bool) -> f64 {
        self.avg_cycle_ms_with(e, factor_reload, self.g(e, self.fit.ds.a.reload))
    }
    fn avg_cycle_ms_with(&self, e: Entity, factor_reload: bool, reload: f64) -> f64 {
        let a = &self.fit.ds.a;
        let active = self.raw_cycle_ms(e);
        if active == 0.0 {
            return 0.0;
        }
        let inactive = self.g(e, a.reactivation);
        let shots = self.num_shots(e);
        if !factor_reload || shots == 0 || inactive >= reload {
            return active + inactive;
        }
        let early = shots as f64 - 1.0;
        ((active + inactive) * early + (active + reload)) / shots as f64
    }
    fn weapon_kind(&self, e: Entity) -> &'static str {
        let ef = &self.fit.ds.e;
        if self.v.has_effect(e, ef.turret) {
            "turret"
        } else if self.v.has_effect(e, ef.launcher) {
            "missile"
        } else if self.v.has_effect(e, ef.emp_wave) {
            "smartbomb"
        } else if self.v.has_effect(e, ef.vorton) {
            "vorton"
        } else {
            "other"
        }
    }
    fn module_volley(&self, e: Entity, kind: &str) -> Dmg {
        let a = &self.fit.ds.a;
        let ch = self.charge(e);
        let src = ch.unwrap_or(e);
        let mut mult = if self.c.has(e, a.dmg_mult) { self.g(e, a.dmg_mult) } else { 1.0 };
        if kind == "missile" && ch.is_some() {
            mult *= self.g(self.fit.char, a.missile_dmg_mult);
        }
        Dmg(a.dmg.map(|d| self.g(src, d) * mult))
    }
}

/// Missile max range (Pyfa semantics): flight time incl. ship radius, acceleration phase (mass*agility),
/// floor/ceil time blend, FoF range limit, measured from the ship's surface.
fn missile_range(x: &Ctx, ship: Entity, c: Entity) -> Option<f64> {
    let a = &x.fit.ds.a;
    let vel = x.g(c, a.max_velocity);
    if vel <= 0.0 {
        return None;
    }
    let radius = x.g(ship, a.radius);
    let ft = float_unerr(x.g(c, a.explosion_delay) / 1000.0 + radius / vel);
    let accel = x.g(c, a.mass) * x.g(c, a.agility) / 1e6;
    let range_at = |t: f64| {
        let acc = t.min(accel);
        vel / 2.0 * acc + vel * (t - acc)
    };
    let (lt, ht) = (ft.floor(), ft.ceil());
    let (mut lr, mut hr) = (range_at(lt), range_at(ht));
    if x.v.has_effect(c, x.fit.ds.e.fof_missile) {
        let lim = x.g(c, a.max_fof_range);
        if lim > 0.0 {
            lr = lr.min(lim);
            hr = hr.min(lim);
        }
    }
    lr = (lr - radius).max(0.0);
    hr = (hr - radius).max(0.0);
    let hc = ft - lt;
    Some(lr * (1.0 - hc) + hr * hc)
}

pub fn compute(fit: &Fit, req: &FitRequest) -> Value {
    let x = Ctx { fit, c: fit.calc(), v: fit.views() };
    let v = &x.v;
    let ds = fit.ds;
    let (a, ef) = (&ds.a, &ds.e);
    let ship = fit.ship;
    let ch = fit.char;
    let g = |e: Entity, attr: u32| x.g(e, attr);
    let factor_reload = req.options.factor_reload;
    let modules = &fit.modules;
    let state = |e: Entity| v.state(e);
    let slot = |e: Entity| v.fitted(e).slot;

    // ---------------- resources
    let cpu_used: f64 = modules.iter().filter(|&&m| state(m) >= State::Online).map(|&m| g(m, a.cpu)).sum();
    let pg_used: f64 = modules.iter().filter(|&&m| state(m) >= State::Online).map(|&m| g(m, a.power)).sum();
    let calib_used: f64 = modules.iter().filter(|&&m| slot(m) == Some(Slot::Rig)).map(|&m| g(m, a.upgrade_cost)).sum();
    let bw_used: f64 = fit.drones.iter().map(|&d| g(d, a.drone_bw_used) * v.squad(d).active as f64).sum();
    let bay_used: f64 = fit.drones.iter().map(|&d| g(d, a.volume) * v.squad(d).quantity as f64).sum();
    let fbay_used: f64 = fit.fighters.iter().map(|&f| g(f, a.volume) * v.squad(f).quantity as f64).sum();
    let cargo_used: f64 = req.cargo.iter().map(|c| ds.types.get(&c.type_id).map(|t| t.volume).unwrap_or(0.0) * c.quantity as f64).sum();
    let count_slot = |s: Slot| modules.iter().filter(|&&m| slot(m) == Some(s)).count() as f64;
    let turrets_used = modules.iter().filter(|&&m| v.has_effect(m, ef.turret)).count() as f64;
    let launchers_used = modules.iter().filter(|&&m| v.has_effect(m, ef.launcher)).count() as f64;
    let usage = |u: f64, t: f64| json!({"used": u, "total": t});
    let fighter_class = |f: Entity| -> &'static str {
        if g(f, a.fighter_heavy) > 0.0 {
            "heavy"
        } else if g(f, a.fighter_support) > 0.0 {
            "support"
        } else {
            "light"
        }
    };
    let launched: Vec<Entity> = fit.fighters.iter().copied().filter(|&f| v.squad(f).active > 0).collect();
    let class_used = |c: &str| launched.iter().filter(|&&f| fighter_class(f) == c).count() as f64;
    let resources = json!({
        "cpu": usage(cpu_used, g(ship, a.cpu_out)),
        "power": usage(pg_used, g(ship, a.power_out)),
        "calibration": usage(calib_used, g(ship, a.upgrade_cap)),
        "drone_bandwidth": usage(bw_used, g(ship, a.drone_bw)),
        "drone_bay": usage(bay_used, g(ship, a.drone_capacity)),
        "fighter_bay": usage(fbay_used, g(ship, a.fighter_capacity)),
        "cargo": usage(cargo_used, g(ship, a.capacity)),
        "slots": {
            "high": usage(count_slot(Slot::High), g(ship, a.hi_slots)),
            "mid": usage(count_slot(Slot::Mid), g(ship, a.med_slots)),
            "low": usage(count_slot(Slot::Low), g(ship, a.low_slots)),
            "rig": usage(count_slot(Slot::Rig), g(ship, a.rig_slots)),
            "subsystem": usage(count_slot(Slot::Subsystem), g(ship, a.max_subsystems)),
            "service": usage(count_slot(Slot::Service), g(ship, a.service_slots)),
        },
        "hardpoints": {
            "turret": usage(turrets_used, g(ship, a.turret_slots)),
            "launcher": usage(launchers_used, g(ship, a.launcher_slots)),
        },
        "fighter_tubes": {
            "total": usage(launched.len() as f64, g(ship, a.fighter_tubes)),
            "light": usage(class_used("light"), g(ship, a.fighter_light_slots)),
            "support": usage(class_used("support"), g(ship, a.fighter_support_slots)),
            "heavy": usage(class_used("heavy"), g(ship, a.fighter_heavy_slots)),
        },
    });

    // ---------------- offense
    let tp = req.target_profile.clone().unwrap_or_default();
    let tp_res = Resists { em: tp.em, thermal: tp.thermal, kinetic: tp.kinetic, explosive: tp.explosive };
    let default_spool = req.options.default_spool.unwrap_or(Spool { kind: SpoolType::SpoolScale, amount: 1.0 });
    let mut weapons = Vec::new();
    let (mut w_vol, mut w_dps) = (Dmg::default(), Dmg::default());
    for &m in modules {
        if state(m) < State::Active {
            continue;
        }
        let kind = x.weapon_kind(m);
        let base = x.module_volley(m, kind);
        if base.total() == 0.0 {
            continue;
        }
        let cyc = x.avg_cycle_ms(m, factor_reload);
        let raw = x.raw_cycle_ms(m);
        let f = v.fitted(m);
        let sp = spoolup(g(m, a.spool_max), g(m, a.spool_step), raw / 1000.0, f.spool.unwrap_or(default_spool));
        let vol = base.scale(1.0 + sp);
        let dps = if cyc > 0.0 { vol.scale(1000.0 / cyc) } else { Dmg::default() };
        w_vol.add(&vol);
        w_dps.add(&dps);
        let mut w = json!({
            "module_index": f.req_index, "type_id": v.item(m).type_id, "name": x.type_name(m), "kind": kind,
            "charge_type_id": f.charge.map(|c| v.item(c).type_id),
            "volley": vol.json(), "dps": dps.json(), "cycle_time_ms": cyc,
        });
        match kind {
            "turret" => {
                w["optimal_m"] = json!(g(m, a.max_range));
                w["falloff_m"] = json!(g(m, a.falloff));
                w["tracking"] = json!(g(m, a.tracking));
            }
            "missile" => {
                if let Some(c) = f.charge {
                    if let Some(r) = missile_range(&x, ship, c) {
                        w["range_m"] = json!(r);
                    }
                    w["explosion_radius"] = json!(g(c, a.aoe_cloud));
                    w["explosion_velocity"] = json!(g(c, a.aoe_velocity));
                }
            }
            "smartbomb" => w["range_m"] = json!(g(m, a.emp_range)),
            _ => {}
        }
        if sp > 0.0 {
            w["spool_multiplier"] = json!(1.0 + sp);
            w["volley_unspooled"] = base.json();
        }
        weapons.push(w);
    }
    let (mut d_vol, mut d_dps) = (Dmg::default(), Dmg::default());
    let mut drone_out = Vec::new();
    for &d in &fit.drones {
        let sq = v.squad(d);
        let n = sq.active as f64;
        if n == 0.0 {
            continue;
        }
        let mult = if x.c.has(d, a.dmg_mult) { g(d, a.dmg_mult) } else { 1.0 };
        let v = Dmg(a.dmg.map(|k| g(d, k))).scale(mult * n);
        let cyc = x.raw_cycle_ms(d);
        if v.total() == 0.0 || cyc == 0.0 {
            continue;
        }
        let dps = v.scale(1000.0 / cyc);
        d_vol.add(&v);
        d_dps.add(&dps);
        drone_out.push(json!({"drone_index": sq.req_index, "type_id": x.v.item(d).type_id, "name": x.type_name(d), "count": n, "volley": v.json(), "dps": dps.json(),
            "optimal_m": g(d, a.max_range), "falloff_m": g(d, a.falloff), "tracking": g(d, a.tracking),
            "max_velocity": g(d, a.max_velocity), "signature_radius": g(d, a.sig)}));
    }
    let (mut f_vol, mut f_dps) = (Dmg::default(), Dmg::default());
    let mut fighter_out = Vec::new();
    for &f in &fit.fighters {
        let sq = v.squad(f);
        let n = sq.active as f64;
        if n == 0.0 {
            continue;
        }
        let abilities = fit.world.get::<&FighterAbilities>(f).map(|x| x.0.clone()).unwrap_or_default();
        let (mut fv, mut fd) = (Dmg::default(), Dmg::default());
        for (eid, at) in [(ef.f_attack, &a.fam), (ef.f_missiles, &a.fmi)] {
            if !v.has_effect(f, eid) || !abilities.contains(&eid) {
                continue;
            }
            let m = g(f, at[0]);
            let m = if m == 0.0 { 1.0 } else { m };
            let v = Dmg([g(f, at[1]), g(f, at[2]), g(f, at[3]), g(f, at[4])]).scale(m * n);
            let dur = g(f, at[5]);
            fv.add(&v);
            if dur > 0.0 {
                fd.add(&v.scale(1000.0 / dur));
            }
        }
        if fv.total() > 0.0 {
            f_vol.add(&fv);
            f_dps.add(&fd);
            fighter_out.push(json!({"fighter_index": sq.req_index, "type_id": v.item(f).type_id, "name": x.type_name(f), "squadron_size": n, "volley": fv.json(), "dps": fd.json(),
                "max_velocity": g(f, a.max_velocity), "signature_radius": g(f, a.sig)}));
        }
    }
    let mut t_vol = w_vol;
    t_vol.add(&d_vol);
    t_vol.add(&f_vol);
    let mut t_dps = w_dps;
    t_dps.add(&d_dps);
    t_dps.add(&f_dps);
    let offense = json!({
        "weapons": weapons, "drones": drone_out, "fighters": fighter_out,
        "total": {"weapon_dps": w_dps.total(), "weapon_volley": w_vol.total(), "drone_dps": d_dps.total(), "drone_volley": d_vol.total(),
                  "fighter_dps": f_dps.total(), "fighter_volley": f_vol.total(), "dps": t_dps.json(), "volley": t_vol.json()},
        "vs_target_profile": {"dps": t_dps.vs(&tp_res), "volley": t_vol.vs(&tp_res)},
    });

    // ---------------- defense
    let dp = req.damage_pattern.unwrap_or(Resists { em: 25.0, thermal: 25.0, kinetic: 25.0, explosive: 25.0 });
    let dp_tot = (dp.em + dp.thermal + dp.kinetic + dp.explosive).max(1e-12);
    let layer = |ids: &[u32; 4]| ids.map(|i| g(ship, i));
    let effectivify = |amount: f64, r: [f64; 4]| {
        let div = (dp.em * r[0] + dp.thermal * r[1] + dp.kinetic * r[2] + dp.explosive * r[3]) / dp_tot;
        if div == 0.0 { amount } else { amount / div }
    };
    let (rs, ra, rh) = (layer(&a.res_shield), layer(&a.res_armor), layer(&a.res_hull));
    let hp_s = g(ship, a.shield_capacity);
    let hp_a = g(ship, a.armor_hp);
    let hp_h = g(ship, a.hp);
    let (e_s, e_a, e_h) = (effectivify(hp_s, rs), effectivify(hp_a, ra), effectivify(hp_h, rh));
    let res_json = |r: [f64; 4]| json!({"em": r[0], "thermal": r[1], "kinetic": r[2], "explosive": r[3]});
    let (mut shield_rep, mut armor_rep, mut hull_rep) = (0.0, 0.0, 0.0);
    for &m in modules {
        if state(m) < State::Active {
            continue;
        }
        let dur = g(m, a.duration) / 1000.0;
        if dur <= 0.0 {
            continue;
        }
        if v.has_effect(m, ef.shield_boost) || v.has_effect(m, ef.fueled_shield_boost) {
            shield_rep += g(m, a.shield_bonus) / dur;
        }
        if v.has_effect(m, ef.armor_repair) {
            armor_rep += g(m, a.armor_dmg_amount) / dur;
        }
        if v.has_effect(m, ef.fueled_armor_repair) {
            let paste = x.charge(m).map(|c| x.type_name(c) == "Nanite Repair Paste").unwrap_or(false);
            armor_rep += g(m, a.armor_dmg_amount) * if paste { 3.0 } else { 1.0 } / dur;
        }
        if v.has_effect(m, ef.structure_repair) {
            hull_rep += g(m, a.structure_dmg_amount) / dur;
        }
    }
    // incoming remote repairs (Pyfa applied-RR diminishing-returns formula)
    {
        let mut lists: [Vec<(f64, f64)>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for &e in &fit.order {
            if let Ok(r) = fit.world.get::<&IncomingRep>(e) {
                let dur = g(e, a.duration) / 1000.0;
                if dur > 0.0 {
                    lists[r.layer as usize].push((g(e, r.amount_attr) * r.mult * r.factor, dur));
                }
            }
        }
        let applied = |l: &Vec<(f64, f64)>| -> f64 {
            let total: f64 = l.iter().map(|(x, c)| x / c.trunc()).sum();
            l.iter()
                .map(|(x, c)| {
                    let rrps = x / c.trunc();
                    let m = 7000.0 + rrps * 20.0;
                    (1.0 - (((rrps + m) / (total + m)) - 1.0).powi(2)) * x / c
                })
                .sum()
        };
        shield_rep += applied(&lists[0]);
        armor_rep += applied(&lists[1]);
        hull_rep += applied(&lists[2]);
    }
    let srr = g(ship, a.shield_recharge) / 1000.0;
    let passive = if srr > 0.0 { 10.0 / srr * 0.5 * 0.5 * hp_s } else { 0.0 };
    let mut defense = json!({
        "hp": {"shield": hp_s, "armor": hp_a, "hull": hp_h, "total": hp_s + hp_a + hp_h},
        "resonance": {"shield": res_json(rs), "armor": res_json(ra), "hull": res_json(rh)},
        "ehp": {"shield": e_s, "armor": e_a, "hull": e_h, "total": e_s + e_a + e_h},
        "damage_pattern": {"em": dp.em, "thermal": dp.thermal, "kinetic": dp.kinetic, "explosive": dp.explosive},
        "tank": {
            "raw": {"passive_shield": passive, "shield_repair": shield_rep, "armor_repair": armor_rep, "hull_repair": hull_rep},
            "effective": {"passive_shield": effectivify(passive, rs), "shield_repair": effectivify(shield_rep, rs),
                          "armor_repair": effectivify(armor_rep, ra), "hull_repair": effectivify(hull_rep, rh)},
        },
    });

    // ---------------- capacitor
    let cap = g(ship, a.cap_capacity);
    let rr = g(ship, a.recharge_rate);
    let peak = if rr > 0.0 { 10.0 / (rr / 1000.0) * 0.5 * 0.5 * cap } else { 0.0 };
    let mut drains = Vec::new();
    let (mut cap_used, mut cap_added) = (0.0, 0.0);
    let mut module_rows = Vec::new();
    let mut cap_use_of: Vec<(Entity, f64)> = Vec::new();
    for &m in modules {
        let f = v.fitted(m);
        let mut cap_need = g(m, a.cap_need);
        let is_inj = ds.group_names.get(&v.item(m).group).map(|n| n == "Capacitor Booster").unwrap_or(false);
        if is_inj {
            cap_need = -f.charge.map(|c| g(c, a.capacitor_bonus)).unwrap_or(0.0);
        }
        if v.has_effect(m, ef.nos) && !req.options.nos_no_target_cap {
            cap_need = -g(m, a.power_transfer);
        }
        let cyc_raw = x.raw_cycle_ms(m);
        let full = cyc_raw + g(m, a.reactivation);
        let mut row = json!({"module_index": f.req_index, "type_id": v.item(m).type_id, "name": x.type_name(m),
            "slot": f.slot, "state": state(m), "cpu": g(m, a.cpu), "power": g(m, a.power)});
        if cyc_raw > 0.0 {
            row["cycle_time_ms"] = json!(cyc_raw);
        }
        if state(m) >= State::Active && cap_need != 0.0 && full > 0.0 {
            // capacitor boosters always have their 10 s reload factored into the rate (Pyfa forces it)
            let avg = if is_inj { x.avg_cycle_ms_with(m, true, 10_000.0) } else { x.avg_cycle_ms(m, factor_reload) };
            let use_ = if avg > 0.0 { cap_need / (avg / 1000.0) } else { 0.0 };
            if use_ > 0.0 {
                cap_used += use_
            } else {
                cap_added -= use_
            }
            row["cap_use_gj_s"] = json!(use_);
            cap_use_of.push((m, use_));
            drains.push(Drain {
                duration: full.trunc(),
                cap_need,
                clip_size: x.num_shots(m),
                reload_ms: g(m, a.reload),
                is_injector: is_inj,
                disable_stagger: v.has_effect(m, ef.turret),
            });
        }
        module_rows.push(row);
    }
    // incoming neuts / nos / cap transfers (extra simulation drains after the fit's own modules)
    let sig_now = g(ship, a.sig);
    for &e in &fit.order {
        if let Ok(d) = fit.world.get::<&IncomingDrain>(e) {
            let mut need = g(e, d.amount_attr) * d.factor * d.sign;
            if d.resist != 0 {
                need *= g(ship, d.resist);
            }
            let sres = g(e, a.neut_sig_res);
            if sres != 0.0 {
                need *= (sig_now / sres).min(1.0);
            }
            let dur = g(e, d.duration_attr);
            if need != 0.0 && dur > 0.0 {
                // like Pyfa's capUsed / capRecharge, incoming drains and transfers count in the rates
                let rate = need / (dur.trunc() / 1000.0);
                if rate > 0.0 {
                    cap_used += rate
                } else {
                    cap_added -= rate
                }
                drains.push(Drain { duration: dur.trunc(), cap_need: need, clip_size: 0, reload_ms: 0.0, is_injector: false, disable_stagger: false });
            }
        }
    }
    let mut capj = json!({"capacity": cap, "recharge_time_s": rr / 1000.0, "peak_recharge_gj_s": peak,
        "use_gj_s": cap_used, "injected_gj_s": cap_added, "delta_gj_s": peak + cap_added - cap_used});
    if drains.is_empty() {
        capj["stable"] = json!(true);
        capj["stable_percent"] = json!(100.0);
    } else {
        let o = &req.options.cap_sim;
        let r = capsim::simulate(cap, rr, &drains, o.reload || factor_reload, true, o.max_time_s.unwrap_or(6.0 * 3600.0) * 1000.0);
        let st = (r.stable_low + r.stable_high) / 2.0;
        let stable = r.stable && st > 0.0;
        capj["stable"] = json!(stable);
        if stable {
            capj["stable_percent"] = json!((st * 100.0).min(100.0));
        } else {
            capj["depletes_in_s"] = json!(r.t_s);
        }
        capj["eve_stable_percent"] = json!(r.eve_stable * 100.0);
        capj["sim_iterations"] = json!(r.iterations);
    }

    // ---------------- sustainable tank: when the capacitor is not stable (or reload is factored), cap-using local
    // repairers only run to the extent the peak recharge (plus injection) can pay for them, best rep/GJ first.
    {
        let cap_stable = capj["stable"].as_bool().unwrap_or(true);
        let mut sus = [shield_rep, armor_rep, hull_rep];
        if !cap_stable || factor_reload {
            let gname = |m: Entity| ds.group_names.get(&v.item(m).group).map(|n| n.as_str()).unwrap_or("");
            let layer_attr = |g: &str| match g {
                "Shield Booster" | "Ancillary Shield Booster" => Some((0usize, a.shield_bonus)),
                "Armor Repair Unit" | "Ancillary Armor Repairer" => Some((1, a.armor_dmg_amount)),
                "Hull Repair Unit" => Some((2, a.structure_dmg_amount)),
                _ => None,
            };
            let paste_mult = |m: Entity| {
                let paste = x.charge(m).map(|c| x.type_name(c) == "Nanite Repair Paste").unwrap_or(false);
                let k = g(m, a.charged_armor_mult);
                if paste && k != 0.0 { k } else { 1.0 }
            };
            let mut adj = [0.0f64; 3];
            let mut used = cap_used;
            // (module, layer, amount attr, cap/s, efficiency)
            let mut reps: Vec<(Entity, usize, u32, f64, f64)> = Vec::new();
            for l in 0..3 {
                for &m in modules {
                    if state(m) < State::Active {
                        continue;
                    }
                    let gn = gname(m);
                    let Some((ml, attr)) = layer_attr(gn) else { continue };
                    if ml != l {
                        continue;
                    }
                    let cyc = x.raw_cycle_ms(m) / 1000.0;
                    if cyc <= 0.0 {
                        continue;
                    }
                    let amount = g(m, attr);
                    let use_ = cap_use_of.iter().find(|(e, _)| *e == m).map(|p| p.1).unwrap_or(0.0);
                    if use_ != 0.0 {
                        used -= use_;
                        adj[l] -= amount * paste_mult(m) / cyc;
                        let k = g(m, a.charged_armor_mult);
                        let eff = amount * if k != 0.0 { k } else { 1.0 } / g(m, a.cap_need);
                        reps.push((m, l, attr, use_, eff));
                    } else if gn == "Ancillary Shield Booster" {
                        let reload = if factor_reload && x.charge(m).is_some() { g(m, a.reload) } else { 0.0 };
                        let shots = x.num_shots(m).max(1) as f64;
                        let off = reload / (shots * cyc * 1000.0 + reload);
                        adj[l] -= amount * off / cyc;
                    }
                }
            }
            reps.sort_by(|p, q| q.4.partial_cmp(&p.4).unwrap_or(std::cmp::Ordering::Equal));
            let budget = peak + cap_added;
            for (m, l, attr, use_, _) in reps {
                if used > budget {
                    break;
                }
                let cyc = x.raw_cycle_ms(m) / 1000.0;
                let frac = ((budget - used) / use_).min(1.0);
                let amount = g(m, attr);
                if x.charge(m).is_none() {
                    adj[l] += frac * amount / cyc;
                } else {
                    let reload = if factor_reload { g(m, a.reload) } else { 0.0 };
                    let active_ms = x.num_shots(m).max(1) as f64 * cyc * 1000.0;
                    adj[l] += frac * amount * (active_ms / (active_ms + reload)) * paste_mult(m) / cyc;
                }
                used += use_;
            }
            for l in 0..3 {
                sus[l] += adj[l];
            }
        }
        defense["tank"]["sustained"] = json!({"passive_shield": passive, "shield_repair": sus[0], "armor_repair": sus[1], "hull_repair": sus[2]});
        defense["tank"]["sustained_effective"] = json!({"passive_shield": effectivify(passive, rs), "shield_repair": effectivify(sus[0], rs),
            "armor_repair": effectivify(sus[1], ra), "hull_repair": effectivify(sus[2], rh)});
    }

    // ---------------- navigation
    let maxv = g(ship, a.max_velocity);
    let limit = g(ship, a.speed_limit);
    let max_speed = if limit > 0.0 && maxv > limit { limit } else { maxv };
    let mass = g(ship, a.mass);
    let agility = g(ship, a.agility);
    let nz = |v: f64| if v == 0.0 { 1.0 } else { v };
    let warp_need = g(ship, a.warp_cap_need);
    let sig = g(ship, a.sig);
    let navigation = json!({
        "max_velocity": max_speed, "align_time_s": -(0.25f64.ln()) * agility * mass / 1e6, "mass": mass, "agility": agility,
        "signature_radius": sig, "warp_speed_au_s": nz(g(ship, a.base_warp)) * nz(g(ship, a.warp_mult)),
        "max_warp_distance_au": if warp_need > 0.0 && mass > 0.0 { cap / (mass * warp_need) } else { 0.0 },
        "warp_scramble_status": g(ship, a.warp_scramble),
    });

    // ---------------- targeting
    let mut best = ("none", 0.0f64);
    for (n, at) in ["radar", "ladar", "magnetometric", "gravimetric"].into_iter().zip(a.sensor) {
        let v = g(ship, at);
        if v > best.1 {
            best = (n, v);
        }
    }
    // ECM jam chance (Pyfa): jam strength per jammer is read for the target's strongest sensor type (a tie makes it
    // "Multispectral", which no jammer has); chance = 1 - prod(1 - min(1, strength / sensor strength))
    let jam_chance = {
        let mut st = ("", -1.0f64);
        for (n, k) in [("Magnetometric", 2usize), ("Ladar", 1), ("Radar", 0), ("Gravimetric", 3)] {
            let x = g(ship, a.sensor[k]);
            if x > st.1 {
                st = (n, x);
            } else if x == st.1 {
                st = ("Multispectral", x);
            }
        }
        let sensors = best.1;
        let mut keep = 1.0f64;
        for &e in &fit.order {
            if let Ok(j) = fit.world.get::<&IncomingEcm>(e) {
                let an = if j.fighter { format!("fighterAbilityECMStrength{}", st.0) } else { format!("scan{}StrengthBonus", st.0) };
                let id = ds.attr_id(&an);
                let mut strength = if id != 0 && x.c.has(j.src, id) { g(j.src, id) } else { 0.0 } * j.factor;
                if j.resist != 0 {
                    strength *= g(ship, j.resist);
                }
                if sensors > 0.0 {
                    keep *= 1.0 - (strength / sensors).min(1.0);
                }
            }
        }
        (1.0 - keep) * 100.0
    };
    let scan_res = g(ship, a.scan_resolution);
    let lt = |s: f64| lock_time(scan_res, s);
    let targeting = json!({
        "max_targets": g(ship, a.max_locked).min(g(ch, a.max_locked).max(0.0)),
        "max_range_m": g(ship, a.max_target_range), "scan_resolution": scan_res,
        "sensor_strength": best.1, "sensor_type": best.0, "jam_chance_percent": jam_chance,
        "probe_size": if best.1 > 0.0 { Some((sig / best.1).max(1.08)) } else { None },
        "lock_time_s": {"sig_25m": lt(25.0), "sig_40m": lt(40.0), "sig_125m": lt(125.0), "sig_400m": lt(400.0), "sig_target_profile": tp.signature_radius.and_then(lt)},
    });
    let drones_j = json!({
        "active": fit.drones.iter().map(|&d| v.squad(d).active).sum::<u32>(),
        "max_active": g(ch, a.max_active_drones),
        "control_range_m": g(ch, a.drone_control),
    });

    let mut out = Map::new();
    out.insert("meta".into(), json!({"schema_version": 1, "engine": concat!("eve-dogma-h ", env!("CARGO_PKG_VERSION"), " (Rust ECS/hecs)"),
        "sde_build": ds.build, "dataset_sha256": ds.sha256}));
    let st = &ds.types[&v.item(ship).type_id];
    out.insert("ship".into(), json!({"type_id": st.id, "name": st.name, "group": ds.group_names.get(&st.group)}));
    out.insert("resources".into(), resources);
    out.insert("offense".into(), offense);
    out.insert("defense".into(), defense);
    out.insert("capacitor".into(), capj);
    out.insert("navigation".into(), navigation);
    out.insert("targeting".into(), targeting);
    out.insert("drones".into(), drones_j);
    out.insert("modules".into(), Value::Array(module_rows));
    if req.options.validate {
        out.insert("violations".into(), Value::Array(crate::validate::validate(fit, &x.c, cpu_used, pg_used, calib_used, bw_used)));
    }
    if !fit.warnings.is_empty() {
        out.insert("warnings".into(), json!(fit.warnings));
    }
    if let Some(sel) = req.options.include_attributes.as_deref() {
        out.insert("attributes".into(), dump_attributes(fit, &x.c, sel));
    }
    let mut out = Value::Object(out);
    tidy(&mut out);
    out
}

fn dump(fit: &Fit, c: &Calc, e: Entity, filter: &Option<Vec<&str>>) -> Value {
    let mut m = Map::new();
    for k in c.attr_ids(e) {
        let name = fit.ds.attrs.get(&k).map(|a| a.name.clone()).unwrap_or_else(|| k.to_string());
        if let Some(f) = filter {
            if !f.contains(&name.as_str()) {
                continue;
            }
        }
        m.insert(name, json!(c.get(e, k)));
    }
    Value::Object(m)
}

/// `include_attributes`: "all", "ship", or a comma list of attribute names (dumped for every item kind).
fn dump_attributes(fit: &Fit, c: &Calc, sel: &str) -> Value {
    let filter: Option<Vec<&str>> = match sel {
        "all" | "ship" => None,
        s => Some(s.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect()),
    };
    let v = fit.views();
    let mut m = Map::new();
    m.insert("ship".into(), dump(fit, c, fit.ship, &filter));
    if sel == "ship" {
        return Value::Object(m);
    }
    m.insert("character".into(), dump(fit, c, fit.char, &filter));
    let mods: Vec<Value> = fit
        .modules
        .iter()
        .map(|&e| {
            let f = v.fitted(e);
            json!({"module_index": f.req_index, "type_id": v.item(e).type_id, "attributes": dump(fit, c, e, &filter),
                   "charge": f.charge.map(|ch| dump(fit, c, ch, &filter))})
        })
        .collect();
    m.insert("modules".into(), Value::Array(mods));
    let dr: Vec<Value> = fit.drones.iter().map(|&e| json!({"drone_index": v.squad(e).req_index, "attributes": dump(fit, c, e, &filter)})).collect();
    m.insert("drones".into(), Value::Array(dr));
    Value::Object(m)
}
