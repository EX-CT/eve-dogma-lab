//! `damage` graph (Pyfa "Damage Stats"): applied dps / volley / inflicted damage vs distance, time, target speed
//! or target signature radius. Shared application math is also used by `application_profile`.
use super::common::*;
use super::cycles::{Cycles, module_cycles};
use super::{GErr, GraphRequest, in_range};
use crate::api::build_calc;
use crate::data::Dataset;
use crate::eos::cx::{Fit, It, calc_range_factor, NONE};
use crate::eos::stats::float_unerr;
use crate::request::{FitRequest, Spool, SpoolType, StateReq};

const INF: f64 = f64::INFINITY;

// ---------------------------------------------------------------- damage container (Pyfa DmgTypes)

/// em/th/ki/ex plus breacher entries (key, absolute, relative)
#[derive(Clone, Default, Debug)]
pub struct Gd {
    pub d: [f64; 4],
    pub br: Vec<(f64, f64, f64)>,
}

impl Gd {
    pub fn from4(em: f64, th: f64, ki: f64, ex: f64) -> Gd {
        Gd { d: [em, th, ki, ex], br: vec![] }
    }
    pub fn add(&mut self, o: &Gd) {
        for i in 0..4 {
            self.d[i] += o.d[i];
        }
        self.br.extend_from_slice(&o.br);
    }
    pub fn mul(&self, f: f64) -> Gd {
        Gd { d: [self.d[0] * f, self.d[1] * f, self.d[2] * f, self.d[3] * f], br: self.br.iter().map(|b| (b.0, b.1 * f, b.2 * f)).collect() }
    }
    fn keys(&self) -> Vec<f64> {
        let mut k: Vec<f64> = Vec::new();
        for b in &self.br {
            if !k.contains(&b.0) {
                k.push(b.0);
            }
        }
        k.sort_by(|a, b| a.partial_cmp(b).unwrap());
        k
    }
    /// pure damage: per breacher key the best min(abs, rel·hp) (hp None = no profile: abs)
    pub fn pure(&self, hp: Option<f64>) -> f64 {
        let mut s = 0.0;
        for k in self.keys() {
            let mut best: f64 = 0.0;
            let mut first = true;
            for b in self.br.iter().filter(|b| b.0 == k) {
                let v = match hp {
                    None => b.1,
                    Some(h) => b.1.min(b.2 * h),
                };
                if first || v > best {
                    best = v;
                    first = false;
                }
            }
            s += best;
        }
        s
    }
    /// total without a profile
    pub fn raw_total(&self) -> f64 {
        self.d[0] + self.d[1] + self.d[2] + self.d[3] + self.pure(None)
    }
    pub fn total_vs(&self, res: [f64; 4], hp: f64) -> f64 {
        let mut t = 0.0;
        for i in 0..4 {
            t += self.d[i] * (1.0 - res[i]);
        }
        t + self.pure(Some(hp))
    }
    fn eq(&self, o: &Gd) -> bool {
        (0..4).all(|i| float_unerr(self.d[i]) == float_unerr(o.d[i])) && self.keys() == o.keys()
    }
}

fn gd_of(d: crate::eos::stats::Dmg) -> Gd {
    Gd::from4(d.em, d.th, d.ki, d.ex)
}

// ---------------------------------------------------------------- dealers

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Dk {
    M(It),
    D(It),
    F(It, u32),
}

/// Module.getVolleyParameters as Gd (breachers carry their DoT data)
pub fn mod_volley_params(fit: &Fit, m: It, spool: Option<Spool>) -> Vec<(f64, Gd)> {
    if fit.is_breacher(m) && fit.items[m].state >= crate::eos::cx::ACTIVE {
        let c = fit.items[m].charge;
        let sub = (fit.g(c, "dotDuration") / 1000.0).floor() as i64;
        let abs = fit.g(c, "dotMaxDamagePerTick");
        let rel = fit.g(c, "dotMaxHPPercentagePerTick") / 100.0;
        let sp = crate::eos::stats::calculate_spoolup(fit.g(m, "damageMultiplierBonusMax"), fit.g(m, "damageMultiplierBonusPerCycle"), fit.raw_cycle_time(m) / 1000.0, spool).0;
        let k = 1.0 + sp;
        return (0..sub).map(|i| (1.0 + i as f64, Gd { d: [0.0; 4], br: vec![(1.0 + i as f64, abs * k, rel * k)] })).collect();
    }
    fit.volley_params_sp(m, spool).into_iter().map(|(t, d)| (t, gd_of(d))).collect()
}

fn mod_deals(fit: &Fit, m: It) -> bool {
    mod_volley_params(fit, m, fit.spool_opts(m, None)).iter().any(|x| x.1.raw_total() > 0.0)
}

fn ability_cycles_reload(fit: &Fit, f: It, factor: bool) -> Vec<(u32, Cycles)> {
    let ab: Vec<u32> = fit.items[f].abilities.iter().map(|a| a.0).collect();
    let ct = |e: u32| fit.ability_cycle(f, e);
    let ns = |e: u32| fit.ability_num_shots(f, e);
    let all = || ab.iter().copied().filter(|&e| ct(e) > 0.0).map(|e| (e, Cycles::single(ct(e), 0.0, INF, false))).collect::<Vec<_>>();
    if !factor {
        return all();
    }
    let limited: Vec<u32> = ab.iter().copied().filter(|&e| ns(e) > 0.0 && ct(e) > 0.0).collect();
    if limited.is_empty() {
        return all();
    }
    let valid: Vec<u32> = ab.iter().copied().filter(|&e| ct(e) > 0.0).collect();
    if valid.is_empty() {
        return vec![];
    }
    let mut most = limited[0];
    for &e in &limited {
        if ct(e) * ns(e) < ct(most) * ns(most) {
            most = e;
        }
    }
    let dur = ct(most) * ns(most);
    let mut until: Vec<(u32, f64, Option<f64>)> = vec![(most, ns(most), None)];
    for &e in &valid {
        if e == most {
            continue;
        }
        let full = float_unerr(dur / ct(e)).trunc();
        let mut extra = Some(float_unerr(dur - full * ct(e)));
        if extra == Some(0.0) {
            extra = None;
        }
        until.push((e, full, extra));
    }
    let get = |e: u32| until.iter().find(|x| x.0 == e).copied().unwrap();
    let mut refuel = f64::NEG_INFINITY;
    for &e in &valid {
        let (_, mut spent, extra) = get(e);
        if extra.is_some() {
            spent += 1.0;
        }
        refuel = refuel.max(fit.ability_reload(f, e, Some(spent)));
    }
    let mut out = Vec::new();
    for &e in &valid {
        let (_, regular, extra) = get(e);
        let mut seq = Vec::new();
        if let Some(x) = extra {
            if regular > 0.0 {
                seq.push((ct(e), 0.0, regular, false));
            }
            seq.push((x, refuel, 1.0, true));
        } else {
            if regular - 1.0 > 0.0 {
                seq.push((ct(e), 0.0, regular - 1.0, false));
            }
            seq.push((ct(e), refuel, 1.0, true));
        }
        out.push((e, Cycles { seq, repeat: INF }));
    }
    out
}

