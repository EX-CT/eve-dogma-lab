//! Fit statistics on top of the evaluated dogma graph (Pyfa-equivalent formulas).
use crate::capsim::{self, Drain};
use crate::engine::{Fit, Kind};
use crate::request::{FitRequest, Resists, Slot, Spool, SpoolType, State};
use crate::jv as json;
use crate::out::{Obj as Map, J as Value};

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

/// Pyfa eos/utils/spoolSupport.calculateSpoolup -> (value, cycles, time)
pub fn spoolup(max: f64, step: f64, cycle_s: f64, spool: Spool) -> (f64, f64, f64) {
    if max == 0.0 || step == 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let cycles = match spool.kind {
        SpoolType::SpoolScale => float_unerr(max * spool.amount / step).ceil(),
        SpoolType::CycleScale => (spool.amount * float_unerr(max / step).ceil()).round(),
        SpoolType::Time => float_unerr(spool.amount / cycle_s).floor().min(float_unerr(max / step).ceil()),
        SpoolType::Cycles => spool.amount.floor().min(float_unerr(max / step).ceil()),
    };
    let v = (cycles * step).min(max);
    (v, cycles, cycles * cycle_s)
}

#[derive(Default, Clone, Copy)]
struct Dmg {
    em: f64,
    th: f64,
    ki: f64,
    ex: f64,
}
impl Dmg {
    fn total(&self) -> f64 {
        self.em + self.th + self.ki + self.ex
    }
    fn scale(&self, k: f64) -> Dmg {
        Dmg { em: self.em * k, th: self.th * k, ki: self.ki * k, ex: self.ex * k }
    }
    fn add(&mut self, o: &Dmg) {
        self.em += o.em;
        self.th += o.th;
        self.ki += o.ki;
        self.ex += o.ex;
    }
    fn vs(&self, r: &Resists) -> f64 {
        self.em * (1.0 - r.em) + self.th * (1.0 - r.thermal) + self.ki * (1.0 - r.kinetic) + self.ex * (1.0 - r.explosive)
    }
    fn json(&self) -> Value {
        json!({"em": self.em, "thermal": self.th, "kinetic": self.ki, "explosive": self.ex, "total": self.total()})
    }
}

struct Ids {
    cpu: u32,
    power: u32,
    cpu_out: u32,
    power_out: u32,
    upgrade_cost: u32,
    upgrade_cap: u32,
    speed: u32,
    duration: u32,
    cap_need: u32,
    reload: u32,
    reactivation: u32,
    charge_rate: u32,
    dmg_mult: u32,
    dmg: [u32; 4],
    /// extra duration attributes of raw_cycle_ms that exist in the dataset (in check order)
    dur_extra: Vec<u32>,
    /// per item: bit k set when the item has an effect named in FX[k] (resolved once per stats call)
    fx: Vec<u16>,
}

/// Effects the stats code tests by name; `Ids::fx` holds one bit per entry.
const FX: [&[&str]; 11] = [
    &["turretFitted"],
    &["launcherFitted"],
    &["empWave"],
    &["ChainLightning"],
    &["doomsdaySlash"],
    &["fofMissileLaunching"],
    &["shieldBoosting", "fueledShieldBoosting"],
    &["armorRepair"],
    &["fueledArmorRepair"],
    &["structureRepair"],
    &["energyNosferatuFalloff"],
];
const FX_TURRET: u16 = 1 << 0;
const FX_LAUNCHER: u16 = 1 << 1;
const FX_EMP_WAVE: u16 = 1 << 2;
const FX_CHAIN_LIGHTNING: u16 = 1 << 3;
const FX_DOOMSDAY_SLASH: u16 = 1 << 4;
const FX_FOF: u16 = 1 << 5;
const FX_SHIELD_BOOST: u16 = 1 << 6;
const FX_ARMOR_REP: u16 = 1 << 7;
const FX_FUELED_ARMOR_REP: u16 = 1 << 8;
const FX_HULL_REP: u16 = 1 << 9;
const FX_NOS: u16 = 1 << 10;

fn fx_flags(f: &Fit) -> Vec<u16> {
    // (effect id, bit); effect names are unique among these
    let ids: Vec<(u32, u16)> = FX
        .iter()
        .enumerate()
        .flat_map(|(k, names)| names.iter().map(move |n| (k, *n)))
        .map(|(k, n)| (f.ds.effect_id(n), 1u16 << k))
        .filter(|&(e, _)| e != 0)
        .collect();
    f.items
        .iter()
        .map(|it| it.effects.iter().fold(0u16, |m, (e, _)| ids.iter().fold(m, |m, &(x, b)| if x == *e { m | b } else { m })))
        .collect()
}

fn ids(f: &Fit) -> Ids {
    let a = |n: &str| f.ds.attr_id(n);
    Ids {
        cpu: crate::attr_id!(f.ds, "cpu"),
        power: crate::attr_id!(f.ds, "power"),
        cpu_out: crate::attr_id!(f.ds, "cpuOutput"),
        power_out: crate::attr_id!(f.ds, "powerOutput"),
        upgrade_cost: crate::attr_id!(f.ds, "upgradeCost"),
        upgrade_cap: crate::attr_id!(f.ds, "upgradeCapacity"),
        speed: crate::attr_id!(f.ds, "speed"),
        duration: crate::attr_id!(f.ds, "duration"),
        cap_need: crate::attr_id!(f.ds, "capacitorNeed"),
        reload: crate::attr_id!(f.ds, "reloadTime"),
        reactivation: crate::attr_id!(f.ds, "moduleReactivationDelay"),
        charge_rate: crate::attr_id!(f.ds, "chargeRate"),
        dmg_mult: crate::attr_id!(f.ds, "damageMultiplier"),
        dmg: [crate::attr_id!(f.ds, "emDamage"), crate::attr_id!(f.ds, "thermalDamage"), crate::attr_id!(f.ds, "kineticDamage"), crate::attr_id!(f.ds, "explosiveDamage")],
        dur_extra: [
            "durationHighisGood",
            "durationSensorDampeningBurstProjector",
            "durationTargetIlluminationBurstProjector",
            "durationECMJammerBurstProjector",
            "durationWeaponDisruptionBurstProjector",
        ]
        .iter()
        .map(|n| a(n))
        .filter(|&x| x != 0)
        .collect(),
        fx: fx_flags(f),
    }
}

pub(crate) fn round6(v: f64) -> f64 {
    if v.is_finite() { (v * 1e6).round() / 1e6 } else { v }
}

/// Pyfa eos.utils.float.floatUnerr: round away float noise, keeping 7 significant digits
pub fn float_unerr7(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let rf = 7 - v.abs().log10().ceil() as i32;
    if rf >= 0 {
        format!("{:.*}", rf as usize, v).parse().unwrap_or(v)
    } else {
        let p = 10f64.powi(-rf);
        (v / p).round() * p
    }
}

/// Python round(v, 2) (correctly rounded, ties to even on the exact binary value)
pub fn py_round2(v: f64) -> f64 {
    if !v.is_finite() {
        return v;
    }
    let x = v * 100.0;
    // (eve-dogma-rs 8122ddd) away from a .5 tie the scaled rounding is exact; near a tie use the correctly
    // rounded decimal formatting
    if ((x - x.trunc()).abs() - 0.5).abs() > 1e-6 {
        return x.round() / 100.0;
    }
    format!("{v:.2}").parse().unwrap_or(v)
}

