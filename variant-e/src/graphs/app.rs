//! `application_profile` graph (Pyfa "Application Profile", fitApplicationProfile): best charge per distance for
//! the fit's dominant weapon group (turrets or launchers), with Pyfa's coarse transition scan and distance-sampled
//! projected (web / TP) cache.
use super::damage::{Ctx, build_target, proj_data, sig_mult, tackled_speed};
use super::{GErr, GraphRequest};
use crate::api::build_calc;
use crate::data::{Dataset, TypeInfo};
use crate::eos::cx::{Fit, It, calc_range_factor, NONE};

const INF: f64 = f64::INFINITY;

fn sample_step(max_d: f64) -> f64 {
    if max_d <= 0.0 || max_d.is_nan() {
        return 100.0;
    }
    let step = max_d / 300.0;
    if step <= 100.0 {
        return 100.0;
    }
    (step / 100.0).ceil() * 100.0
}

fn base(ds: &Dataset, t: &TypeInfo, name: &str) -> Option<f64> {
    let id = ds.attr_id(name);
    if id == 0 { None } else { t.attr(id) }
}
/// `charge.getAttribute(x) or d`
fn base_or(ds: &Dataset, t: &TypeInfo, name: &str, d: f64) -> f64 {
    match base(ds, t, name) {
        Some(v) if v != 0.0 => v,
        _ => d,
    }
}

/// metaGroup (1 T1, 2 T2, 4 faction) derived from the dataset (no metaGroupID there): meta level 5 = Tech II,
/// a variation parent = faction, else Tech I
fn meta_group(t: &TypeInfo) -> u32 {
    if t.meta_level == Some(5) {
        2
    } else if t.variation_parent.is_some() {
        4
    } else {
        1
    }
}

fn filter_quality<'a>(charges: Vec<&'a TypeInfo>, q: &str) -> Vec<&'a TypeInfo> {
    if q == "all" {
        return charges;
    }
    const NAVY: [&str; 4] = ["Imperial Navy ", "Republic Fleet ", "Caldari Navy ", "Federation Navy "];
    const CAPNAVY: [&str; 3] = ["Sansha ", "Arch Angel ", "Shadow "];
    charges
        .into_iter()
        .filter(|c| match meta_group(c) {
            1 => true,
            2 => q == "navy",
            4 if q == "navy" => {
                if c.name.ends_with(" XL") {
                    CAPNAVY.iter().any(|p| c.name.starts_with(p))
                } else {
                    NAVY.iter().any(|p| c.name.starts_with(p))
                }
            }
            _ => false,
        })
        .collect()
}

/// getValidChargesForModule: published charges of the module's charge groups passing isValidCharge
fn valid_charges<'a>(fit: &Fit<'a>, m: It) -> Vec<&'a TypeInfo> {
    let ds = fit.ds;
    let mut groups: Vec<u32> = Vec::new();
    for i in 0..5 {
        let g = fit.g(m, &format!("chargeGroup{i}"));
        if g != 0.0 {
            groups.push(g as u32);
        }
    }
    let cap = base(ds, fit.items[m].t, "capacity");
    let size = fit.g(m, "chargeSize");
    let mut out = Vec::new();
    for &g in &groups {
        for t in ds.types.in_group(g) {
            // dataset marks the Civilian charges unpublished; Pyfa's eve.db lists them as published
            if !(t.published || t.name.starts_with("Civilian ")) {
                continue;
            }
            if out.iter().any(|x: &&TypeInfo| x.id == t.id) {
                continue;
            }
            if let (Some(v), Some(c)) = (base(ds, t, "volume"), cap) {
                if v > c {
                    continue;
                }
            }
            if size > 0.0 && base(ds, t, "chargeSize").unwrap_or(0.0) != size {
                continue;
            }
            out.push(t);
        }
    }
    out
}

struct ProjCache {
    on: bool,
    base_speed: f64,
    base_sig: f64,
    pts: Vec<(f64, f64, f64)>,
}
impl ProjCache {
    fn at(&self, d: f64) -> (f64, f64) {
        if !self.on || self.pts.is_empty() {
            return (self.base_speed, self.base_sig);
        }
        let n = self.pts.len();
        let idx = self.pts.partition_point(|p| p.0 <= d) as i64 - 1;
        let idx = idx.max(0) as usize;
        if idx >= n - 1 {
            let p = self.pts[n - 1];
            return (p.1, p.2);
        }
        let (lo, hi) = (self.pts[idx], self.pts[idx + 1]);
        if d <= lo.0 {
            return (lo.1, lo.2);
        }
        let t = if hi.0 > lo.0 { (d - lo.0) / (hi.0 - lo.0) } else { 0.0 };
        let sp = lo.1 + t * (hi.1 - lo.1);
        let sg = if lo.2 == INF || hi.2 == INF { INF } else { lo.2 + t * (hi.2 - lo.2) };
        (sp, sg)
    }
}