/// getCycleParametersPerEffectOptimizedDps
fn ability_cycles_opt(fit: &Fit, f: It, reload_override: Option<bool>) -> Vec<(u32, Cycles)> {
    let ab = fit.items[f].abilities.clone();
    let inf: Vec<(u32, Cycles)> = ab
        .iter()
        .filter(|a| fit.ability_num_shots(f, a.0) == 0.0 && fit.ability_cycle(f, a.0) > 0.0)
        .map(|a| (a.0, Cycles::single(fit.ability_cycle(f, a.0), 0.0, INF, false)))
        .collect();
    let rel = ability_cycles_reload(fit, f, reload_override.unwrap_or(fit.factor_reload));
    let tot = |c: &[(u32, Cycles)]| -> f64 {
        ab.iter()
            .filter_map(|&(e, on)| c.iter().find(|x| x.0 == e).map(|x| fit.ability_volley(f, e, on).total() / (x.1.average() / 1000.0)))
            .sum()
    };
    if tot(&inf) >= tot(&rel) { inf } else { rel }
}

fn fighter_deals(fit: &Fit, f: It) -> bool {
    fit.items[f].active && fit.items[f].amount > 0 && fit.items[f].abilities.iter().any(|&(e, on)| fit.ability_volley(f, e, on).total() > 0.0)
}

fn ability_deals(fit: &Fit, f: It, e: u32) -> bool {
    let p = fit.ability_prefix(e);
    fit.items[f].mad.contains(fit.a(&format!("{p}DamageMultiplier"))) || fit.items[f].charge != NONE
}

/// stats-panel dps (y=dps) or volley per dealer (time not set)
fn static_map(fit: &Fit, volley: bool) -> Vec<(Dk, Gd)> {
    let mut v = Vec::new();
    let def = Some(Spool { kind: SpoolType::SpoolScale, amount: 1.0 });
    for m in active_modules(fit) {
        if !mod_deals(fit, m) {
            continue;
        }
        let sp = fit.spool_opts(m, def);
        let p = mod_volley_params(fit, m, sp);
        let vol = p.iter().min_by(|a, b| a.0.partial_cmp(&b.0).unwrap()).map(|x| x.1.clone()).unwrap_or_default();
        let g = if volley {
            vol
        } else {
            match fit.cycle_avg(m, None) {
                None => Gd::default(),
                Some(a) if p.is_empty() || a == 0.0 => Gd::default(),
                Some(_) if fit.is_breacher(m) => vol,
                Some(a) => {
                    let mut s = Gd::default();
                    for (_, x) in &p {
                        s.add(x);
                    }
                    s.mul(1.0 / (a / 1000.0))
                }
            }
        };
        v.push((Dk::M(m), g));
    }
    for d in active_drones(fit) {
        let vol = gd_of(fit.drone_volley(d));
        if vol.raw_total() <= 0.0 {
            continue;
        }
        v.push((Dk::D(d), if volley { vol } else { gd_of(fit.drone_dps(d)) }));
    }
    for &f in &fit.fighters {
        if !fighter_deals(fit, f) {
            continue;
        }
        if volley {
            for &(e, on) in &fit.items[f].abilities {
                v.push((Dk::F(f, e), gd_of(fit.ability_volley(f, e, on))));
            }
        } else {
            let cyc = ability_cycles_opt(fit, f, None);
            for &(e, on) in &fit.items[f].abilities {
                if let Some(c) = cyc.iter().find(|x| x.0 == e) {
                    v.push((Dk::F(f, e), gd_of(fit.ability_volley(f, e, on)).mul(1.0 / (c.1.average() / 1000.0))));
                }
            }
        }
    }
    v
}

/// time cache per dealer: dps/volley change points and damage events
pub struct TimeData {
    pub dv: Vec<(Dk, Vec<(f64, Gd, Gd)>)>,
    pub dmg: Vec<(Dk, Vec<(f64, Gd)>)>,
}

