//! Unit tests for `out` (the output document and its writer); compiled only for `cargo test`.
use super::*;

fn obj(kv: Vec<(&'static str, J)>) -> J {
    let mut o = Obj::new();
    for (k, v) in kv {
        o.insert(k, v);
    }
    J::Object(o)
}

#[test]
fn keys_are_written_sorted() {
    let j = obj(vec![("b", J::U(2)), ("a", J::U(1)), ("ab", J::U(3)), ("B", J::U(0))]);
    assert_eq!(j.to_string(), r#"{"B":0,"a":1,"ab":3,"b":2}"#);
}

#[test]
fn write_owned_matches_write() {
    let mk = || obj(vec![("z", J::Array(vec![obj(vec![("y", J::F(1.5)), ("x", J::Null)])])), ("a", J::Bool(true))]);
    assert_eq!(mk().to_string(), mk().into_string());
}

#[test]
fn key_cmp_is_str_order() {
    let keys = ["", "a", "aa", "ab", "b", "B", "_x", "a_b", "dps", "drones", "\u{4e2d}", "zz"];
    for x in keys {
        for y in keys {
            assert_eq!(key_cmp(x, y), x.cmp(y), "{x:?} vs {y:?}");
        }
    }
}

#[test]
fn floats_round_to_six_decimals_and_non_finite_is_null() {
    assert_eq!(J::F(1.23456789).to_string(), "1.234568");
    assert_eq!(J::F(2.0).to_string(), "2.0");
    assert_eq!(J::F(f64::NAN).to_string(), "null");
    assert_eq!(J::F(f64::INFINITY).to_string(), "null");
}

#[test]
fn integers_and_literals() {
    assert_eq!(J::I(-7).to_string(), "-7");
    assert_eq!(J::U(u64::MAX).to_string(), "18446744073709551615");
    assert_eq!(J::Null.to_string(), "null");
    assert_eq!(J::Bool(false).to_string(), "false");
}

#[test]
fn strings_are_escaped_like_serde_json() {
    let s = "q\"b\\n\nt\t\u{1}\u{4e2d}";
    assert_eq!(J::Str(s.into()).to_string(), serde_json::to_string(s).unwrap());
}

#[test]
fn owned_keys_are_escaped() {
    let mut o = Obj::new();
    o.insert(String::from("a\"b"), J::U(1));
    assert_eq!(J::Object(o).to_string(), r#"{"a\"b":1}"#);
}

#[test]
fn insert_replaces_existing_key() {
    let mut o = Obj::new();
    assert!(o.insert("k", J::U(1)).is_none());
    assert_eq!(o.insert("k", J::U(2)), Some(J::U(1)));
    assert_eq!(o.get("k"), Some(&J::U(2)));
    assert_eq!(J::Object(o).to_string(), r#"{"k":2}"#);
}

#[test]
fn own_moves_the_document() {
    let inner = obj(vec![("x", J::U(1))]);
    let j = crate::jv!({"a": own(inner), "b": own(vec![J::U(1), J::U(2)])});
    assert_eq!(j.to_string(), r#"{"a":{"x":1},"b":[1,2]}"#);
}

#[test]
fn jv_macro_matches_serde_json_macro() {
    let n: Option<u32> = None;
    let j = crate::jv!({"n": n, "v": [1, 2.5, true, null], "s": "x", "o": {"k": -3}});
    let v = serde_json::json!({"n": n, "v": [1, 2.5, true, null], "s": "x", "o": {"k": -3}});
    assert_eq!(j.to_value(), v);
    assert_eq!(j.to_string(), serde_json::to_string(&v).unwrap());
}
