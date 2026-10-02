//! Unit tests for `data` (kept out of the module source; compiled only for `cargo test`).
use super::*;

#[test]
fn sha256_known_vectors() {
    assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    // 56 bytes: the length no longer fits the first padding block
    assert_eq!(
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    let big: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
    assert_eq!(sha256_hex(&big), "59425e4412e296fc74736673ce067027f384203f59c0d2c3e6be7b13347b3ffc");
}

#[test]
fn name_hash_is_stable() {
    // persisted in the snapshot: changing it needs a SNAPSHOT_VERSION bump
    assert_eq!(fnv1a(b""), 0);
    assert_eq!(fnv1a(b"Rifter"), 0x3f55a9af);
    assert_eq!(fnv1a(b"Large Shield Extender II"), 0x882a98f9);
    assert_ne!(fnv1a(b"Rifter"), fnv1a(b"rifter"));
    assert_ne!(fnv1a(b"abcdefgh"), fnv1a(b"abcdefgh\0"));
}