fn time_data(fit: &Fit, max_t: f64) -> TimeData {
    let mut segs: Vec<(Dk, Vec<(f64, f64, Gd, Gd)>)> = Vec::new();
    let mut dmg: Vec<(Dk, Vec<(f64, Gd)>)> = Vec::new();
    fn add_dv(segs: &mut Vec<(Dk, Vec<(f64, f64, Gd, Gd)>)>, k: Dk, t0: f64, t1: f64, vols: &[Gd]) {
        if vols.is_empty() {
            return;
        }
        let mut s = Gd::default();
        for v in vols {
            s.add(v);
        }
        if s.raw_total() > 0.0 {
            let dps = s.mul(1.0 / (t1 - t0));
            let mut best = &vols[0];
            for v in vols {
                if v.raw_total() > best.raw_total() {
                    best = v;
                }
            }
            let best = best.clone();
            match segs.iter_mut().find(|x| x.0 == k) {
                Some(x) => x.1.push((t0, t1, dps, best)),
                None => segs.push((k, vec![(t0, t1, dps, best)])),
            }
        }
    }
    fn add_dmg(dmg: &mut Vec<(Dk, Vec<(f64, Gd)>)>, k: Dk, t: f64, g: &mut Gd) {
        if g.raw_total() == 0.0 {
            return;
        }
        for b in g.br.iter_mut() {
            b.0 += t;
        }
        let e = match dmg.iter_mut().position(|x| x.0 == k) {
            Some(i) => &mut dmg[i].1,
            None => {
                dmg.push((k, vec![]));
                &mut dmg.last_mut().unwrap().1
            }
        };
        match e.iter_mut().find(|x| x.0 == t) {
            Some(x) => x.1 = g.clone(),
            None => e.push((t, g.clone())),
        }
    }
    for m in active_modules(fit) {
        if !mod_deals(fit, m) {
            continue;
        }
        let br = fit.is_breacher(m);
        let cyc = if br { Some(Cycles::single(1000.0, 0.0, INF, false)) } else { module_cycles(fit, m, Some(true)) };
        let Some(cyc) = cyc else { continue };
        let mut t = 0.0;
        let mut nonstop = 0.0;
        for (ct, it, _) in cyc.iter() {
            let mut vols = Vec::new();
            let sp = Some(Spool { kind: SpoolType::Cycles, amount: nonstop });
            for (vt, mut v) in mod_volley_params(fit, m, sp) {
                let mut time = t + vt / 1000.0;
                if br {
                    time += 1.0;
                }
                add_dmg(&mut dmg, Dk::M(m), time, &mut v);
                vols.push(v);
                if br {
                    break;
                }
            }
            let (mut t0, mut t1) = (t, t + ct / 1000.0);
            if br {
                t0 += 1.0;
                t1 += 1.0;
            }
            add_dv(&mut segs, Dk::M(m), t0, t1, &vols);
            if it > 0.0 {
                nonstop = 0.0;
            } else {
                nonstop += 1.0;
            }
            if t > max_t {
                break;
            }
            t += ct / 1000.0 + it / 1000.0;
        }
    }
    for d in active_drones(fit) {
        let vol = gd_of(fit.drone_volley(d));
        if vol.raw_total() <= 0.0 {
            continue;
        }
        let ct0 = fit.drone_cycle(d);
        if ct0 == 0.0 {
            continue;
        }
        let mut t = 0.0;
        for (ct, it, _) in Cycles::single(ct0, 0.0, INF, false).iter() {
            let mut v = vol.clone();
            add_dmg(&mut dmg, Dk::D(d), t, &mut v);
            add_dv(&mut segs, Dk::D(d), t, t + ct / 1000.0, &[v]);
            if t > max_t {
                break;
            }
            t += ct / 1000.0 + it / 1000.0;
        }
    }
    for &f in &fit.fighters {
        if !fighter_deals(fit, f) {
            continue;
        }
        let cyc = ability_cycles_opt(fit, f, Some(true));
        for (e, c) in cyc {
            let Some(&(_, on)) = fit.items[f].abilities.iter().find(|a| a.0 == e) else { continue };
            let vol = gd_of(fit.ability_volley(f, e, on));
            let mut t = 0.0;
            for (ct, it, _) in c.iter() {
                let mut v = vol.clone();
                add_dmg(&mut dmg, Dk::F(f, e), t, &mut v);
                add_dv(&mut segs, Dk::F(f, e), t, t + ct / 1000.0, &[v]);
                if t > max_t {
                    break;
                }
                t += ct / 1000.0 + it / 1000.0;
            }
        }
    }
    // segments -> change points
    let mut dv = Vec::new();
    for (k, list) in segs {
        let mut pts: Vec<(f64, Gd, Gd)> = Vec::new();
        let mut prev: Option<(f64, Gd, Gd)> = None;
        for (t0, t1, dps, vol) in list {
            match &prev {
                None => pts.push((t0, dps.clone(), vol.clone())),
                Some((pe, pd, pv)) => {
                    if float_unerr(*pe) < float_unerr(t0) {
                        let pe = *pe;
                        match pts.iter_mut().find(|x| x.0 == pe) {
                            Some(x) => {
                                x.1 = Gd::default();
                                x.2 = Gd::default();
                            }
                            None => pts.push((pe, Gd::default(), Gd::default())),
                        }
                        pts.push((t0, dps.clone(), vol.clone()));
                    } else if !dps.eq(pd) || !vol.eq(pv) {
                        match pts.iter_mut().find(|x| x.0 == t0) {
                            Some(x) => {
                                x.1 = dps.clone();
                                x.2 = vol.clone();
                            }
                            None => pts.push((t0, dps.clone(), vol.clone())),
                        }
                    }
                }
            }
            prev = Some((t1, dps, vol));
        }
        dv.push((k, pts));
    }
    TimeData { dv, dmg }
}

/// time-cache data point per dealer (y: dps | volley | damage)
fn time_map(td: &TimeData, y: &str, t: f64) -> Vec<(Dk, Gd)> {
    let tu = float_unerr(t);
    let mut out = Vec::new();
    if y == "damage" {
        // global change times ≤ t; per dealer the cumulative sum of its events up to the latest such time
        let mut latest: Option<f64> = None;
        for (_, ev) in &td.dmg {
            for e in ev {
                if float_unerr(e.0) <= tu && latest.is_none_or(|l| e.0 > l) {
                    latest = Some(e.0);
                }
            }
        }
        let Some(l) = latest else { return out };
        for (k, ev) in &td.dmg {
            let mut evs: Vec<&(f64, Gd)> = ev.iter().filter(|e| e.0 <= l).collect();
            if evs.is_empty() {
                continue;
            }
            evs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            let mut s = Gd::default();
            for e in evs {
                s.add(&e.1);
            }
            out.push((*k, s));
        }
    } else {
        let mut latest: Option<f64> = None;
        for (_, pts) in &td.dv {
            for p in pts {
                if float_unerr(p.0) <= tu && latest.is_none_or(|l| p.0 > l) {
                    latest = Some(p.0);
                }
            }
        }
        let Some(l) = latest else { return out };
        for (k, pts) in &td.dv {
            let mut best: Option<&(f64, Gd, Gd)> = None;
            for p in pts {
                if p.0 <= l && best.is_none_or(|b| p.0 > b.0) {
                    best = Some(p);
                }
            }
            if let Some(p) = best {
                out.push((*k, if y == "dps" { p.1.clone() } else { p.2.clone() }));
            }
        }
    }
    out
}