fn tidy(mut v: serde_json::Value) -> serde_json::Value {
    tidy_mut(&mut v);
    v
}

/// In-place rounding (no map rebuilds).
fn tidy_mut(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Number(n) => {
            if n.is_f64() {
                if let Some(f) = n.as_f64() {
                    *v = serde_json::json!(round6(f));
                }
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(tidy_mut),
        serde_json::Value::Object(o) => o.values_mut().for_each(tidy_mut),
        _ => {}
    }
}

/// Attribute ids `validate` needs, resolved once per dataset (instead of format! + name lookup per module).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidateIds {
    can_fit_group: Vec<u32>,
    can_fit_type: Vec<u32>,
    charge_group: Vec<u32>,
    req_skill: [(u32, u32); 6],
}

impl ValidateIds {
    pub fn new(ds: &crate::data::Dataset) -> ValidateIds {
        ValidateIds {
            can_fit_group: (1..=20).map(|k| ds.attr_id(&format!("canFitShipGroup{k:02}"))).filter(|x| *x != 0).collect(),
            can_fit_type: (1..=11).map(|k| ds.attr_id(&format!("canFitShipType{k}"))).filter(|x| *x != 0).collect(),
            // kept unfiltered: an unknown name maps to id 0 exactly like the per-call lookup did
            charge_group: (1..=5).map(|k| ds.attr_id(&format!("chargeGroup{k}"))).collect(),
            req_skill: std::array::from_fn(|k| (ds.attr_id(&format!("requiredSkill{}", k + 1)), ds.attr_id(&format!("requiredSkill{}Level", k + 1)))),
        }
    }
}

impl<'a> Fit<'a> {
    #[inline]
    fn fx(&self, id: &Ids, i: usize, flag: u16) -> bool {
        id.fx[i] & flag != 0
    }

    fn has_effect_named(&self, i: usize, names: &[&str]) -> bool {
        self.items[i].effects.iter().any(|(e, _)| self.ds.effects.get(e).map(|x| names.contains(&x.name.as_str())).unwrap_or(false))
    }

    fn raw_cycle_ms(&self, i: usize, id: &Ids) -> f64 {
        let mut v: f64 = self.get(i, id.speed).max(self.get(i, id.duration));
        for &a in &id.dur_extra {
            v = v.max(self.get(i, a));
        }
        v
    }

    fn num_charges(&self, i: usize) -> u32 {
        let Some(c) = self.items[i].charge else { return 0 };
        let vol = self.get(c, 161);
        let cap = self.base(i, 38);
        if vol <= 0.0 { 0 } else { float_unerr(cap / vol).floor() as u32 }
    }

    fn num_shots(&self, i: usize, id: &Ids) -> u32 {
        let Some(c) = self.items[i].charge else { return 0 };
        let n = self.num_charges(i);
        if n > 0 && self.has(i, id.charge_rate) {
            let r = self.get(i, id.charge_rate);
            return if r > 0.0 { (n as f64 / r).floor() as u32 } else { 0 };
        }
        let cgd = crate::attr_id!(self.ds, "crystalsGetDamaged");
        if n > 0 && self.has(c, cgd) {
            if self.get(c, cgd) == 1.0 {
                let hp = self.get(c, 9);
                let chance = self.get(c, crate::attr_id!(self.ds, "crystalVolatilityChance"));
                let dmg = self.get(c, crate::attr_id!(self.ds, "crystalVolatilityDamage"));
                if dmg * chance > 0.0 {
                    return ((n as f64 * hp) / (dmg * chance)).floor() as u32;
                }
            }
            return 0;
        }
        0
    }

    /// Average cycle time in ms (Pyfa getCycleParameters(...).averageTime)
    fn avg_cycle_ms(&self, i: usize, id: &Ids, factor_reload: bool) -> f64 {
        let active = self.raw_cycle_ms(i, id);
        if active == 0.0 {
            return 0.0;
        }
        let inactive = self.get(i, id.reactivation);
        let shots = self.num_shots(i, id);
        let reload = self.get(i, id.reload);
        if !factor_reload || shots == 0 || inactive >= reload {
            return active + inactive;
        }
        let early = shots as f64 - 1.0;
        ((active + inactive) * early + (active + reload)) / shots as f64
    }

    fn module_volley(&self, i: usize, id: &Ids) -> (Dmg, &'static str) {
        let it = &self.items[i];
        let kind = if self.fx(&id, i, FX_TURRET) {
            "turret"
        } else if self.fx(&id, i, FX_LAUNCHER) {
            "missile"
        } else if self.fx(&id, i, FX_EMP_WAVE) {
            "smartbomb"
        } else if self.fx(&id, i, FX_CHAIN_LIGHTNING) {
            "vorton"
        } else {
            "other"
        };
        let src = it.charge.unwrap_or(i);
        let mut mult = if self.has(i, id.dmg_mult) { self.get(i, id.dmg_mult) } else { 1.0 };
        if kind == "missile" && it.charge.is_some() {
            // missile damage is scaled by the pilot's missileDamageMultiplier (BCS etc. modify the character)
            mult *= self.get(self.char, crate::attr_id!(self.ds, "missileDamageMultiplier"));
        }
        let d = Dmg {
            em: self.get(src, id.dmg[0]) * mult,
            th: self.get(src, id.dmg[1]) * mult,
            ki: self.get(src, id.dmg[2]) * mult,
            ex: self.get(src, id.dmg[3]) * mult,
        };
        (d, kind)
    }

    /// Full stats with every float rounded to 6 decimals.

    pub fn compute_stats(&self, req: &FitRequest) -> serde_json::Value {

        tidy(self.compute_stats_raw(req).to_value())

    }


    /// Stats before rounding: `J::to_string` writes exactly the bytes of `compute_stats` (rounded, sorted keys).