struct TurretCd {
    id: u32,
    raw: f64,
    opt: f64,
    fall: f64,
    track: f64,
}
struct MissileCd {
    id: u32,
    raw: f64,
    raw_dps: f64,
    lo: f64,
    hi: f64,
    ch: f64,
    maxr: f64,
    er: f64,
    ev: f64,
    drf: f64,
    prio: u32,
}

struct Mobility {
    atk_speed: f64,
    atk_angle: f64,
    atk_r: f64,
    tgt_angle: f64,
    tgt_r: f64,
    tracking_none: bool,
}

fn turret_volley(cd: &TurretCd, d: f64, osig: f64, mo: &Mobility, pc: &ProjCache) -> f64 {
    let rf = if d <= cd.opt { 1.0 } else { calc_range_factor(cd.opt, cd.fall, Some(d), false) };
    let tf = if mo.tracking_none {
        1.0
    } else {
        let (ts, sig) = pc.at(d);
        let a = mo.atk_angle * std::f64::consts::PI / 180.0;
        let t = mo.tgt_angle * std::f64::consts::PI / 180.0;
        let ctc = mo.atk_r + d + mo.tgt_r;
        let tr = (mo.atk_speed * a.sin() - ts * t.sin()).abs();
        let ang = if ctc == 0.0 { if tr == 0.0 { 0.0 } else { INF } } else { tr / ctc };
        if cd.track <= 0.0 || sig <= 0.0 {
            0.0
        } else if ang <= 0.0 {
            1.0
        } else {
            0.5f64.powf(((ang * osig) / (cd.track * sig)).powi(2))
        }
    };
    let cth = rf * tf;
    let wc = cth.min(0.01);
    let nc = cth - wc;
    let np = if nc > 0.0 { nc * ((0.01 + cth) / 2.0 + 0.49) } else { 0.0 };
    cd.raw * (np + wc * 3.0)
}

fn missile_volley(cd: &MissileCd, d: f64, pc: &ProjCache) -> f64 {
    let rf = if d <= cd.lo {
        1.0
    } else if d <= cd.hi {
        cd.ch
    } else {
        0.0
    };
    if rf == 0.0 {
        return 0.0;
    }
    let (v, sig) = pc.at(d);
    let mut f: f64 = 1.0;
    if cd.er > 0.0 {
        f = f.min(sig / cd.er);
    }
    if v > 0.0 && cd.er > 0.0 {
        f = f.min(((cd.ev * sig) / (cd.er * v)).powf(cd.drf));
    }
    cd.raw * rf * f
}