// ---------------------------------------------------------------- target

pub struct Target<'a> {
    pub fit: Option<Fit<'a>>,
    /// target fit rebuilt with its MWD/MJD modules offline (Pyfa ignoreAfflictors=scrammables)
    pub unscrammed: Option<Fit<'a>>,
    pub scrammables: bool,
    pub resists: [f64; 4],
    pub hp: f64,
    pub radius: f64,
    pub sig: f64,
    pub vmax: f64,
    pub immune: bool,
}

fn layer_res(fit: &Fit, l: &str) -> [f64; 4] {
    let r = fit.resonances(l);
    [1.0 - r[0], 1.0 - r[1], 1.0 - r[2], 1.0 - r[3]]
}

fn auto_resists(fit: &Fit, req: &FitRequest) -> [f64; 4] {
    let s = fit.ship;
    let (hs, ha, hh) = (fit.g(s, "shieldCapacity"), fit.g(s, "armorHP"), fit.g(s, "hp"));
    let u = [25.0; 4];
    let (es, ea, eh) = (fit.effectivify(u, hs, "shield"), fit.effectivify(u, ha, "armor"), fit.effectivify(u, hh, "hull"));
    let te = es + ea + eh;
    let rf = |e: f64, h: f64| if h == 0.0 { 1.0 } else { e / h };
    let (rs, ra, rh) = (rf(es, hs), rf(ea, ha), rf(eh, hh));
    let st = fit.stats(req);
    let raw = st.get("defense").and_then(|v| v.get("tank")).and_then(|v| v.get("raw"));
    let g = |k: &str| match raw.and_then(|r| r.get(k)) {
        Some(crate::jv::Value::F(v)) => *v,
        Some(crate::jv::Value::U(v)) => *v as f64,
        Some(crate::jv::Value::I(v)) => *v as f64,
        _ => 0.0,
    };
    let (ts, ta, th, regen) = (g("shield_repair"), g("armor_repair"), g("hull_repair"), g("passive_shield"));
    let mut sc = [0.0f64; 3];
    sc[0] += 100.0 * (es / te).powf(1.5);
    sc[1] += 100.0 * (ea / te).powf(1.5);
    sc[2] += 100.0 * (eh / te).powf(1.5);
    let best = rs.max(ra).max(rh);
    sc[0] += 25.0 * (rs / best).powf(1.5);
    sc[1] += 25.0 * (ra / best).powf(1.5);
    sc[2] += 25.0 * (rh / best).powf(1.5);
    sc[0] += 10000.0 * ts * rs / te;
    sc[1] += 10000.0 * ta * ra / te;
    sc[2] += 10000.0 * th * rh / te;
    sc[0] += 5000.0 * regen * rs / te;
    let mx = sc[0].max(sc[1]).max(sc[2]);
    if mx == sc[0] {
        layer_res(fit, "shield")
    } else if mx == sc[1] {
        layer_res(fit, "armor")
    } else if mx == sc[2] {
        layer_res(fit, "hull")
    } else {
        [0.0; 4]
    }
}

fn weighted_resists(fit: &Fit) -> [f64; 4] {
    let s = fit.ship;
    let (hs, ha, hh) = (fit.g(s, "shieldCapacity"), fit.g(s, "armorHP"), fit.g(s, "hp"));
    let (rs, ra, rh) = (layer_res(fit, "shield"), layer_res(fit, "armor"), layer_res(fit, "hull"));
    let tot = hs + ha + hh;
    let mut out = [0.0; 4];
    for i in 0..4 {
        let e = hs / (1.0 - rs[i]) + ha / (1.0 - ra[i]) + hh / (1.0 - rh[i]);
        out[i] = 1.0 - tot / e;
    }
    out
}

fn is_scrammable(fit: &Fit, m: It) -> bool {
    ["moduleBonusMicrowarpdrive", "microJumpDrive", "microJumpPortalDrive"].iter().any(|e| has_effect(fit, m, e))
}

pub fn build_target<'a>(ds: &'a Dataset, req: &GraphRequest) -> Result<Target<'a>, GErr> {
    let t = req.target.clone().unwrap_or_default();
    if let Some(freq) = &t.fit {
        let fit = build_calc(ds, freq).map_err(|e| GErr { code: e.code, message: e.message, path: format!("/target/fit{}", e.path) })?;
        let s = fit.ship;
        let mode = t.resist_mode.clone().unwrap_or_else(|| "auto".into());
        let resists = match mode.as_str() {
            "shield" => layer_res(&fit, "shield"),
            "armor" => layer_res(&fit, "armor"),
            "hull" => layer_res(&fit, "hull"),
            "weighted_average" => weighted_resists(&fit),
            _ => auto_resists(&fit, freq),
        };
        let scr: Vec<usize> = active_modules(&fit).into_iter().filter(|&m| is_scrammable(&fit, m)).map(|m| fit.items[m].req_index).collect();
        let unscrammed = if scr.is_empty() {
            None
        } else {
            let mut r2: FitRequest = (**freq).clone();
            for i in &scr {
                if let Some(mm) = r2.modules.get_mut(*i) {
                    mm.state = Some(StateReq::Online);
                }
            }
            build_calc(ds, &r2).ok()
        };
        return Ok(Target {
            hp: fit.g(s, "shieldCapacity") + fit.g(s, "armorHP") + fit.g(s, "hp"),
            radius: fit.g(s, "radius"),
            sig: fit.g(s, "signatureRadius"),
            vmax: fit.g(s, "maxVelocity"),
            immune: fit.g(s, "disallowOffensiveModifiers") != 0.0,
            scrammables: !scr.is_empty(),
            resists,
            fit: Some(fit),
            unscrammed,
        });
    }
    let p = t.profile.unwrap_or_default();
    Ok(Target {
        fit: None,
        unscrammed: None,
        scrammables: false,
        resists: [p.em, p.thermal, p.kinetic, p.explosive],
        hp: match p.hp {
            Some(h) if h != -1.0 => h,
            _ => INF,
        },
        radius: p.radius.unwrap_or(0.0),
        sig: match p.signature_radius {
            Some(s) if s != -1.0 && s.is_finite() => s,
            _ => INF,
        },
        vmax: p.max_velocity.unwrap_or(0.0),
        immune: false,
    })
}

