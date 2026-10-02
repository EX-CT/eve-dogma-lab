//! Unit tests for `engine` (kept out of the module source; compiled only for `cargo test`).
use super::*;

#[test]
fn stacking_penalty_strongest_first() {
    // single modifier: full strength
    let mut v = 100.0;
    penalize(&mut v, &mut [1.1]);
    assert!((v - 110.0).abs() < 1e-9);
    // order-independent; the weaker one is penalised by exp(-1/2.67^2)
    let (mut a, mut b) = (100.0, 100.0);
    penalize(&mut a, &mut [1.1, 1.2]);
    penalize(&mut b, &mut [1.2, 1.1]);
    assert_eq!(a, b);
    let k = (-1.0f64 / 7.1289).exp();
    assert!((a - 100.0 * 1.2 * (1.0 + 0.1 * k)).abs() < 1e-9);
    // reductions rank by distance from 1 too
    let mut c = 100.0;
    penalize(&mut c, &mut [1.05, 0.8]);
    assert!((c - 100.0 * 0.8 * (1.0 + 0.05 * k)).abs() < 1e-9);
    let mut e = 7.0;
    penalize(&mut e, &mut []);
    assert_eq!(e, 7.0);
}

#[test]
fn skill_range_returns_all_entries_of_one_skill() {
    let v = [(1, 10), (3, 30), (3, 31), (3, 32), (7, 70)];
    let mut out = Vec::new();
    skill_range(&v, 3, &mut out);
    assert_eq!(out, [30, 31, 32]);
    out.clear();
    skill_range(&v, 4, &mut out);
    assert!(out.is_empty());
    skill_range(&v, 7, &mut out);
    assert_eq!(out, [70]);
}