    pub fn compute_stats_raw(&self, req: &FitRequest) -> Value {
        let ds = self.ds;
        let id = ids(self);
        let ship = self.ship;
        let ch = self.char;
        let g = |i: usize, n: &str| self.get(i, ds.attr_id(n));
        let factor_reload = req.options.factor_reload;
        let modules: Vec<usize> = (0..self.items.len()).filter(|&i| self.items[i].kind == Kind::Module).collect();
        let online = |i: usize| self.items[i].state >= State::Online;
        let active = |i: usize| self.items[i].state >= State::Active;

        // ---------------- resources
        let sum = |attr: u32, f: &dyn Fn(usize) -> bool| -> f64 { modules.iter().filter(|&&i| f(i)).map(|&i| self.get(i, attr)).sum() };
        let cpu_used = sum(id.cpu, &online);
        let pg_used = sum(id.power, &online);
        let calib_used: f64 = modules.iter().filter(|&&i| self.items[i].slot == Some(Slot::Rig)).map(|&i| self.get(i, id.upgrade_cost)).sum();
        let drones: Vec<usize> = (0..self.items.len()).filter(|&i| self.items[i].kind == Kind::Drone).collect();
        let fighters: Vec<usize> = (0..self.items.len()).filter(|&i| self.items[i].kind == Kind::Fighter).collect();
        let bw_used: f64 = drones.iter().map(|&i| self.get(i, crate::attr_id!(ds, "droneBandwidthUsed")) * self.items[i].active_count as f64).sum();
        let bay_used: f64 = drones.iter().map(|&i| self.get(i, 161) * self.items[i].quantity as f64).sum();
        let fbay_used: f64 = fighters.iter().map(|&i| self.get(i, 161) * self.items[i].quantity as f64).sum();
        let cargo_used: f64 = req.cargo.iter().map(|c| ds.types.get(&c.type_id).map(|t| t.volume).unwrap_or(0.0) * c.quantity as f64).sum();
        let count_slot = |s: Slot| modules.iter().filter(|&&i| self.items[i].slot == Some(s)).count();
        let turrets_used = modules.iter().filter(|&&i| self.fx(&id, i, FX_TURRET)).count();
        let launchers_used = modules.iter().filter(|&&i| self.fx(&id, i, FX_LAUNCHER)).count();
        let usage = |u: f64, t: f64| json!({"used": u, "total": t});
        let slot_tot = |n: &str| g(ship, n);
        let fighter_class = |i: usize| -> &'static str {
            if self.get(i, crate::attr_id!(ds, "fighterSquadronIsHeavy")) > 0.0 {
                "heavy"
            } else if self.get(i, crate::attr_id!(ds, "fighterSquadronIsSupport")) > 0.0 {
                "support"
            } else {
                "light"
            }
        };
        let tubes_used = fighters.iter().filter(|&&i| self.items[i].active_count > 0).count();
        let class_used = |c: &str| fighters.iter().filter(|&&i| self.items[i].active_count > 0 && fighter_class(i) == c).count() as f64;
        let resources = json!({
            "cpu": crate::out::own(usage(cpu_used, self.get(ship, id.cpu_out))),
            "power": crate::out::own(usage(pg_used, self.get(ship, id.power_out))),
            "calibration": crate::out::own(usage(calib_used, self.get(ship, id.upgrade_cap))),
            "drone_bandwidth": crate::out::own(usage(bw_used, self.get(ship, crate::attr_id!(ds, "droneBandwidth")))),
            "drone_bay": crate::out::own(usage(bay_used, self.get(ship, crate::attr_id!(ds, "droneCapacity")))),
            "fighter_bay": crate::out::own(usage(fbay_used, self.get(ship, crate::attr_id!(ds, "fighterCapacity")))),
            "cargo": crate::out::own(usage(cargo_used, self.get(ship, 38))),
            "slots": {
                "high": crate::out::own(usage(count_slot(Slot::High) as f64, slot_tot("hiSlots"))),
                "mid": crate::out::own(usage(count_slot(Slot::Mid) as f64, slot_tot("medSlots"))),
                "low": crate::out::own(usage(count_slot(Slot::Low) as f64, slot_tot("lowSlots"))),
                "rig": crate::out::own(usage(count_slot(Slot::Rig) as f64, slot_tot("rigSlots"))),
                "subsystem": crate::out::own(usage(count_slot(Slot::Subsystem) as f64, slot_tot("maxSubSystems"))),
                "service": crate::out::own(usage(count_slot(Slot::Service) as f64, slot_tot("serviceSlots"))),
            },
            "hardpoints": {
                "turret": crate::out::own(usage(turrets_used as f64, slot_tot("turretSlotsLeft"))),
                "launcher": crate::out::own(usage(launchers_used as f64, slot_tot("launcherSlotsLeft"))),
            },
            "fighter_tubes": {
                "total": crate::out::own(usage(tubes_used as f64, self.get(ship, crate::attr_id!(ds, "fighterTubes")))),
                "light": crate::out::own(usage(class_used("light"), self.get(ship, crate::attr_id!(ds, "fighterLightSlots")))),
                "support": crate::out::own(usage(class_used("support"), self.get(ship, crate::attr_id!(ds, "fighterSupportSlots")))),
                "heavy": crate::out::own(usage(class_used("heavy"), self.get(ship, crate::attr_id!(ds, "fighterHeavySlots")))),
            },
        });