impl<'a> Target<'a> {
    /// getMaxVelocity / getSigRadius with extra multipliers [(mult, resist attr)] and optional scram
    fn ext(&self, name: &str, base: f64, extra: &[(f64, u32)], scram: bool) -> f64 {
        match &self.fit {
            None => {
                if extra.is_empty() {
                    base
                } else {
                    base * calc_multiplier(&[extra.iter().map(|x| x.0).collect()])
                }
            }
            Some(f) => {
                if extra.is_empty() && !(scram && self.scrammables) {
                    return base;
                }
                let fit = if scram && self.scrammables { self.unscrammed.as_ref().unwrap_or(f) } else { f };
                let s = fit.ship;
                let mults: Vec<f64> = extra
                    .iter()
                    .map(|&(m, r)| {
                        if r == 0 {
                            return m;
                        }
                        let rv = fit.items[s].mad.get(r, fit.ds);
                        match rv {
                            None => m,
                            Some(v) if v == 1.0 => m,
                            Some(v) => (m - 1.0) * v + 1.0,
                        }
                    })
                    .collect();
                let id = fit.a(name);
                fit.items[s].mad.get_extended(id, &mults, fit.ds)
            }
        }
    }
}

// ---------------------------------------------------------------- projected (webs / TPs)

struct ModProj {
    boost: f64,
    opt: f64,
    fall: f64,
    res: u32,
}
struct MobProj {
    boost: f64,
    opt: f64,
    fall: f64,
    res: u32,
    speed: f64,
    radius: f64,
}

fn res_id(fit: &Fit, it: It, effect: &str, prefix: Option<&str>) -> u32 {
    let e = fit.ds.effect_id(effect);
    if let Some(r) = fit.ds.effects.get(&e).and_then(|x| x.resistance_attr) {
        if r != 0 {
            return r;
        }
    }
    match prefix {
        Some(p) => {
            let r = fit.g(it, &format!("{p}ResistanceID")) as u32;
            if r != 0 { r } else { fit.g(it, &format!("{p}RemoteResistanceID")) as u32 }
        }
        None => fit.g(it, "remoteResistanceID") as u32,
    }
}

pub struct Proj {
    web_mods: Vec<ModProj>,
    tp_mods: Vec<ModProj>,
    web_drones: Vec<MobProj>,
    tp_drones: Vec<MobProj>,
    web_fighters: Vec<MobProj>,
    scram_range: Option<f64>,
}

pub fn proj_data(fit: &Fit) -> Proj {
    let mut p = Proj { web_mods: vec![], tp_mods: vec![], web_drones: vec![], tp_drones: vec![], web_fighters: vec![], scram_range: None };
    for m in active_modules(fit) {
        let (mr, fo) = (fit.max_range(m).unwrap_or(0.0), fit.falloff(m).unwrap_or(0.0));
        for e in ["remoteWebifierFalloff", "structureModuleEffectStasisWebifier"] {
            if has_effect(fit, m, e) {
                p.web_mods.push(ModProj { boost: fit.g(m, "speedFactor"), opt: mr, fall: fo, res: res_id(fit, m, e, None) });
            }
        }
        if has_effect(fit, m, "doomsdayAOEWeb") {
            p.web_mods.push(ModProj { boost: fit.g(m, "speedFactor"), opt: (mr + fit.g(m, "doomsdayAOERange")).max(0.0), fall: fo, res: res_id(fit, m, "doomsdayAOEWeb", None) });
        }
        for e in ["remoteTargetPaintFalloff", "structureModuleEffectTargetPainter"] {
            if has_effect(fit, m, e) {
                p.tp_mods.push(ModProj { boost: fit.g(m, "signatureRadiusBonus"), opt: mr, fall: fo, res: res_id(fit, m, e, None) });
            }
        }
        if has_effect(fit, m, "doomsdayAOEPaint") {
            p.tp_mods.push(ModProj { boost: fit.g(m, "signatureRadiusBonus"), opt: (mr + fit.g(m, "doomsdayAOERange")).max(0.0), fall: fo, res: res_id(fit, m, "doomsdayAOEPaint", None) });
        }
        let regular = ["warpScrambleBlockMWDWithNPCEffect", "structureWarpScrambleBlockMWDWithNPCEffect"].iter().any(|e| has_effect(fit, m, e)) && fit.g(m, "activationBlockedStrenght") != 0.0;
        let c = fit.items[m].charge;
        let hic = has_effect(fit, m, "warpDisruptSphere") && c != NONE && has_effect(fit, c, "shipModuleFocusedWarpScramblingScript");
        if regular || hic {
            p.scram_range = Some(p.scram_range.unwrap_or(0.0).max(fit.max_range(m).unwrap_or(0.0)));
        }
    }
    for d in active_drones(fit) {
        let n = fit.items[d].amount_active;
        let mk = |boost: f64, e: &str| MobProj {
            boost,
            opt: fit.drone_max_range(d).unwrap_or(0.0),
            fall: fit.drone_falloff(d).unwrap_or(0.0),
            res: res_id(fit, d, e, None),
            speed: fit.g(d, "maxVelocity"),
            radius: fit.g(d, "radius"),
        };
        if has_effect(fit, d, "remoteWebifierEntity") {
            for _ in 0..n {
                p.web_drones.push(mk(fit.g(d, "speedFactor"), "remoteWebifierEntity"));
            }
        }
        if has_effect(fit, d, "remoteTargetPaintEntity") {
            for _ in 0..n {
                p.tp_drones.push(mk(fit.g(d, "signatureRadiusBonus"), "remoteTargetPaintEntity"));
            }
        }
    }
    for (f, e) in active_abilities(fit) {
        if effect_name(fit, e) == "fighterAbilityStasisWebifier" {
            let pre = "fighterAbilityStasisWebifier";
            p.web_fighters.push(MobProj {
                boost: fit.g(f, &format!("{pre}SpeedPenalty")) * fit.items[f].amount as f64,
                opt: fit.g(f, &format!("{pre}OptimalRange")),
                fall: fit.g(f, &format!("{pre}FalloffRange")),
                res: res_id(fit, f, pre, Some(pre)),
                speed: fit.g(f, "maxVelocity"),
                radius: fit.g(f, "radius"),
            });
        }
    }
    p
}

