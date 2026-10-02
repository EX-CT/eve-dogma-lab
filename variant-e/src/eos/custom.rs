//! Hand ports of the few Pyfa handlers the transpiler does not express (GPL-3.0-or-later, from Pyfa
//! eos/effects.py). Only handlers that affect the local fit are ported; projected cap/RR/ECM bookkeeping
//! (`fit.addDrain`, `fit._armorRr`, `fit.addProjectedEcm`) is recorded where the stats need it.
use super::cx::*;

pub fn run(cx: &mut Cx, eid: u32, me: It) -> bool {
    match eid {
        101 => use_missiles(cx, me),
        1615 => {
            // shipAdvancedSpaceshipCommandAgilityBonus
            let skill = cx.ds.type_by_name.get("Advanced Spaceship Command").copied().unwrap_or(0);
            let sk = cx.skill_by_type.get(&skill).copied().unwrap_or(NONE);
            let v = cx.attr(sk, cx.ds.attr_id("agilityBonus"));
            let s = cx.ship;
            cx.op(s, Op::Boost, cx.ds.attr_id("agility"), v, O { skill, kw: true, ..O::default() });
        }
        4928 => rah(cx, me),
        6197 => {
            // energyNosferatuFalloff (local part): capacitorNeed forced to -powerTransferAmount
            let amount = cx.attr(me, cx.ds.attr_id("powerTransferAmount"));
            if !cx.ctx(Ctx::Projected) && cx.ctx(Ctx::Module) {
                cx.op(me, Op::Force, cx.ds.attr_id("capacitorNeed"), -amount, O { kw: true, ..O::default() });
            }
        }
        6672 => {
            // structureCombatRigSecurityModification
            let name = match cx.sys_sec {
                0 => "hiSecModifier",
                1 => "lowSecModifier",
                _ => "nullSecModifier",
            };
            let m = cx.attr(me, cx.ds.attr_id(name));
            for a in [
                "structureRigDoomsdayDamageLossTargetBonus",
                "structureRigScanResBonus",
                "structureRigPDRangeBonus",
                "structureRigPDCapUseBonus",
                "structureRigMissileExploVeloBonus",
                "structureRigMissileVelocityBonus",
                "structureRigEwarOptimalBonus",
                "structureRigEwarFalloffBonus",
                "structureRigEwarCapUseBonus",
                "structureRigMissileExplosionRadiusBonus",
                "structureRigMaxTargetRangeBonus",
            ] {
                let id = cx.ds.attr_id(a);
                cx.op(me, Op::Multiply, id, m, O { kw: true, ..O::default() });
            }
        }
        _ => return false,
    }
    true
}

fn use_missiles(cx: &mut Cx, me: It) {
    cx.set_reload_time(me, 10000.0);
    if !cx.ctx(Ctx::Projected) {
        return;
    }
    let g = cx.ds.group_name(cx.group_id(me));
    if g == "Interdiction Sphere Launcher" {
        let sf = cx.charge_attr(me, cx.ds.attr_id("speedFactor"));
        if sf != 0.0 {
            let s = cx.ship;
            cx.op(s, Op::Boost, cx.ds.attr_id("maxVelocity"), sf, O { kw: true, ..O::default() });
        }
    }
}

const ARMOR_RES: [&str; 4] = ["armorEmDamageResonance", "armorThermalDamageResonance", "armorKineticDamageResonance", "armorExplosiveDamageResonance"];

/// adaptiveArmorHardener (runTime late)
fn rah(cx: &mut Cx, me: It) {
    let ids: Vec<u32> = ARMOR_RES.iter().map(|n| cx.ds.attr_id(n)).collect();
    let ship = cx.ship;
    let pen = O { stack: true, group: 1, ..O::default() }; // penaltyGroup='preMul'
    let Some(dp) = cx.damage_pattern else {
        // 'disable': unadapted resonances
        for &a in &ids {
            let v = cx.attr(me, a);
            cx.op(ship, Op::Multiply, a, v, pen);
        }
        return;
    };
    let base: Vec<f64> = (0..4).map(|k| dp[k] * cx.attr(ship, ids[k])).collect();
    let shift = cx.attr(me, cx.ds.attr_id("resistanceShiftAmount")) / 100.0;
    let mut res: Vec<f64> = ids.iter().map(|&a| cx.attr(me, a)).collect();
    let mut cycles: Vec<[f64; 4]> = Vec::new();
    let mut loop_start: isize = -20;
    for _ in 0..50 {
        let mut t: Vec<(usize, f64, f64)> = [0usize, 3, 2, 1].iter().map(|&k| (k, base[k] * res[k], res[k])).collect();
        t.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let (c0, c1, c2, c3);
        if t[2].1 == 0.0 {
            c0 = 1.0 - t[0].2;
            c1 = 1.0 - t[1].2;
            c2 = 1.0 - t[2].2;
            c3 = -(c0 + c1 + c2);
        } else if t[1].1 == 0.0 {
            c0 = 1.0 - t[0].2;
            c1 = 1.0 - t[1].2;
            c2 = -(c0 + c1) / 2.0;
            c3 = -(c0 + c1) / 2.0;
        } else {
            c0 = shift.min(1.0 - t[0].2);
            c1 = shift.min(1.0 - t[1].2);
            c2 = -(c0 + c1) / 2.0;
            c3 = -(c0 + c1) / 2.0;
        }
        res[t[0].0] = t[0].2 + c0;
        res[t[1].0] = t[1].2 + c1;
        res[t[2].0] = t[2].2 + c2;
        res[t[3].0] = t[3].2 + c3;
        for (i, v) in cycles.iter().enumerate() {
            if (0..4).all(|k| (res[k] - v[k]).abs() <= 1e-6) {
                loop_start = i as isize;
                break;
            }
        }
        if loop_start >= 0 {
            break;
        }
        cycles.push([res[0], res[1], res[2], res[3]]);
    }
    // python cycleList[loopStart:] (negative = from the end)
    let n = cycles.len() as isize;
    let start = if loop_start < 0 { (n + loop_start).max(0) } else { loop_start } as usize;
    let lp = &cycles[start..];
    let mut avg = [0.0f64; 4];
    for c in lp {
        for k in 0..4 {
            avg[k] += c[k];
        }
    }
    for k in 0..4 {
        avg[k] = py_round3(avg[k] / lp.len() as f64);
    }
    for k in 0..4 {
        let cur = cx.attr(me, ids[k]);
        cx.op(me, Op::Increase, ids[k], avg[k] - cur, O::default());
        cx.op(ship, Op::Multiply, ids[k], avg[k], O { kw: true, ..pen });
    }
}

pub fn py_round3(v: f64) -> f64 {
    format!("{:.3}", v).parse().unwrap_or(v)
}