        // ---------------- offense
        let tp = req.target_profile.clone().unwrap_or_default();
        let tp_res = Resists { em: tp.em, thermal: tp.thermal, kinetic: tp.kinetic, explosive: tp.explosive };
        let default_spool = req.options.default_spool.unwrap_or(Spool { kind: SpoolType::SpoolScale, amount: 1.0 });
        let mut weapons = Vec::new();
        let mut w_vol = Dmg::default();
        let mut w_dps = Dmg::default();
        for &i in &modules {
            if !active(i) {
                continue;
            }
            let (base, kind) = self.module_volley(i, &id);
            if base.total() == 0.0 {
                continue;
            }
            let cyc = self.avg_cycle_ms(i, &id, factor_reload);
            let raw = self.raw_cycle_ms(i, &id);
            let spool = self.items[i].spool.unwrap_or(default_spool);
            let (sp, _, _) = spoolup(self.get(i, crate::attr_id!(ds, "damageMultiplierBonusMax")), self.get(i, crate::attr_id!(ds, "damageMultiplierBonusPerCycle")), raw / 1000.0, spool);
            let vol_spooled = base.scale(1.0 + sp);
            // doomsdays / lances deal their volley every doomsdayDamageCycleTime during doomsdayDamageDuration
            // (Pyfa getVolleyParameters subcycles; the Reaper slash hits once); volley = one tick
            let (dd, dsub) = (self.get(i, crate::attr_id!(ds, "doomsdayDamageDuration")), self.get(i, crate::attr_id!(ds, "doomsdayDamageCycleTime")));
            let subcycles = if dd != 0.0 && dsub != 0.0 && !self.fx(&id, i, FX_DOOMSDAY_SLASH) { float_unerr7(dd / dsub).floor().max(0.0) } else { 1.0 };
            let dps = if cyc > 0.0 { vol_spooled.scale(subcycles * 1000.0 / cyc) } else { Dmg::default() };
            w_vol.add(&vol_spooled); // Pyfa reports spooled volley
            w_dps.add(&dps);
            let opt = self.get(i, crate::attr_id!(ds, "maxRange"));
            let fo = self.get(i, crate::attr_id!(ds, "falloff"));
            let mut w = json!({
                "module_index": self.items[i].req_index, "type_id": self.items[i].type_id,
                "name": ds.types[&self.items[i].type_id].name, "kind": kind,
                "charge_type_id": self.items[i].charge.map(|c| self.items[c].type_id),
                "volley": crate::out::own(vol_spooled.json()), "dps": crate::out::own(dps.json()), "cycle_time_ms": cyc,
            });
            if kind == "turret" {
                w["optimal_m"] = json!(opt);
                w["falloff_m"] = json!(fo);
                w["tracking"] = json!(self.get(i, crate::attr_id!(ds, "trackingSpeed")));
            } else if kind == "missile" {
                if let Some(c) = self.items[i].charge {
                    // Pyfa missileMaxRangeData: flight time + ship radius bonus, acceleration phase,
                    // floor/ceil blend, FoF limit, centre-to-surface (eos/saveddata/module.py, LGPL)
                    let vel = self.get(c, crate::attr_id!(ds, "maxVelocity"));
                    if vel > 0.0 {
                        let radius = self.get(ship, crate::attr_id!(ds, "radius"));
                        let ft = self.get(c, crate::attr_id!(ds, "explosionDelay")) / 1000.0 + radius / vel;
                        let ft = (ft * 1e9).round() / 1e9; // floatUnerr
                        let accel_cap = self.get(c, crate::attr_id!(ds, "mass")) * self.get(c, crate::attr_id!(ds, "agility")) / 1e6;
                        let range_at = |t: f64| {
                            let acc = t.min(accel_cap);
                            vel / 2.0 * acc + vel * (t - acc)
                        };
                        let (lt, ht) = (ft.floor(), ft.ceil());
                        let (mut lr, mut hr) = (range_at(lt), range_at(ht));
                        if self.fx(&id, c, FX_FOF) {
                            let lim = self.get(c, crate::attr_id!(ds, "maxFOFTargetRange"));
                            if lim > 0.0 {
                                lr = lr.min(lim);
                                hr = hr.min(lim);
                            }
                        }
                        lr = (lr - radius).max(0.0);
                        hr = (hr - radius).max(0.0);
                        let hc = ft - lt;
                        w["range_m"] = json!(lr * (1.0 - hc) + hr * hc);
                    }
                    w["explosion_radius"] = json!(self.get(c, crate::attr_id!(ds, "aoeCloudSize")));
                    w["explosion_velocity"] = json!(self.get(c, crate::attr_id!(ds, "aoeVelocity")));
                }
            } else if kind == "smartbomb" {
                w["range_m"] = json!(self.get(i, crate::attr_id!(ds, "empFieldRange")));
            }
            if sp > 0.0 {
                w["spool_multiplier"] = json!(1.0 + sp);
                w["volley_unspooled"] = base.json();
            }
            weapons.push(w);
        }
        let mut d_vol = Dmg::default();
        let mut d_dps = Dmg::default();
        let mut drone_out = Vec::new();
        for &i in &drones {
            let n = self.items[i].active_count as f64;
            if n == 0.0 {
                continue;
            }
            let mult = if self.has(i, id.dmg_mult) { self.get(i, id.dmg_mult) } else { 1.0 };
            let v = Dmg { em: self.get(i, id.dmg[0]), th: self.get(i, id.dmg[1]), ki: self.get(i, id.dmg[2]), ex: self.get(i, id.dmg[3]) }.scale(mult * n);
            let cyc = self.raw_cycle_ms(i, &id);
            if v.total() == 0.0 || cyc == 0.0 {
                continue;
            }
            let dps = v.scale(1000.0 / cyc);
            d_vol.add(&v);
            d_dps.add(&dps);
            drone_out.push(json!({"drone_index": self.items[i].req_index, "type_id": self.items[i].type_id, "name": ds.types[&self.items[i].type_id].name, "count": n, "volley": crate::out::own(v.json()), "dps": crate::out::own(dps.json()),
                "optimal_m": self.get(i, crate::attr_id!(ds, "maxRange")), "falloff_m": self.get(i, crate::attr_id!(ds, "falloff")), "tracking": self.get(i, crate::attr_id!(ds, "trackingSpeed")),
                "max_velocity": self.get(i, crate::attr_id!(ds, "maxVelocity")), "signature_radius": self.get(i, crate::attr_id!(ds, "signatureRadius"))}));
        }
        let mut f_vol = Dmg::default();
        let mut f_dps = Dmg::default();
        let mut fighter_out = Vec::new();
        for &i in &fighters {
            let n = self.items[i].active_count as f64;
            if n == 0.0 {
                continue;
            }
            let mut fv = Dmg::default();
            let mut fd = Dmg::default();
            for (eff, prefix) in [("fighterAbilityAttackM", "fighterAbilityAttackMissile"), ("fighterAbilityMissiles", "fighterAbilityMissiles")] {
                let eid = ds.effect_id(eff);
                let Some(&(_, def)) = self.items[i].effects.iter().find(|(e, _)| *e == eid) else { continue };
                let used = match &self.items[i].fighter_abilities {
                    Some(l) => l.contains(&eid),
                    None => def,
                };
                if !used {
                    continue;
                }
                let m = g(i, &format!("{prefix}DamageMultiplier"));
                let m = if m == 0.0 { 1.0 } else { m };
                let v = Dmg {
                    em: g(i, &format!("{prefix}DamageEM")),
                    th: g(i, &format!("{prefix}DamageTherm")),
                    ki: g(i, &format!("{prefix}DamageKin")),
                    ex: g(i, &format!("{prefix}DamageExp")),
                }
                .scale(m * n);
                let dur = g(i, &format!("{prefix}Duration"));
                fv.add(&v);
                if dur > 0.0 {
                    fd.add(&v.scale(1000.0 / dur));
                }
            }
            if fv.total() > 0.0 {
                f_vol.add(&fv);
                f_dps.add(&fd);
                fighter_out.push(json!({"fighter_index": self.items[i].req_index, "type_id": self.items[i].type_id, "name": ds.types[&self.items[i].type_id].name, "squadron_size": n, "volley": crate::out::own(fv.json()), "dps": crate::out::own(fd.json()),
                    "max_velocity": self.get(i, crate::attr_id!(ds, "maxVelocity")), "signature_radius": self.get(i, crate::attr_id!(ds, "signatureRadius"))}));
            }
        }
        let mut t_vol = w_vol;
        t_vol.add(&d_vol);
        t_vol.add(&f_vol);
        let mut t_dps = w_dps;
        t_dps.add(&d_dps);
        t_dps.add(&f_dps);
        let offense = json!({
            "weapons": crate::out::own(weapons), "drones": crate::out::own(drone_out), "fighters": crate::out::own(fighter_out),
            "total": {"weapon_dps": w_dps.total(), "weapon_volley": w_vol.total(), "drone_dps": d_dps.total(), "drone_volley": d_vol.total(),
                      "fighter_dps": f_dps.total(), "fighter_volley": f_vol.total(), "dps": crate::out::own(t_dps.json()), "volley": crate::out::own(t_vol.json())},
            "vs_target_profile": {"dps": t_dps.vs(&tp_res), "volley": t_vol.vs(&tp_res)},
        });

