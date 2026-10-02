//! Unit tests for `hash` (kept out of the module source; compiled only for `cargo test`).
use super::*;
use std::hash::Hash;

fn h<T: Hash + ?Sized>(v: &T) -> u64 {
    let mut s = FxHasher::default();
    v.hash(&mut s);
    s.finish()
}

#[test]
fn integer_mixing_matches_fxhash() {
    assert_eq!(h(&0u32), 0);
    assert_eq!(h(&1u32), K.rotate_left(26));
    assert_ne!(h(&1u32), h(&2u32));
}

#[test]
fn strings_hash_by_content() {
    assert_eq!(h("Rifter"), h(&String::from("Rifter")));
    assert_ne!(h("Rifter"), h("rifter"));
    assert_ne!(h("ab"), h("ab\0"));
    let mut m: FxHashMap<String, u32> = FxHashMap::default();
    m.insert("Rifter".into(), 587);
    assert_eq!(m.get("Rifter"), Some(&587));
}
