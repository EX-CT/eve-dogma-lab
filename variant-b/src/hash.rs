//! Fx-style hasher (add, multiply; rotate on finish) for the engine's small integer-keyed maps. In-tree instead
//! of the rustc-hash crate: same integer mixing, so u32-keyed maps behave exactly as before. Not DoS-resistant,
//! which is fine: keys come from the dataset and requests, never from an untrusted bulk source.
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

const K: u64 = 0xf135_7aea_2e62_a9c5;

#[derive(Default, Clone, Copy)]
pub struct FxHasher {
    h: u64,
}

impl FxHasher {
    #[inline]
    fn add(&mut self, w: u64) {
        self.h = self.h.wrapping_add(w).wrapping_mul(K);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut c = bytes.chunks_exact(8);
        for w in &mut c {
            self.add(u64::from_le_bytes(w.try_into().unwrap()));
        }
        let r = c.remainder();
        if !r.is_empty() {
            let mut t = [0u8; 8];
            t[..r.len()].copy_from_slice(r);
            self.add(u64::from_le_bytes(t) ^ (r.len() as u64) << 59);
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.h.rotate_left(26)
    }
}

pub type FxBuildHasher = BuildHasherDefault<FxHasher>;
pub type FxHashMap<K, V> = HashMap<K, V, FxBuildHasher>;

#[cfg(test)]
mod tests {
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
}