        // ---------------- defense
        let dp = req.damage_pattern.unwrap_or(Resists { em: 25.0, thermal: 25.0, kinetic: 25.0, explosive: 25.0 });
        let dp_tot = (dp.em + dp.thermal + dp.kinetic + dp.explosive).max(1e-12);
        let layer_res = |prefix: &str| -> [f64; 4] {
            let names: [String; 4] = if prefix.is_empty() {
                ["emDamageResonance".into(), "thermalDamageResonance".into(), "kineticDamageResonance".into(), "explosiveDamageResonance".into()]
            } else {
                [format!("{prefix}EmDamageResonance"), format!("{prefix}ThermalDamageResonance"), format!("{prefix}KineticDamageResonance"), format!("{prefix}ExplosiveDamageResonance")]
            };
            [g(ship, &names[0]), g(ship, &names[1]), g(ship, &names[2]), g(ship, &names[3])]
        };
        let effectivify = |amount: f64, r: [f64; 4]| {
            let div = (dp.em * r[0] + dp.thermal * r[1] + dp.kinetic * r[2] + dp.explosive * r[3]) / dp_tot;
            if div == 0.0 { amount } else { amount / div }
        };
        let (rs, ra, rh) = (layer_res("shield"), layer_res("armor"), layer_res(""));
        let hp_s = self.get(ship, crate::attr_id!(ds, "shieldCapacity"));
        let hp_a = self.get(ship, crate::attr_id!(ds, "armorHP"));
        let hp_h = self.get(ship, 9);
        let (e_s, e_a, e_h) = (effectivify(hp_s, rs), effectivify(hp_a, ra), effectivify(hp_h, rh));
        let res_json = |r: [f64; 4]| json!({"em": r[0], "thermal": r[1], "kinetic": r[2], "explosive": r[3]});
        // local repairs
        let mut shield_rep = 0.0;
        let mut armor_rep = 0.0;
        let mut hull_rep = 0.0;
        for &i in &modules {
            if !active(i) {
                continue;
            }
            let dur = self.get(i, id.duration) / 1000.0;
            if dur <= 0.0 {
                continue;
            }
            if self.fx(&id, i, FX_SHIELD_BOOST) {
                shield_rep += self.get(i, crate::attr_id!(ds, "shieldBonus")) / dur;
            }
            if self.fx(&id, i, FX_ARMOR_REP) {
                armor_rep += self.get(i, crate::attr_id!(ds, "armorDamageAmount")) / dur;
            }
            if self.fx(&id, i, FX_FUELED_ARMOR_REP) {
                let paste = self.items[i].charge.map(|c| ds.types[&self.items[c].type_id].name == "Nanite Repair Paste").unwrap_or(false);
                armor_rep += self.get(i, crate::attr_id!(ds, "armorDamageAmount")) * if paste { 3.0 } else { 1.0 } / dur;
            }
            if self.fx(&id, i, FX_HULL_REP) {
                hull_rep += self.get(i, crate::attr_id!(ds, "structureDamageAmount")) / dur;
            }
        }
        // incoming remote repairs (Pyfa __getAppliedRr diminishing-returns formula)
        {
            let mut lists: [Vec<(f64, f64)>; 3] = [Vec::new(), Vec::new(), Vec::new()];
            for ps in &self.proj_special {
                if let crate::engine::ProjSpecial::Rep { item, layer, amount, mult, factor } = *ps {
                    let dur = self.get(item, id.duration) / 1000.0;
                    if dur > 0.0 {
                        lists[layer as usize].push((self.get(item, amount) * mult * factor, dur));
                    }
                }
            }
            let applied = |l: &Vec<(f64, f64)>| -> f64 {
                let total: f64 = l.iter().map(|(a, c)| a / c.trunc()).sum();
                l.iter()
                    .map(|(a, c)| {
                        let rrps = a / c.trunc();
                        let m = 7000.0 + rrps * 20.0;
                        (1.0 - (((rrps + m) / (total + m)) - 1.0).powi(2)) * a / c
                    })
                    .sum()
            };
            shield_rep += applied(&lists[0]);
            armor_rep += applied(&lists[1]);
            hull_rep += applied(&lists[2]);
        }
        let shield_rr_s = self.get(ship, crate::attr_id!(ds, "shieldRechargeRate")) / 1000.0;
        let passive = if shield_rr_s > 0.0 { 10.0 / shield_rr_s * 0.5 * 0.5 * hp_s } else { 0.0 };
        let mut defense = json!({
            "hp": {"shield": hp_s, "armor": hp_a, "hull": hp_h, "total": hp_s + hp_a + hp_h},
            "resonance": {"shield": crate::out::own(res_json(rs)), "armor": crate::out::own(res_json(ra)), "hull": crate::out::own(res_json(rh))},
            "ehp": {"shield": e_s, "armor": e_a, "hull": e_h, "total": e_s + e_a + e_h},
            "damage_pattern": {"em": dp.em, "thermal": dp.thermal, "kinetic": dp.kinetic, "explosive": dp.explosive},
            "tank": {
                "raw": {"passive_shield": passive, "shield_repair": shield_rep, "armor_repair": armor_rep, "hull_repair": hull_rep},
                "effective": {"passive_shield": effectivify(passive, rs), "shield_repair": effectivify(shield_rep, rs),
                              "armor_repair": effectivify(armor_rep, ra), "hull_repair": effectivify(hull_rep, rh)},
            },
        });

