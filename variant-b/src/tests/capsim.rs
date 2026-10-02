//! Unit tests for `capsim` (kept out of the module source; compiled only for `cargo test`).
use super::*;

fn same(a: &CapResult, b: &CapResult) -> bool {
    a.stable == b.stable
        && a.stable_low.to_bits() == b.stable_low.to_bits()
        && a.stable_high.to_bits() == b.stable_high.to_bits()
        && a.t_s.to_bits() == b.t_s.to_bits()
        && a.depletes_in_s.map(f64::to_bits) == b.depletes_in_s.map(f64::to_bits)
        // the fast path updates the top event in place like eve-dogma-rs (one sift-down), the reference pops and
        // pushes: same pop order, different final heap layout, so EVE's sum over the heap may differ in the last bit
        && ((a.eve_stable - b.eve_stable).abs() <= 1e-12 * a.eve_stable.abs().max(1.0))
        && a.iterations == b.iterations
}

#[test]
fn fast_matches_reference_randomised() {
    let mut x: u64 = 0x9E3779B97F4A7C15;
    let mut rnd = |n: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x % n
    };
    for case in 0..3000 {
        let nd = 1 + rnd(9) as usize;
        let mut drains = Vec::new();
        for _ in 0..nd {
            let dup = !drains.is_empty() && rnd(3) == 0;
            if dup {
                let d: Drain = drains[rnd(drains.len() as u64) as usize];
                drains.push(d);
                continue;
            }
            let inj = rnd(6) == 0;
            let durs = [1000.0, 2000.0, 2500.0, 3000.0, 4500.0, 5000.0, 6000.0, 10000.0, 12000.0, 3333.0];
            let duration = durs[rnd(durs.len() as u64) as usize] + if rnd(4) == 0 { rnd(997) as f64 } else { 0.0 };
            let cap_need = if inj { -((50 + rnd(800)) as f64) } else { (rnd(400) as f64) * 0.5 + if rnd(5) == 0 { -10.0 } else { 0.0 } };
            let clip = if inj || rnd(4) == 0 { 1 + rnd(9) as u32 } else { 0 };
            drains.push(Drain { duration, cap_need, clip_size: clip, reload_ms: (rnd(3) * 5000) as f64, is_injector: inj, disable_stagger: rnd(5) == 0 });
        }
        let cap = (200 + rnd(6000)) as f64;
        let rr = (60_000 + rnd(900_000)) as f64;
        let reload = rnd(2) == 0;
        let stagger = rnd(3) != 0;
        let tmax = [6.0 * 3600.0 * 1000.0, 600_000.0][rnd(2) as usize];
        let a = simulate_ref(cap, rr, &drains, 1.0, reload, stagger, tmax);
        let b = simulate_fast(cap, rr, &drains, 1.0, reload, stagger, tmax);
        assert!(same(&a, &b), "case {case}: {drains:?} cap {cap} rr {rr} reload {reload} stagger {stagger}\n{a:?}\n{b:?}");
    }
}

fn drain(duration: f64, cap_need: f64) -> Drain {
    Drain { duration, cap_need, clip_size: 0, reload_ms: 0.0, is_injector: false, disable_stagger: false }
}

#[test]
fn no_drains_is_stable_at_full() {
    let r = simulate(500.0, 200_000.0, &[], 1.0, false, true, 600_000.0);
    assert!(r.stable);
    assert_eq!(r.depletes_in_s, None);
    assert!((r.stable_low - 1.0).abs() < 1e-9, "{r:?}");
}

#[test]
fn light_drain_is_stable_heavy_drain_runs_out() {
    // peak recharge of 500 GJ / 200 s is 2.5 * 500 / 200 = 6.25 GJ/s
    let light = simulate(500.0, 200_000.0, &[drain(5000.0, 10.0)], 1.0, false, true, 3_600_000.0);
    assert!(light.stable && light.stable_low > 0.25 && light.stable_low < 1.0, "{light:?}");
    let heavy = simulate(500.0, 200_000.0, &[drain(1000.0, 50.0)], 1.0, false, true, 3_600_000.0);
    assert!(!heavy.stable, "{heavy:?}");
    let t = heavy.depletes_in_s.expect("runs out");
    // 50 GJ/s use against at most 6.25 GJ/s recharge: between 500/50 and 500/(50-6.25) seconds
    assert!(t > 9.0 && t < 12.0, "{t}");
}

#[test]
fn injector_extends_cap_life() {
    let base = [drain(1000.0, 50.0)];
    let mut with_inj = base.to_vec();
    with_inj.push(Drain { duration: 12_000.0, cap_need: -400.0, clip_size: 1, reload_ms: 10_000.0, is_injector: true, disable_stagger: false });
    let a = simulate(500.0, 200_000.0, &base, 1.0, true, true, 3_600_000.0);
    let b = simulate(500.0, 200_000.0, &with_inj, 1.0, true, true, 3_600_000.0);
    let (ta, tb) = (a.depletes_in_s.unwrap(), b.depletes_in_s.unwrap_or(f64::INFINITY));
    assert!(tb > ta, "{ta} vs {tb}");
}