struct Ctx<'r> {
    req: &'r GraphRequest,
}
impl Ctx<'_> {
    fn lock(&self, fit: &Fit, d: Option<f64>) -> bool {
        in_lock_range(fit, self.req.settings.ignore_lock_range, d)
    }
    fn dcr(&self, fit: &Fit, d: Option<f64>) -> bool {
        in_drone_range(fit, self.req.settings.ignore_drone_control_range, d)
    }
    fn mode(&self) -> &str {
        self.req.settings.mobile_drone_mode.as_str()
    }
}

fn scram_active(c: &Ctx, src: &Fit, p: &Proj, d: Option<f64>) -> bool {
    if !c.lock(src, d) {
        return false;
    }
    match p.scram_range {
        None => false,
        Some(r) => !matches!(d, Some(d) if d > r),
    }
}

fn tackled_speed(c: &Ctx, src: &Fit, tgt: &Target, p: &Proj, cur: f64, d: Option<f64>) -> f64 {
    if tgt.immune {
        return cur;
    }
    let vmax = tgt.vmax;
    if vmax == 0.0 {
        return vmax;
    }
    let lock = c.lock(src, d);
    let dcr = c.dcr(src, d);
    let ratio = cur / vmax;
    let scram = scram_active(c, src, p, d);
    let mut applied: Vec<(f64, u32)> = Vec::new();
    if lock {
        for w in &p.web_mods {
            let b = w.boost * calc_range_factor(w.opt, w.fall, d, true);
            if b != 0.0 {
                applied.push((1.0 + b / 100.0, w.res));
            }
        }
    }
    let mut max_t = tgt.ext("maxVelocity", vmax, &applied, scram);
    let mut cur_t = max_t * ratio;
    let mut mobile: Vec<&MobProj> = Vec::new();
    if lock {
        mobile.extend(p.web_fighters.iter());
    }
    if lock && dcr {
        mobile.extend(p.web_drones.iter());
    }
    let ar = src.g(src.ship, "radius");
    let (long, rest): (Vec<&MobProj>, Vec<&MobProj>) = mobile.into_iter().partition(|mw| match d {
        None => true,
        Some(d) => d <= mw.opt - ar + mw.radius,
    });
    if !long.is_empty() {
        for mw in &long {
            applied.push((1.0 + mw.boost / 100.0, mw.res));
        }
        max_t = tgt.ext("maxVelocity", vmax, &applied, scram);
        cur_t = max_t * ratio;
    }
    let mut rest = rest;
    while !rest.is_empty() {
        let fastest = rest.iter().map(|m| m.speed).fold(f64::NEG_INFINITY, f64::max);
        let (now, later): (Vec<&MobProj>, Vec<&MobProj>) = rest.into_iter().partition(|m| m.speed == fastest);
        for mw in now {
            let b = if (c.mode() == "auto" && mw.speed >= cur_t) || c.mode() == "follow_target" {
                mw.boost
            } else {
                mw.boost * calc_range_factor(mw.opt, mw.fall, d.map(|d| d + ar - mw.radius), true)
            };
            applied.push((1.0 + b / 100.0, mw.res));
        }
        rest = later;
        max_t = tgt.ext("maxVelocity", vmax, &applied, scram);
        cur_t = max_t * ratio;
    }
    float_unerr(cur_t)
}

fn sig_mult(c: &Ctx, src: &Fit, tgt: &Target, p: &Proj, tgt_speed: f64, d: Option<f64>) -> f64 {
    if tgt.immune {
        return 1.0;
    }
    let lock = c.lock(src, d);
    let dcr = c.dcr(src, d);
    let init = tgt.sig;
    let scram = scram_active(c, src, p, d);
    let mut applied: Vec<(f64, u32)> = Vec::new();
    if lock {
        for t in &p.tp_mods {
            let b = t.boost * calc_range_factor(t.opt, t.fall, d, true);
            if b != 0.0 {
                applied.push((1.0 + b / 100.0, t.res));
            }
        }
    }
    let mut mobile: Vec<&MobProj> = Vec::new();
    if lock && dcr {
        mobile.extend(p.tp_drones.iter());
    }
    let ar = src.g(src.ship, "radius");
    for m in mobile {
        let b = if (c.mode() == "auto" && m.speed >= tgt_speed) || c.mode() == "follow_target" {
            m.boost
        } else {
            m.boost * calc_range_factor(m.opt, m.fall, d.map(|d| d + ar - m.radius), true)
        };
        applied.push((1.0 + b / 100.0, m.res));
    }
    let modified = tgt.ext("signatureRadius", init, &applied, scram);
    if modified == INF && init == INF {
        return 1.0;
    }
    float_unerr(modified / init)
}

// ---------------------------------------------------------------- application

pub struct Mob {
    pub atk_speed: f64,
    pub atk_angle: f64,
    pub tgt_speed: f64,
    pub tgt_angle: f64,
    pub tgt_sig: f64,
}

fn turret_mult(cth: f64) -> f64 {
    let wc = cth.min(0.01);
    let wp = wc * 3.0;
    let nc = cth - wc;
    let np = if nc > 0.0 { nc * ((0.01 + cth) / 2.0 + 0.49) } else { 0.0 };
    np + wp
}

