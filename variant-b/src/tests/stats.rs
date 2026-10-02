//! Unit tests for `stats` (kept out of the module source; compiled only for `cargo test`).
use super::*;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12 * b.abs().max(1.0)
}

#[test]
fn range_factor_optimal_falloff() {
    assert_eq!(range_factor(1000.0, 500.0, None, false), 1.0);
    assert_eq!(range_factor(1000.0, 500.0, Some(800.0), false), 1.0);
    assert!(close(range_factor(1000.0, 500.0, Some(1500.0), false), 0.5));
    assert!(close(range_factor(1000.0, 500.0, Some(2000.0), false), 0.0625));
    assert!(range_factor(1000.0, 500.0, Some(2600.0), false) > 0.0);
    assert_eq!(range_factor(1000.0, 500.0, Some(2600.0), true), 0.0);
    assert_eq!(range_factor(1000.0, 0.0, Some(1000.0), false), 1.0);
    assert_eq!(range_factor(1000.0, 0.0, Some(1000.1), false), 0.0);
}

#[test]
fn lock_time_formula_and_cap() {
    assert!(close(lock_time(100.0, 100.0).unwrap(), 14.248854624310974));
    assert_eq!(lock_time(1.0, 0.01), Some(1800.0));
    assert_eq!(lock_time(0.0, 100.0), None);
    assert_eq!(lock_time(100.0, 0.0), None);
}

#[test]
fn spoolup_kinds() {
    let s = |kind, amount| Spool { kind, amount };
    assert_eq!(spoolup(0.0, 0.1, 4.0, s(SpoolType::Cycles, 3.0)), (0.0, 0.0, 0.0));
    let (v, c, t) = spoolup(1.0, 0.1, 4.0, s(SpoolType::Cycles, 3.0));
    assert!(close(v, 0.3) && c == 3.0 && t == 12.0);
    let (v, c, _) = spoolup(1.0, 0.1, 4.0, s(SpoolType::SpoolScale, 1.0));
    assert!(close(v, 1.0) && c == 10.0);
    let (v, c, _) = spoolup(1.0, 0.1, 4.0, s(SpoolType::CycleScale, 0.5));
    assert!(close(v, 0.5) && c == 5.0);
    let (_, c, _) = spoolup(1.0, 0.1, 4.0, s(SpoolType::Time, 21.0));
    assert_eq!(c, 5.0);
    // never more than the cap, never more cycles than needed to reach it
    let (v, c, _) = spoolup(1.0, 0.1, 4.0, s(SpoolType::Cycles, 99.0));
    assert!(close(v, 1.0) && c == 10.0);
}

#[test]
fn float_unerr7_keeps_seven_significant_digits() {
    assert_eq!(float_unerr7(0.30000000000000004), 0.3);
    assert_eq!(float_unerr7(123456.789), 123456.8);
    assert_eq!(float_unerr7(1234567890.0), 1234568000.0);
    assert_eq!(float_unerr7(-0.000123456789), -0.0001234568);
    assert_eq!(float_unerr7(0.0), 0.0);
    assert!(float_unerr7(f64::NAN).is_nan());
}

#[test]
fn py_round2_matches_python_round() {
    // binary value decides ties exactly like CPython's round()
    for (v, want) in [(2.675, 2.67), (0.125, 0.12), (0.375, 0.38), (1.005, 1.0), (-0.125, -0.12), (3.14159, 3.14), (-2.5, -2.5), (1e20, 1e20)] {
        assert_eq!(py_round2(v), want, "{v}");
    }
    assert!(py_round2(f64::INFINITY).is_infinite());
}

#[test]
fn write_rounded_is_tidy_compact_json() {
    let v = json!({"b": [1, -2, 3.5, 0.1234567891, 1e300, null, true], "a": {"z": 1.0, "y": -0.0, "s": "q\"\n"}, "c": []});
    let s = write_rounded(&v);
    assert_eq!(s, serde_json::to_string(&tidy(v)).unwrap());
    assert!(s.contains("0.123457"), "{s}");
    assert!(s.starts_with("{\"a\":"), "keys are sorted: {s}");
}

#[test]
fn dmg_profile_math() {
    let d = Dmg { em: 1.0, th: 2.0, ki: 3.0, ex: 4.0 };
    assert_eq!(d.total(), 10.0);
    assert_eq!(d.scale(0.5).total(), 5.0);
    let r = Resists { em: 0.5, thermal: 0.0, kinetic: 1.0, explosive: 0.25 };
    assert!(close(d.vs(&r), 0.5 + 2.0 + 0.0 + 3.0));
}