/// coarse scan + 10 m bisection (Pyfa calculateTransitions); best(d) -> (volley, charge index or None)
fn transitions(max_d: f64, best: &dyn Fn(f64) -> (f64, Option<usize>), missile: bool) -> Vec<(f64, Option<usize>)> {
    let res = sample_step(max_d);
    let mut out = Vec::new();
    let (_, b0) = best(0.0);
    out.push((0.0, b0));
    let mut cur = b0;
    let mut d = res;
    while d <= max_d {
        let (mut bv, bi) = best(d);
        if bi != cur {
            let (mut lo, mut hi) = (d - res, d);
            while hi - lo > 10.0 {
                let mid = ((lo + hi) / 2.0).floor();
                if best(mid).1 == cur {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            bv = best(hi).0;
            out.push((hi, bi));
            cur = bi;
        }
        if missile && bv < 0.01 {
            out.push((d, None));
            break;
        }
        d += res;
    }
    out
}

fn pick(tr: &[(f64, Option<usize>)], d: f64) -> Option<usize> {
    if tr.is_empty() {
        return None;
    }
    let idx = (tr.partition_point(|t| t.0 <= d) as i64 - 1).max(0) as usize;
    tr[idx].1
}

#[allow(clippy::type_complexity)]
pub fn run(ds: &Dataset, req: &GraphRequest) -> Result<(Vec<(String, Vec<Option<f64>>)>, Vec<(String, Vec<Option<u32>>)>), GErr> {
    let src = build_calc(ds, &req.fit)?;
    let tgt = build_target(ds, req)?;
    let c = Ctx { req };
    let q = req.ps("ammo_quality").unwrap_or("all").to_string();
    let tgt_speed = req.p("tgt_speed_mps").unwrap_or_else(|| req.pd("tgt_speed_pct", 100.0) / 100.0 * tgt.vmax);
    let atk_speed = req.p("atk_speed_mps").unwrap_or_else(|| req.pd("atk_speed_pct", 0.0) / 100.0 * src.g(src.ship, "maxVelocity"));
    let resists = if req.settings.ignore_resists { None } else { Some(tgt.resists) };
    let ship_r = src.g(src.ship, "radius");
    let base_sig = tgt.sig;
    let mo = Mobility {
        atk_speed,
        atk_angle: req.pd("atk_angle_deg", 90.0),
        atk_r: ship_r,
        tgt_angle: req.pd("tgt_angle_deg", 90.0),
        tgt_r: tgt.radius,
        tracking_none: base_sig == 0.0,
    };
    // dominant weapon group
    let mods: Vec<It> = src.modules.iter().copied().filter(|&m| src.items[m].state >= crate::eos::cx::ACTIVE && src.g(m, "miningAmount") == 0.0).collect();
    let nt = mods.iter().filter(|&&m| src.hardpoint(m) == 1).count();
    let nl = mods.iter().filter(|&&m| src.hardpoint(m) == 2).count();
    let kind = if nt == 0 && nl == 0 { 0 } else if nt >= nl { 1 } else { 2 };
    // weapon groups by type: (first module, count)
    let mut groups: Vec<(It, usize)> = Vec::new();
    if kind != 0 {
        for &m in &mods {
            if src.hardpoint(m) != kind {
                continue;
            }
            match groups.iter_mut().find(|g| src.items[g.0].t.id == src.items[m].t.id) {
                Some(g) => g.1 += 1,
                None => groups.push((m, 1)),
            }
        }
    }
    struct TG {
        cds: Vec<TurretCd>,
        osig: f64,
        cyc: f64,
        n: usize,
        max_d: f64,
    }
    struct MG {
        cds: Vec<MissileCd>,
        cyc: f64,
        n: usize,
        max_d: f64,
    }
    let mut tgs: Vec<TG> = Vec::new();
    let mut mgs: Vec<MG> = Vec::new();
    let mut max_eff: f64 = 0.0;
    for &(m, n) in &groups {
        let Some(cyc) = src.cycle_avg(m, None) else { continue };
        let charges = filter_quality(valid_charges(&src, m), &q);
        if charges.is_empty() {
            continue;
        }
        let ch = src.items[m].charge;
        let loaded = if ch != NONE { Some(src.items[ch].t) } else { None };
        if kind == 1 {
            let nz = |v: f64, d: f64| if v != 0.0 { v } else { d };
            let mut opt = src.g(m, "maxRange");
            let mut fall = src.g(m, "falloff");
            let mut track = src.g(m, "trackingSpeed");
            let osig = src.g(m, "optimalSigRadius");
            let dmult = nz(src.g(m, "damageMultiplier"), 1.0);
            let mut skill = 1.0;
            if let Some(lt) = loaded {
                let (rm, fm, tm) = (base_or(ds, lt, "weaponRangeMultiplier", 1.0), base_or(ds, lt, "fallofMultiplier", 1.0), base_or(ds, lt, "trackingSpeedMultiplier", 1.0));
                opt /= rm;
                fall /= fm;
                track /= tm;
                let bd: f64 = ["emDamage", "thermalDamage", "kineticDamage", "explosiveDamage"].iter().map(|a| base_or(ds, lt, a, 0.0)).sum();
                if bd > 0.0 {
                    let md: f64 = ["emDamage", "thermalDamage", "kineticDamage", "explosiveDamage"].iter().map(|a| src.g(ch, a)).sum();
                    skill = md / bd;
                }
            }
            let longest = charges.iter().map(|c| base_or(ds, c, "weaponRangeMultiplier", 1.0)).fold(1.0f64, f64::max);
            max_eff = max_eff.max(((opt * longest) + fall * 3.1).trunc());
            let cds: Vec<TurretCd> = charges
                .iter()
                .map(|c| {
                    let dm = ["emDamage", "thermalDamage", "kineticDamage", "explosiveDamage"].map(|a| base_or(ds, c, a, 0.0));
                    let tot: f64 = match resists {
                        Some(r) => (0..4).map(|i| dm[i] * (1.0 - r[i])).sum(),
                        None => dm.iter().sum(),
                    };
                    TurretCd {
                        id: c.id,
                        raw: tot * skill * dmult,
                        opt: opt * base_or(ds, c, "weaponRangeMultiplier", 1.0),
                        fall: fall * base_or(ds, c, "fallofMultiplier", 1.0),
                        track: track * base_or(ds, c, "trackingSpeedMultiplier", 1.0),
                    }
                })
                .collect();
            let mo_ = cds.iter().map(|c| c.opt).fold(f64::NEG_INFINITY, f64::max);
            let mf = cds.iter().map(|c| c.fall).fold(f64::NEG_INFINITY, f64::max);
            tgs.push(TG { cds, osig, cyc, n, max_d: (mo_ + mf * 3.1).trunc() });
        } else {
            // multipliers the loaded charge receives (modified / base, or applied to a pre-assigned 1)
            let mult = |a: &str| -> f64 {
                let Some(lt) = loaded else { return 1.0 };
                let b = base_or(ds, lt, a, 0.0);
                if b > 0.0 {
                    return src.g(ch, a) / b;
                }
                let id = ds.attr_id(a);
                let v = src.items[ch].mad.get_preassigned(id, 1.0, ds);
                if v != 0.0 { v } else { 1.0 }
            };
            let dmg_m = ["emDamage", "thermalDamage", "kineticDamage", "explosiveDamage"].map(|a| mult(a));
            let (vm, em) = (mult("maxVelocity"), mult("explosionDelay"));
            let (erm, evm, drm) = (mult("aoeCloudSize"), mult("aoeVelocity"), mult("aoeDamageReductionFactor"));
            let lmult = {
                let v = src.g(m, "damageMultiplier");
                if v != 0.0 { v } else { 1.0 }
            };
            let mk = |res: Option<[f64; 4]>| -> Vec<MissileCd> {
                let mut v: Vec<MissileCd> = Vec::new();
                for c in &charges {
                    let bv = base_or(ds, c, "maxVelocity", 0.0);
                    let bd = base_or(ds, c, "explosionDelay", 0.0);
                    if bv <= 0.0 || bd <= 0.0 {
                        continue;
                    }
                    let mass = base_or(ds, c, "mass", 1.0);
                    let ag = base_or(ds, c, "agility", 1.0);
                    let vel = bv * vm;
                    let delay = bd * em;
                    let ft = delay / 1000.0 + ship_r / vel;
                    let (lt, ht) = (ft.floor(), ft.ceil());
                    let rng = |t: f64| {
                        let acc = t.min(mass * ag / 1e6);
                        vel / 2.0 * acc + vel * (t - acc)
                    };
                    let lo = (rng(lt) - ship_r).max(0.0);
                    let hi = (rng(ht) - ship_r).max(0.0);
                    let dm = ["emDamage", "thermalDamage", "kineticDamage", "explosiveDamage"].map(|a| base_or(ds, c, a, 0.0));
                    let d4 = [dm[0] * dmg_m[0], dm[1] * dmg_m[1], dm[2] * dmg_m[2], dm[3] * dmg_m[3]];
                    let tot = match res {
                        Some(r) => d4[0] * (1.0 - r[0]) + d4[1] * (1.0 - r[1]) + d4[2] * (1.0 - r[2]) + d4[3] * (1.0 - r[3]),
                        None => d4[0] + d4[1] + d4[2] + d4[3],
                    };
                    let raw = tot * lmult;
                    let ln = c.name.to_lowercase();
                    let prio = if ln.contains("mjolnir") {
                        0
                    } else if ln.contains("inferno") {
                        1
                    } else if ln.contains("scourge") {
                        2
                    } else if ln.contains("nova") {
                        3
                    } else {
                        99
                    };
                    v.push(MissileCd {
                        id: c.id,
                        raw,
                        raw_dps: if cyc > 0.0 { raw / (cyc / 1000.0) } else { 0.0 },
                        lo,
                        hi,
                        ch: ft - lt,
                        maxr: hi,
                        er: base_or(ds, c, "aoeCloudSize", 0.0) * erm,
                        ev: base_or(ds, c, "aoeVelocity", 0.0) * evm,
                        drf: base_or(ds, c, "aoeDamageReductionFactor", 1.0) * drm,
                        prio,
                    });
                }
                v.sort_by(|a, b| b.maxr.partial_cmp(&a.maxr).unwrap().then(b.raw_dps.partial_cmp(&a.raw_dps).unwrap()));
                v
            };
            let range_cds = mk(None);
            if range_cds.is_empty() {
                continue;
            }
            max_eff = max_eff.max(range_cds[0].maxr);
            let cds = mk(resists);
            if cds.is_empty() {
                continue;
            }
            let md = cds[0].maxr.trunc();
            mgs.push(MG { cds, cyc, n, max_d: md });
        }
    }
    // projected cache sampled every getSampleStep(max effective range)
    let mut pc = ProjCache { on: req.settings.apply_projected, base_speed: tgt_speed, base_sig, pts: vec![] };
    if pc.on && (!tgs.is_empty() || !mgs.is_empty()) {
        let p = proj_data(&src);
        let res = sample_step(max_eff);
        let mut d = 0.0;
        while d <= max_eff {
            let ts = tackled_speed(&c, &src, &tgt, &p, tgt_speed, Some(d));
            let sm = sig_mult(&c, &src, &tgt, &p, ts, Some(d));
            pc.pts.push((d, ts, base_sig * sm));
            d += res;
        }
    }
    // transitions per group
    let ttr: Vec<Vec<(f64, Option<usize>)>> = tgs
        .iter()
        .map(|g| {
            let best = |d: f64| -> (f64, Option<usize>) {
                let mut bv = 0.0;
                let mut bi = None;
                for (i, cd) in g.cds.iter().enumerate() {
                    let v = turret_volley(cd, d, g.osig, &mo, &pc);
                    if v > bv {
                        bv = v;
                        bi = Some(i);
                    }
                }
                (bv, bi)
            };
            transitions(g.max_d, &best, false)
        })
        .collect();
    let mtr: Vec<Vec<(f64, Option<usize>)>> = mgs
        .iter()
        .map(|g| {
            let best = |d: f64| -> (f64, Option<usize>) {
                let mut bv = 0.0;
                let mut bi = None;
                let mut bp = 99;
                for (i, cd) in g.cds.iter().enumerate() {
                    let v = missile_volley(cd, d, &pc);
                    if v > bv || (v == bv && v > 0.0 && cd.prio < bp) {
                        bv = v;
                        bi = Some(i);
                        bp = cd.prio;
                    }
                }
                (bv, bi)
            };
            transitions(g.max_d, &best, true)
        })
        .collect();
    let mut vals_out = Vec::new();
    let mut ids_out = Vec::new();
    for y in &req.y {
        let mut vals = Vec::new();
        let mut ids = Vec::new();
        for &x in &req.x.values {
            let mut tot = 0.0;
            let mut id: Option<u32> = None;
            let mut any = false;
            for (g, tr) in tgs.iter().zip(&ttr) {
                any = true;
                // turret: transitions always resolve to some index (0 when nothing hits)
                let i = pick(tr, x).unwrap_or(0);
                let cd = &g.cds[i];
                let v = turret_volley(cd, x, g.osig, &mo, &pc);
                let named = pick(tr, x).is_some();
                tot += if y == "dps" { if g.cyc > 0.0 { v / (g.cyc / 1000.0) } else { 0.0 } } else { v } * g.n as f64;
                if id.is_none() && named {
                    id = Some(cd.id);
                }
            }
            for (g, tr) in mgs.iter().zip(&mtr) {
                any = true;
                let Some(i) = pick(tr, x) else { continue };
                let cd = &g.cds[i];
                let v = missile_volley(cd, x, &pc);
                tot += if y == "dps" { if g.cyc > 0.0 { v / (g.cyc / 1000.0) } else { 0.0 } } else { v } * g.n as f64;
                if id.is_none() {
                    id = Some(cd.id);
                }
            }
            let _ = any;
            vals.push(Some(tot));
            ids.push(id);
        }
        vals_out.push((y.clone(), vals));
        ids_out.push((format!("{y}_charge_type_id"), ids));
    }
    Ok((vals_out, ids_out))
}