#[allow(clippy::too_many_arguments)]
fn turret_cth(atk_speed: f64, atk_angle: f64, atk_r: f64, opt: f64, fall: f64, track: f64, osig: f64, d: Option<f64>, tgt_speed: f64, tgt_angle: f64, tgt_r: f64, sig: f64) -> f64 {
    let ang = match d {
        None => 0.0,
        Some(d) => {
            let a = atk_angle * std::f64::consts::PI / 180.0;
            let t = tgt_angle * std::f64::consts::PI / 180.0;
            let ctc = atk_r + d + tgt_r;
            let ts = (atk_speed * a.sin() - tgt_speed * t.sin()).abs();
            if ctc == 0.0 {
                if ts == 0.0 { 0.0 } else { INF }
            } else {
                ts / ctc
            }
        }
    };
    let rf = calc_range_factor(opt, fall, d, false);
    let tf = 0.5f64.powf(((ang * osig) / (track * sig)).powi(2));
    rf * tf
}

fn missile_factor(er: f64, ev: f64, drf: f64, v: f64, sig: f64) -> f64 {
    let mut m: f64 = 1.0;
    if er > 0.0 {
        m = m.min(sig / er);
    }
    if v > 0.0 {
        m = m.min(((ev * sig) / (er * v)).powf(drf));
    }
    m
}

fn bomb_factor(er: f64, sig: f64) -> f64 {
    if er == 0.0 { 1.0 } else { (sig / er).min(1.0) }
}

fn dist_factor(fit: &Fit, m: It, d: Option<f64>) -> Option<f64> {
    let (lo, hi, ch) = fit.missile_range(m)?;
    Some(match d {
        None => 1.0,
        Some(d) if d <= lo => 1.0,
        Some(d) if d <= hi => ch,
        _ => 0.0,
    })
}

fn application(c: &Ctx, src: &Fit, tgt: &Target, k: Dk, d: Option<f64>, mo: &Mob) -> f64 {
    let lock = c.lock(src, d);
    let ar = src.g(src.ship, "radius");
    let v = match k {
        Dk::M(m) => {
            let gname = src.ds.group_name(src.items[m].t.group);
            let ch = src.items[m].charge;
            if has_effect(src, m, "ChainLightning") {
                if !lock {
                    0.0
                } else {
                    calc_range_factor(src.g(m, "maxRange"), 0.0, d, true)
                        * missile_factor(src.g(m, "aoeCloudSize"), src.g(m, "aoeVelocity"), src.g(m, "aoeDamageReductionFactor"), mo.tgt_speed, mo.tgt_sig)
                }
            } else if src.hardpoint(m) == 1 {
                if !lock {
                    0.0
                } else {
                    turret_mult(turret_cth(
                        mo.atk_speed,
                        mo.atk_angle,
                        ar,
                        src.max_range(m).unwrap_or(0.0),
                        src.falloff(m).unwrap_or(0.0),
                        src.g(m, "trackingSpeed"),
                        src.g(m, "optimalSigRadius"),
                        d,
                        mo.tgt_speed,
                        mo.tgt_angle,
                        tgt.radius,
                        mo.tgt_sig,
                    ))
                }
            } else if src.hardpoint(m) == 2 || src.items[m].t.id == 32461 {
                let fof = ch != NONE && has_effect(src, ch, "fofMissileLaunching");
                if !(lock || fof) {
                    0.0
                } else {
                    match dist_factor(src, m, d) {
                        None => 0.0,
                        Some(df) => df * missile_factor(src.g(ch, "aoeCloudSize"), src.g(ch, "aoeVelocity"), src.g(ch, "aoeDamageReductionFactor"), mo.tgt_speed, mo.tgt_sig),
                    }
                }
            } else if gname == "Smart Bomb" || gname == "Structure Area Denial Module" {
                match src.max_range(m) {
                    None => 0.0,
                    Some(r) => {
                        if matches!(d, Some(d) if d > r) { 0.0 } else { 1.0 }
                    }
                }
            } else if gname == "Missile Launcher Bomb" {
                match src.max_range(m) {
                    None => 0.0,
                    Some(r) => {
                        let blast = src.g(ch, "explosionRange");
                        let tr = tgt.radius;
                        if matches!(d, Some(d) if d < (r - ar - tr - blast).max(0.0)) || matches!(d, Some(d) if d > (r - ar + tr + blast).max(0.0)) {
                            0.0
                        } else {
                            bomb_factor(src.g(ch, "aoeCloudSize"), mo.tgt_sig)
                        }
                    }
                }
            } else if gname == "Structure Guided Bomb Launcher" {
                if !lock {
                    0.0
                } else {
                    match src.max_range(m) {
                        None => 0.0,
                        Some(r) => {
                            if matches!(d, Some(d) if d > r - ar) {
                                0.0
                            } else {
                                let er = src.g(ch, "aoeCloudSize");
                                if er == 0.0 { 1.0 } else { (mo.tgt_sig / er).min(1.0) }
                            }
                        }
                    }
                }
            } else if gname == "Super Weapon" || gname == "Structure Doomsday Weapon" {
                let st = ["superWeaponAmarr", "superWeaponCaldari", "superWeaponGallente", "superWeaponMinmatar"];
                if !lock && (st.iter().any(|e| has_effect(src, m, e)) || has_effect(src, m, "lightningWeapon")) {
                    0.0
                } else {
                    let r = src.max_range(m);
                    if matches!((d, r), (Some(d), Some(r)) if r != 0.0 && d > r) {
                        0.0
                    } else if st.iter().any(|e| has_effect(src, m, e)) && tgt.fit.as_ref().is_some_and(|tf| !tf.req_skill(tf.ship, 3344)) {
                        0.0
                    } else {
                        let ds = src.g(m, "signatureRadius");
                        if ds == 0.0 { 1.0 } else { (mo.tgt_sig / ds).min(1.0) }
                    }
                }
            } else if src.is_breacher(m) {
                if !lock {
                    0.0
                } else {
                    match dist_factor(src, m, d) {
                        None => 0.0,
                        Some(df) => df * tgt.fit.as_ref().map(|tf| tf.gd(tf.ship, "breacherPodDamageResistance", 1.0)).unwrap_or(1.0),
                    }
                }
            } else {
                0.0
            }
        }
        Dk::D(dr) => {
            if !lock || !c.dcr(src, d) {
                0.0
            } else {
                let ds = src.g(dr, "maxVelocity");
                let cth = if ds > 1.0 && ((c.mode() == "auto" && ds >= mo.tgt_speed) || c.mode() == "follow_target") {
                    1.0
                } else {
                    let rr = src.g(dr, "radius");
                    turret_cth(
                        mo.atk_speed.min(ds),
                        mo.atk_angle,
                        rr,
                        src.drone_max_range(dr).unwrap_or(0.0),
                        src.drone_falloff(dr).unwrap_or(0.0),
                        src.g(dr, "trackingSpeed"),
                        src.g(dr, "optimalSigRadius"),
                        d.map(|d| d + ar - rr),
                        mo.tgt_speed,
                        mo.tgt_angle,
                        tgt.radius,
                        mo.tgt_sig,
                    )
                };
                turret_mult(cth)
            }
        }
        Dk::F(f, e) => {
            let p = src.ability_prefix(e);
            if !(lock || p == "fighterAbilityLaunchBomb") {
                0.0
            } else if p == "fighterAbilityLaunchBomb" {
                bomb_factor(src.g(src.items[f].charge, "aoeCloudSize"), mo.tgt_sig)
            } else {
                let fs = src.g(f, "maxVelocity");
                let rf = if (c.mode() == "auto" && fs >= mo.tgt_speed) || c.mode() == "follow_target" {
                    1.0
                } else {
                    let mut opt = src.g(f, &format!("{p}RangeOptimal"));
                    if opt == 0.0 {
                        opt = src.g(f, &format!("{p}Range"));
                    }
                    calc_range_factor(opt, src.g(f, &format!("{p}RangeFalloff")), d.map(|d| d + ar - src.g(f, "radius")), true)
                };
                let opt_attr = |a: &str, b: &str| {
                    let id = src.a(a);
                    if id != 0 {
                        if let Some(v) = src.attr_opt(f, id) {
                            return v;
                        }
                    }
                    src.g(f, b)
                };
                let drf = opt_attr(&format!("{p}ReductionFactor"), &format!("{p}DamageReductionFactor"));
                let drs = opt_attr(&format!("{p}ReductionSensitivity"), &format!("{p}DamageReductionSensitivity"));
                let mf = missile_factor(src.g(f, &format!("{p}ExplosionRadius")), src.g(f, &format!("{p}ExplosionVelocity")), drf.ln() / drs.ln(), mo.tgt_speed, mo.tgt_sig);
                let mut res = 1.0;
                if let Some(tf) = &tgt.fit {
                    let rid = src.g(f, &format!("{p}ResistanceID")) as u32;
                    if rid != 0 {
                        res = tf.attr_opt(tf.ship, rid).unwrap_or(1.0);
                    }
                }
                rf * mf * res
            }
        }
    };
    float_unerr(v)
}