        // ---------------- capacitor
        let cap = self.get(ship, crate::attr_id!(ds, "capacitorCapacity"));
        let rr = self.get(ship, crate::attr_id!(ds, "rechargeRate"));
        let peak = if rr > 0.0 { 10.0 / (rr / 1000.0) * 0.5 * 0.5 * cap } else { 0.0 };
        let mut drains = Vec::new();
        let mut cap_used = 0.0;
        let mut cap_added = 0.0;
        let booster_grp = |i: usize| ds.groups.get(&self.items[i].group).map(|g| g.name == "Capacitor Booster").unwrap_or(false);
        let mut module_rows = Vec::new();
        for &i in &modules {
            let mut cap_need = self.get(i, id.cap_need);
            let is_inj = booster_grp(i);
            if is_inj {
                cap_need = -self.items[i].charge.map(|c| self.get(c, crate::attr_id!(ds, "capacitorBonus"))).unwrap_or(0.0);
            }
            if self.fx(&id, i, FX_NOS) && !req.options.nos_no_target_cap {
                // local nosferatu counts as cap income (assumes the target has cap), like Pyfa
                cap_need = -self.get(i, crate::attr_id!(ds, "powerTransferAmount"));
            }
            let cyc_raw = self.raw_cycle_ms(i, &id);
            let full = cyc_raw + self.get(i, id.reactivation);
            let mut row = json!({"module_index": self.items[i].req_index, "type_id": self.items[i].type_id,
                "name": ds.types[&self.items[i].type_id].name, "slot": self.items[i].slot, "state": self.items[i].state,
                "cpu": self.get(i, id.cpu), "power": self.get(i, id.power)});
            if cyc_raw > 0.0 {
                row["cycle_time_ms"] = json!(cyc_raw);
            }
            if active(i) && cap_need != 0.0 && full > 0.0 {
                // Pyfa forces reload into capacitor boosters' average cycle (module.forceReload)
                let avg = self.avg_cycle_ms(i, &id, factor_reload || is_inj);
                let use_ = if avg > 0.0 { cap_need / (avg / 1000.0) } else { 0.0 };
                if use_ > 0.0 { cap_used += use_ } else { cap_added -= use_ }
                row["cap_use_gj_s"] = json!(use_);
                drains.push(Drain {
                    duration: full.trunc(),
                    cap_need,
                    clip_size: self.num_shots(i, &id),
                    reload_ms: self.get(i, id.reload),
                    is_injector: is_inj,
                    disable_stagger: self.fx(&id, i, FX_TURRET),
                });
            }
            module_rows.push(row);
        }
        // incoming neuts / nos / cap transfers (Pyfa fit.addDrain): no stagger, after the fit's own modules
        let sig_now = self.get(ship, crate::attr_id!(ds, "signatureRadius"));
        for ps in &self.proj_special {
            if let crate::engine::ProjSpecial::Drain { item, amount, duration, factor, resist, sign } = *ps {
                let mut need = self.get(item, amount) * factor * sign;
                if resist != 0 {
                    need *= self.get(ship, resist);
                }
                let sres = self.get(item, crate::attr_id!(ds, "energyNeutralizerSignatureResolution"));
                if sres != 0.0 {
                    need *= (sig_now / sres).min(1.0);
                }
                let dur = self.get(item, duration);
                if need != 0.0 && dur > 0.0 {
                    if need > 0.0 { cap_used += need / (dur.trunc() / 1000.0) } else { cap_added -= need / (dur.trunc() / 1000.0) }
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
            crate::engine::prof_start();
            let r = capsim::simulate(cap, rr, &drains, 1.0, o.reload || factor_reload, true, o.max_time_s.unwrap_or(6.0 * 3600.0) * 1000.0);
            crate::engine::prof(4);
            let st = (r.stable_low + r.stable_high) / 2.0;
            capj["stable"] = json!(r.stable && st > 0.0);
            if r.stable && st > 0.0 {
                capj["stable_percent"] = json!((st * 100.0).min(100.0));
            } else {
                capj["depletes_in_s"] = json!(r.t_s);
            }
            capj["eve_stable_percent"] = json!(r.eve_stable * 100.0);
            capj["sim_iterations"] = json!(r.iterations);
        }

        // ---------------- sustainable tank (Pyfa Fit.sustainableTank, eos LGPL): when the capacitor is not
        // stable (or reload is factored), local cap-using repairers only run as far as peak recharge allows.
        {
            let stable_now = capj["stable"].as_bool().unwrap_or(true);
            let mut sus = [shield_rep, armor_rep, hull_rep];
            if !stable_now || factor_reload {
                let grp_of = |i: usize| ds.groups.get(&self.items[i].group).map(|g| g.name.as_str()).unwrap_or("");
                let spec = |gname: &str| -> Option<(usize, &'static str)> {
                    match gname {
                        "Shield Booster" | "Ancillary Shield Booster" => Some((0, "shieldBonus")),
                        "Armor Repair Unit" | "Ancillary Armor Repairer" => Some((1, "armorDamageAmount")),
                        "Hull Repair Unit" => Some((2, "structureDamageAmount")),
                        _ => None,
                    }
                };
                let mut adj = [0.0f64; 3];
                let mut used = cap_used;
                let mut reps: Vec<(usize, usize, &'static str, f64)> = Vec::new();
                for layer in 0..3 {
                    for &i in &modules {
                        if !active(i) {
                            continue;
                        }
                        let gname = grp_of(i);
                        let Some((l, attr)) = spec(gname) else { continue };
                        if l != layer {
                            continue;
                        }
                        let cap_need = self.get(i, id.cap_need);
                        let avg = self.avg_cycle_ms(i, &id, factor_reload);
                        let cap_use = if cap_need != 0.0 && avg > 0.0 { cap_need / (avg / 1000.0) } else { 0.0 };
                        let cyc = self.raw_cycle_ms(i, &id);
                        if cyc <= 0.0 {
                            continue;
                        }
                        let amount = g(i, attr);
                        let charge = self.items[i].charge;
                        let paste = charge.map(|c| ds.types[&self.items[c].type_id].name == "Nanite Repair Paste").unwrap_or(false);
                        if cap_use != 0.0 {
                            used -= cap_use;
                            let mult = if paste { let m = self.get(i, crate::attr_id!(ds, "chargedArmorDamageMultiplier")); if m == 0.0 { 1.0 } else { m } } else { 1.0 };
                            adj[l] -= amount * mult / (cyc / 1000.0);
                            reps.push((i, l, attr, cap_use));
                        } else if gname == "Ancillary Shield Booster" {
                            let reload = if factor_reload && charge.is_some() { self.get(i, id.reload) } else { 0.0 };
                            let shots = self.num_shots(i, &id).max(1) as f64;
                            let off = reload / (shots * cyc + reload);
                            adj[l] -= amount * off / (cyc / 1000.0);
                        }
                    }
                }
                let eff = |i: usize, attr: &str| {
                    let m = self.get(i, crate::attr_id!(ds, "chargedArmorDamageMultiplier"));
                    g(i, attr) * if m == 0.0 { 1.0 } else { m } / self.get(i, id.cap_need)
                };
                reps.sort_by(|a, b| eff(b.0, b.2).partial_cmp(&eff(a.0, a.2)).unwrap_or(std::cmp::Ordering::Equal));
                let total_peak = peak + cap_added;
                for (i, l, attr, cap_use) in reps {
                    if used > total_peak {
                        break;
                    }
                    let charge = self.items[i].charge;
                    let reload = if factor_reload && charge.is_some() { self.get(i, id.reload) } else { 0.0 };
                    let cyc = self.raw_cycle_ms(i, &id);
                    let sustain = ((total_peak - used) / cap_use).min(1.0);
                    let amount = g(i, attr);
                    if charge.is_none() {
                        adj[l] += sustain * amount / (cyc / 1000.0);
                    } else {
                        let paste = ds.types[&self.items[charge.unwrap()].type_id].name == "Nanite Repair Paste";
                        let mult = if paste { let m = self.get(i, crate::attr_id!(ds, "chargedArmorDamageMultiplier")); if m == 0.0 { 1.0 } else { m } } else { 1.0 };
                        let shots = self.num_shots(i, &id).max(1) as f64;
                        let on = shots * cyc / (shots * cyc + reload);
                        adj[l] += sustain * amount * on * mult / (cyc / 1000.0);
                    }
                    used += cap_use;
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
        let maxv = self.get(ship, crate::attr_id!(ds, "maxVelocity"));
        let limit = self.get(ship, crate::attr_id!(ds, "speedLimit"));
        let max_speed = if limit > 0.0 && maxv > limit { limit } else { maxv };
        let mass = self.get(ship, 4);
        let agility = self.get(ship, crate::attr_id!(ds, "agility"));
        let base_warp = { let v = self.get(ship, crate::attr_id!(ds, "baseWarpSpeed")); if v == 0.0 { 1.0 } else { v } };
        let warp_mult = { let v = self.get(ship, crate::attr_id!(ds, "warpSpeedMultiplier")); if v == 0.0 { 1.0 } else { v } };
        let warp_need = self.get(ship, crate::attr_id!(ds, "warpCapacitorNeed"));
        let sig = self.get(ship, crate::attr_id!(ds, "signatureRadius"));
        let navigation = json!({
            "max_velocity": max_speed, "align_time_s": -(0.25f64.ln()) * agility * mass / 1e6, "mass": mass, "agility": agility,
            "signature_radius": sig, "warp_speed_au_s": base_warp * warp_mult,
            "max_warp_distance_au": if warp_need > 0.0 && mass > 0.0 { cap / (mass * warp_need) } else { 0.0 },
            "warp_scramble_status": self.get(ship, crate::attr_id!(ds, "warpScrambleStatus")),
        });

        // ---------------- targeting
        let strengths = [("radar", "scanRadarStrength"), ("ladar", "scanLadarStrength"), ("magnetometric", "scanMagnetometricStrength"), ("gravimetric", "scanGravimetricStrength")];
        let mut best = ("none", 0.0f64);
        for (n, at) in strengths {
            let v = g(ship, at);
            if v > best.1 {
                best = (n, v);
            }
        }
        // ECM jam chance (Pyfa Fit.jamChance): strengths vs the strongest sensor type (ties -> multispectral -> 0)
        let jam = {
            let mut max_s = -1.0f64;
            let mut ty: Option<&str> = None;
            for t in ["Magnetometric", "Ladar", "Radar", "Gravimetric"] {
                let v = g(ship, &format!("scan{t}Strength"));
                if v > max_s {
                    max_s = v;
                    ty = Some(t);
                } else if v == max_s {
                    ty = None;
                }
            }
            let mut retain = 1.0f64;
            let mut any = false;
            for ps in &self.proj_special {
                if let crate::engine::ProjSpecial::Ecm { item, fighter, factor, resist } = *ps {
                    any = true;
                    let Some(t) = ty else { continue };
                    let attr = if fighter { format!("fighterAbilityECMStrength{t}") } else { format!("scan{t}StrengthBonus") };
                    let mut st = g(item, &attr) * factor;
                    if resist != 0 {
                        let r = self.get(ship, resist);
                        if r != 0.0 {
                            st *= r;
                        }
                    }
                    if max_s > 0.0 {
                        retain *= 1.0 - (st / max_s).min(1.0);
                    }
                }
            }
            if any { Some((1.0 - retain) * 100.0) } else { None }
        };
        let scan_res = self.get(ship, crate::attr_id!(ds, "scanResolution"));
        let lt = |s: f64| lock_time(scan_res, s);
        let ship_targets = self.get(ship, crate::attr_id!(ds, "maxLockedTargets"));
        let char_targets = self.get(ch, crate::attr_id!(ds, "maxLockedTargets"));
        let targeting = json!({
            "max_targets": ship_targets.min(char_targets.max(0.0)),
            "max_range_m": self.get(ship, crate::attr_id!(ds, "maxTargetRange")), "scan_resolution": scan_res,
            "sensor_strength": best.1, "sensor_type": best.0, "jam_chance_percent": jam.unwrap_or(0.0),
            "probe_size": if best.1 > 0.0 { Some((sig / best.1).max(1.08)) } else { None },
            "lock_time_s": {"sig_25m": lt(25.0), "sig_40m": lt(40.0), "sig_125m": lt(125.0), "sig_400m": lt(400.0), "sig_target_profile": tp.signature_radius.and_then(lt)},
        });

        let drones_j = json!({
            "active": drones.iter().map(|&i| self.items[i].active_count).sum::<u32>(),
            "max_active": self.get(ch, crate::attr_id!(ds, "maxActiveDrones")),
            "control_range_m": self.get(ch, crate::attr_id!(ds, "droneControlDistance")),
        });

        let mut out = Map::new();
        out.insert("meta", json!({"schema_version": 1, "engine": concat!("eve-dogma-vb ", env!("CARGO_PKG_VERSION")),
            "sde_build": ds.build, "dataset_sha256": ds.sha256}));
        let st = &ds.types[&self.items[ship].type_id];
        out.insert("ship", json!({"type_id": st.id, "name": st.name, "group": ds.groups.get(&st.group).map(|g| g.name.clone())}));
        out.insert("resources", resources);
        out.insert("offense", offense);
        out.insert("defense", defense);
        out.insert("capacitor", capj);
        out.insert("navigation", navigation);
        out.insert("targeting", targeting);
        out.insert("drones", drones_j);
        out.insert("modules", Value::Array(module_rows));
        if req.options.validate {
            out.insert("violations", Value::Array(self.validate(req, cpu_used, pg_used, calib_used, bw_used)));
        }
        if !self.warnings.is_empty() {
            out.insert("warnings", json!(self.warnings));
        }
        match req.options.include_attributes.as_deref() {
            Some("ship") => {
                out.insert("attributes", json!({"ship": crate::out::own(self.dump_attrs(ship))}));
            }
            Some("all") => {
                let mut m = Map::new();
                m.insert("ship", self.dump_attrs(ship));
                m.insert("character", self.dump_attrs(ch));
                let mods: Vec<Value> = modules.iter().map(|&i| {
                    json!({"module_index": self.items[i].req_index, "type_id": self.items[i].type_id, "attributes": crate::out::own(self.dump_attrs(i)),
                           "charge": self.items[i].charge.map(|c| self.dump_attrs(c))})
                }).collect();
                m.insert("modules", Value::Array(mods));
                let dr: Vec<Value> = drones.iter().map(|&i| json!({"drone_index": self.items[i].req_index, "attributes": crate::out::own(self.dump_attrs(i))})).collect();
                m.insert("drones", Value::Array(dr));
                out.insert("attributes", Value::Object(m));
            }
            _ => {}
        }
        Value::Object(out)
    }

    pub fn dump_attrs(&self, i: usize) -> Value {
        let keys = self.attr_keys(i);
        let mut m = Map::new();
        for k in keys {
            let name = self.ds.attrs.get(&k).map(|a| a.name.clone()).unwrap_or_else(|| k.to_string());
            m.insert(name, json!(self.get(i, k)));
        }
        Value::Object(m)
    }

    fn validate(&self, req: &FitRequest, cpu: f64, pg: f64, calib: f64, bw: f64) -> Vec<Value> {
        let ds = self.ds;
        let ship = self.ship;
        let g = |i: usize, n: &str| self.get(i, ds.attr_id(n));
        let mut v = Vec::new();
        let mut push = |code: &str, msg: String, idx: Option<usize>| v.push(json!({"code": code, "message": msg, "module_index": idx}));
        if cpu > self.get(ship, crate::attr_id!(ds, "cpuOutput")) + 1e-9 {
            push("CPU_OVERLOAD", format!("CPU used {cpu:.2} > output {:.2}", self.get(ship, crate::attr_id!(ds, "cpuOutput"))), None);
        }
        if pg > self.get(ship, crate::attr_id!(ds, "powerOutput")) + 1e-9 {
            push("POWER_OVERLOAD", format!("Powergrid used {pg:.2} > output {:.2}", self.get(ship, crate::attr_id!(ds, "powerOutput"))), None);
        }
        if calib > self.get(ship, crate::attr_id!(ds, "upgradeCapacity")) + 1e-9 {
            push("CALIBRATION_OVERLOAD", format!("Calibration used {calib} > {}", self.get(ship, crate::attr_id!(ds, "upgradeCapacity"))), None);
        }
        if bw > self.get(ship, crate::attr_id!(ds, "droneBandwidth")) + 1e-9 {
            push("DRONE_BANDWIDTH", format!("Drone bandwidth used {bw} > {}", self.get(ship, crate::attr_id!(ds, "droneBandwidth"))), None);
        }
        let modules: Vec<usize> = (0..self.items.len()).filter(|&i| self.items[i].kind == Kind::Module).collect();
        for (slot, attr) in [(Slot::High, "hiSlots"), (Slot::Mid, "medSlots"), (Slot::Low, "lowSlots"), (Slot::Rig, "rigSlots"), (Slot::Subsystem, "maxSubSystems"), (Slot::Service, "serviceSlots")] {
            let used = modules.iter().filter(|&&i| self.items[i].slot == Some(slot)).count() as f64;
            if used > g(ship, attr) {
                push("SLOTS_EXCEEDED", format!("{slot:?} slots used {used} > {}", g(ship, attr)), None);
            }
        }
        let t = modules.iter().filter(|&&i| self.has_effect_named(i, &["turretFitted"])).count() as f64;
        if t > self.get(ship, crate::attr_id!(ds, "turretSlotsLeft")) {
            push("TURRET_HARDPOINTS", format!("turrets {t} > hardpoints {}", self.get(ship, crate::attr_id!(ds, "turretSlotsLeft"))), None);
        }
        let l = modules.iter().filter(|&&i| self.has_effect_named(i, &["launcherFitted"])).count() as f64;
        if l > self.get(ship, crate::attr_id!(ds, "launcherSlotsLeft")) {
            push("LAUNCHER_HARDPOINTS", format!("launchers {l} > hardpoints {}", self.get(ship, crate::attr_id!(ds, "launcherSlotsLeft"))), None);
        }
        let ship_t = &ds.types[&self.items[ship].type_id];
        let vids = &self.prep.vids;
        let groups_attrs = &vids.can_fit_group;
        let types_attrs = &vids.can_fit_type;
        let (a_mgf, a_mtf, a_mgo, a_mga) =
            (crate::attr_id!(ds, "maxGroupFitted"), crate::attr_id!(ds, "maxTypeFitted"), crate::attr_id!(ds, "maxGroupOnline"), crate::attr_id!(ds, "maxGroupActive"));
        let (a_rig, a_csize) = (crate::attr_id!(ds, "rigSize"), crate::attr_id!(ds, "chargeSize"));
        // canFitShip* attribute ids in ascending order (true = group restriction), merged against each
        // module's sorted attribute list below instead of one binary search per id
        let mut fit_ids: Vec<(u32, bool)> = groups_attrs.iter().map(|&a| (a, true)).chain(types_attrs.iter().map(|&a| (a, false))).collect();
        fit_ids.sort_unstable();
        let mut fitted_group: crate::hash::FxHashMap<u32, u32> = Default::default();
        let mut fitted_type: crate::hash::FxHashMap<u32, u32> = Default::default();
        let mut active_group: crate::hash::FxHashMap<u32, u32> = Default::default();
        let mut online_group: crate::hash::FxHashMap<u32, u32> = Default::default();
        for &i in &modules {
            let it = &self.items[i];
            let idx = it.req_index;
            let name = &ds.types[&it.type_id].name;
            let mt = &ds.types[&it.type_id];
            if it.slot.is_none() {
                push("NOT_FITTABLE", format!("{name} is not a fittable module"), idx);
            }
            // restricted = some nonzero canFitShip* value; allowed = one of them names the ship's group / type
            let (mut restricted, mut allowed) = (false, false);
            if let Some(&(first, _)) = fit_ids.first() {
                let at = &mt.attrs;
                let mut p = at.partition_point(|x| x.0 < first);
                for &(id, is_group) in &fit_ids {
                    while p < at.len() && at[p].0 < id {
                        p += 1;
                    }
                    if p == at.len() {
                        break;
                    }
                    if at[p].0 == id {
                        let v = at[p].1 as u32;
                        if v != 0 {
                            restricted = true;
                            allowed |= if is_group { v == ship_t.group } else { v == ship_t.id };
                        }
                    }
                }
            }
            if restricted && !allowed {
                push("SHIP_RESTRICTION", format!("{name} cannot be fitted to {}", ship_t.name), idx);
            }
            if it.slot == Some(Slot::Rig) {
                let rs = mt.attr(a_rig).unwrap_or(0.0);
                let srs = self.get(ship, crate::attr_id!(ds, "rigSize"));
                if rs != 0.0 && rs != srs {
                    push("RIG_SIZE", format!("{name} rig size {rs} != ship rig size {srs}"), idx);
                }
            }
            *fitted_group.entry(it.group).or_default() += 1;
            *fitted_type.entry(it.type_id).or_default() += 1;
            if it.state >= State::Online {
                *online_group.entry(it.group).or_default() += 1;
            }
            if it.state >= State::Active {
                *active_group.entry(it.group).or_default() += 1;
            }
            let check = |a: u32, map: &crate::hash::FxHashMap<u32, u32>, key: u32| -> Option<(f64, u32)> {
                let lim = mt.attr(a)?;
                let n = *map.get(&key).unwrap_or(&0);
                if lim > 0.0 && n as f64 > lim { Some((lim, n)) } else { None }
            };
            if let Some((lim, n)) = check(a_mgf, &fitted_group, it.group) {
                push("MAX_GROUP_FITTED", format!("{name}: {n} fitted of group, max {lim}"), idx);
            }
            if let Some((lim, n)) = check(a_mtf, &fitted_type, it.type_id) {
                push("MAX_TYPE_FITTED", format!("{name}: {n} fitted, max {lim}"), idx);
            }
            if let Some((lim, n)) = check(a_mgo, &online_group, it.group) {
                push("MAX_GROUP_ONLINE", format!("{name}: {n} online of group, max {lim}"), idx);
            }
            if let Some((lim, n)) = check(a_mga, &active_group, it.group) {
                push("MAX_GROUP_ACTIVE", format!("{name}: {n} active of group, max {lim}"), idx);
            }
            if let Some(c) = it.charge {
                let ct = &ds.types[&self.items[c].type_id];
                let cg_ok = vids.charge_group.iter().filter_map(|&a| mt.attr(a)).any(|v| v as u32 != 0 && v as u32 == ct.group);
                if !cg_ok {
                    push("CHARGE_GROUP", format!("{} cannot be loaded into {name}", ct.name), idx);
                }
                let ms = mt.attr(a_csize);
                let cs = ct.attr(a_csize);
                if let (Some(a), Some(b)) = (ms, cs) {
                    if a != b {
                        push("CHARGE_SIZE", format!("{} size {b} != launcher size {a}", ct.name), idx);
                    }
                }
                if ct.volume > mt.capacity && mt.capacity > 0.0 {
                    push("CHARGE_CAPACITY", format!("{} does not fit into {name}", ct.name), idx);
                }
            }
        }
        // skills
        // self.skills is sorted by id and unique: binary search instead of building a map per fit
        let have = |s: u32| -> f64 { self.skills.binary_search_by_key(&s, |x| x.0).map(|p| self.skills[p].1 as f64).unwrap_or(0.0) };
        let mut missing: Vec<(u32, f64, u32)> = Vec::new();
        for it in &self.items {
            if !matches!(it.kind, Kind::Ship | Kind::Module | Kind::Charge | Kind::Drone | Kind::Fighter | Kind::Implant | Kind::Booster) {
                continue;
            }
            let t = &ds.types[&it.type_id];
            for &(sa, la) in &vids.req_skill {
                let s = t.attr(sa).unwrap_or(0.0) as u32;
                if s == 0 {
                    continue;
                }
                let need = t.attr(la).unwrap_or(1.0);
                if have(s) < need && !missing.iter().any(|m| m.0 == s && m.1 >= need) {
                    missing.push((s, need, it.type_id));
                }
            }
        }
        for (s, need, by) in missing {
            push("MISSING_SKILL", format!("{} {} required by {}", ds.types.get(&s).map(|t| t.name.as_str()).unwrap_or("?"), need, ds.types[&by].name), None);
        }
        let _ = req;
        v
    }
}

#[cfg(test)]
#[path = "tests/stats.rs"]
mod tests;
