//! Unit tests for `eft` (kept out of the module source; compiled only for `cargo test`).
use super::*;

#[test]
fn mut_ref_strips_trailing_reference() {
    assert_eq!(mut_ref("Large Shield Extender II [3]"), ("Large Shield Extender II", Some(3)));
    assert_eq!(mut_ref("Large Shield Extender II [3]  "), ("Large Shield Extender II", Some(3)));
    assert_eq!(mut_ref("Damage Control II"), ("Damage Control II", None));
    assert_eq!(mut_ref("[Empty High slot]"), ("[Empty High slot]", None));
    assert_eq!(mut_ref("Thing [x]"), ("Thing [x]", None));
}

#[test]
fn py_float_matches_python_repr_of_float_unerr() {
    for (v, want) in [
        (0.30000000000000004, "0.3"),
        (123456.789, "123456.8"),
        (1234567890.0, "1234568000.0"),
        (-0.000123456789, "-0.0001234568"),
        (1.0000001e-5, "1e-05"),
        (2.5e16, "2.5e+16"),
        (12.0, "12.0"),
        (0.0, "0.0"),
        (f64::INFINITY, "inf"),
    ] {
        assert_eq!(py_float(v), want, "{v}");
    }
}

#[test]
fn drone_order_follows_pyfa_groups() {
    assert!(drone_order(Some(837)) < drone_order(Some(838)));
    assert!(drone_order(Some(839)) < drone_order(Some(911)));
    assert_eq!(drone_order(Some(1531)), drone_order(Some(837)));
    assert_eq!(drone_order(None), 12);
    assert_eq!(drone_order(Some(1)), 12);
}