// ---------------------------------------------------------------- run

pub fn run(ds: &Dataset, req: &GraphRequest) -> Result<Vec<(String, Vec<Option<f64>>)>, GErr> {
    let src = build_calc(ds, &req.fit)?;
    let tgt = build_target(ds, req)?;
    let c = Ctx { req };
    let tgt_speed0 = req.p("tgt_speed_mps").unwrap_or_else(|| req.pd("tgt_speed_pct", 100.0) / 100.0 * tgt.vmax);
    let atk_speed = req.p("atk_speed_mps").unwrap_or_else(|| req.pd("atk_speed_pct", 0.0) / 100.0 * src.g(src.ship, "maxVelocity"));
    let atk_angle = req.pd("atk_angle_deg", 90.0);
    let tgt_angle = req.pd("tgt_angle_deg", 90.0);
    let ptime = req.p("time_s").map(|t| t.clamp(0.0, 2500.0));
    let pdist = req.p("distance_m");
    let axis = req.x.axis.as_str();
    let proj = if req.settings.apply_projected { Some(proj_data(&src)) } else { None };
    let res = if req.settings.ignore_resists { [0.0; 4] } else { tgt.resists };
    let keys_of = |m: &[(Dk, Gd)]| m.iter().map(|x| x.0).collect::<Vec<_>>();
    // time cache (once, up to the largest needed time)
    let need_t = if axis == "time_s" { req.x.values.iter().copied().filter(|&x| in_range(x, 0.0, 2500.0)).fold(None, |a: Option<f64>, x| Some(a.map_or(x, |a| a.max(x)))) } else { ptime };
    let td = need_t.map(|t| time_data(&src, t));
    let mut out = Vec::new();
    for y in &req.y {
        let mut vals = Vec::new();
        for &x in &req.x.values {
            if axis == "time_s" && !in_range(x, 0.0, 2500.0) {
                vals.push(None);
                continue;
            }
            let (d, t) = match axis {
                "distance_m" => (Some(x), ptime),
                "time_s" => (pdist, Some(x)),
                _ => (pdist, ptime),
            };
            if y == "damage" && t.is_none() {
                vals.push(None);
                continue;
            }
            let dmap = match (t, &td) {
                (Some(t), Some(td)) => time_map(td, y, t),
                _ => static_map(&src, y == "volley"),
            };
            let mut ts = if axis == "tgt_speed_mps" { x } else { tgt_speed0 };
            let mut sig = tgt.sig;
            if let Some(p) = &proj {
                ts = tackled_speed(&c, &src, &tgt, p, ts, d);
                let sm = sig_mult(&c, &src, &tgt, p, ts, d);
                sig = if axis == "tgt_sig_m" { x * sm } else { sig * sm };
            } else if axis == "tgt_sig_m" {
                sig = x;
            }
            let mo = Mob { atk_speed, atk_angle, tgt_speed: ts, tgt_angle, tgt_sig: sig };
            let _ = keys_of(&dmap);
            let mut total = Gd::default();
            for (k, g) in &dmap {
                let a = application(&c, &src, &tgt, *k, d, &mo);
                total.add(&g.mul(a));
            }
            vals.push(Some(total.total_vs(res, tgt.hp)));
        }
        out.push((y.clone(), vals));
    }
    Ok(out)
}

#[allow(dead_code)]
pub fn ability_deals_damage(fit: &Fit, f: It, e: u32) -> bool {
    ability_deals(fit, f, e)
}
