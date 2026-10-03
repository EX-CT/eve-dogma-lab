//! Output document for FitStats: a small JSON tree with `'static` keys where possible, written straight to bytes.
//!
//! Replaces `serde_json::Value` inside `stats` (BTreeMap per object, a `String` per key, a full serde pass): objects
//! are unsorted `Vec`s sorted only when written, keys borrow static strings. The written bytes are exactly
//! `serde_json::to_string(&tidy(value))` of the equivalent `Value` (sorted keys, compact, floats rounded to 6
//! decimals, non-finite -> null); `to_value` gives that `Value` for the library API.
//! The `jv!` macro is serde_json's `json!` (MIT OR Apache-2.0, dtolnay) with the targets swapped to `J`.
use serde_json::Value;
use std::borrow::Cow;

pub type Key = Cow<'static, str>;

/// Byte-wise key order (same as `str::cmp`); short keys usually differ in the first bytes,
/// so a leading-byte check avoids the memcmp call.
#[inline]
fn key_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    match (a.first(), b.first()) {
        (Some(x), Some(y)) if x != y => x.cmp(y),
        _ => a.cmp(b),
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum J {
    #[default]
    Null,
    Bool(bool),
    U(u64),
    I(i64),
    F(f64),
    Str(Cow<'static, str>),
    Array(Vec<J>),
    Object(Obj),
}

/// Object with insertion-ordered entries; keys are unique (insert replaces), sorted at write time.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Obj(Vec<(Key, J)>);

impl Obj {
    #[inline]
    pub fn new() -> Obj {
        Obj(Vec::with_capacity(8))
    }
    /// insert or replace (like `serde_json::Map::insert`)
    #[inline]
    pub fn insert(&mut self, k: impl Into<Key>, v: J) -> Option<J> {
        let k = k.into();
        if let Some(e) = self.0.iter_mut().find(|e| e.0 == k) {
            return Some(std::mem::replace(&mut e.1, v));
        }
        self.0.push((k, v));
        None
    }
    pub fn get(&self, k: &str) -> Option<&J> {
        self.0.iter().find(|e| e.0 == k).map(|e| &e.1)
    }
    fn entry(&mut self, k: &str) -> &mut J {
        let p = match self.0.iter().position(|e| e.0 == k) {
            Some(p) => p,
            None => {
                self.0.push((Cow::Owned(k.to_owned()), J::Null));
                self.0.len() - 1
            }
        };
        &mut self.0[p].1
    }
}

static NULL: J = J::Null;

impl std::ops::Index<&str> for J {
    type Output = J;
    fn index(&self, k: &str) -> &J {
        match self {
            J::Object(o) => o.get(k).unwrap_or(&NULL),
            _ => &NULL,
        }
    }
}

impl std::ops::IndexMut<&str> for J {
    /// like serde_json: Null becomes an empty object, missing keys are inserted as Null
    fn index_mut(&mut self, k: &str) -> &mut J {
        if let J::Null = self {
            *self = J::Object(Obj::new());
        }
        match self {
            J::Object(o) => o.entry(k),
            _ => panic!("cannot index a non-object with {k:?}"),
        }
    }
}

impl J {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            J::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The equivalent `serde_json::Value` (unrounded, like the tree `stats` used to build).
    pub fn to_value(&self) -> Value {
        match self {
            J::Null => Value::Null,
            J::Bool(b) => Value::Bool(*b),
            J::U(u) => Value::from(*u),
            J::I(i) => Value::from(*i),
            J::F(f) => Value::from(*f),
            J::Str(s) => Value::String(s.to_string()),
            J::Array(a) => Value::Array(a.iter().map(J::to_value).collect()),
            J::Object(o) => Value::Object(o.0.iter().map(|(k, v)| (k.to_string(), v.to_value())).collect()),
        }
    }

    /// Compact JSON, keys sorted, floats rounded to 6 decimals (non-finite -> null).
    pub fn write(&self, out: &mut Vec<u8>) {
        use serde_json::ser::Formatter;
        let mut f = serde_json::ser::CompactFormatter;
        match self {
            J::Null => out.extend_from_slice(b"null"),
            J::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
            J::U(u) => {
                let _ = f.write_u64(out, *u);
            }
            J::I(i) => {
                let _ = f.write_i64(out, *i);
            }
            J::F(v) => {
                let r = crate::stats::round6(*v);
                if v.is_finite() && r.is_finite() {
                    let _ = f.write_f64(out, r);
                } else {
                    out.extend_from_slice(b"null");
                }
            }
            J::Str(s) => write_str(out, s),
            J::Array(a) => {
                out.push(b'[');
                for (k, x) in a.iter().enumerate() {
                    if k > 0 {
                        out.push(b',');
                    }
                    x.write(out);
                }
                out.push(b']');
            }
            J::Object(o) => {
                let mut ix: Vec<&(Key, J)> = o.0.iter().collect();
                ix.sort_unstable_by(|a, b| key_cmp(&a.0, &b.0));
                out.push(b'{');
                for (k, (key, x)) in ix.into_iter().enumerate() {
                    if k > 0 {
                        out.push(b',');
                    }
                    write_key(out, key);
                    out.push(b':');
                    x.write(out);
                }
                out.push(b'}');
            }
        }
    }

    /// `write` that consumes the document: objects are sorted in place (no index vector).
    pub fn write_owned(self, out: &mut Vec<u8>) {
        match self {
            J::Array(a) => {
                out.push(b'[');
                for (k, x) in a.into_iter().enumerate() {
                    if k > 0 {
                        out.push(b',');
                    }
                    x.write_owned(out);
                }
                out.push(b']');
            }
            J::Object(mut o) => {
                o.0.sort_unstable_by(|a, b| key_cmp(&a.0, &b.0));
                out.push(b'{');
                for (k, (key, x)) in o.0.into_iter().enumerate() {
                    if k > 0 {
                        out.push(b',');
                    }
                    write_key(out, &key);
                    out.push(b':');
                    x.write_owned(out);
                }
                out.push(b'}');
            }
            leaf => leaf.write(out),
        }
    }

    /// Consuming `to_string`.
    pub fn into_string(self) -> String {
        let mut out = Vec::with_capacity(8 * 1024);
        self.write_owned(&mut out);
        // SAFETY: only valid UTF-8 is written (str contents verbatim or escaped by serde_json, ASCII otherwise)
        unsafe { String::from_utf8_unchecked(out) }
    }

    pub fn to_string(&self) -> String {
        let mut out = Vec::with_capacity(8 * 1024);
        self.write(&mut out);
        // SAFETY: only valid UTF-8 is written (str contents verbatim or escaped by serde_json, ASCII otherwise)
        unsafe { String::from_utf8_unchecked(out) }
    }
}

/// Static keys are identifiers from this crate's source (nothing to escape); owned keys go through serde_json.
#[inline]
fn write_key(out: &mut Vec<u8>, k: &Key) {
    match k {
        Cow::Borrowed(k) => {
            debug_assert!(k.bytes().all(|c| c >= 0x20 && c != b'"' && c != b'\\'));
            out.push(b'"');
            out.extend_from_slice(k.as_bytes());
            out.push(b'"');
        }
        Cow::Owned(k) => write_str(out, k),
    }
}

#[inline]
fn write_str(out: &mut Vec<u8>, s: &str) {
    // serde_json's escaping (quotes, backslash, control characters)
    let _ = serde_json::to_writer(&mut *out, s);
}

/// `jv!` converts its operands by reference; `own(x)` hands an already-built document over by value
/// (taken out of the cell on conversion) instead of deep-cloning it.
pub struct Own(std::cell::Cell<J>);
#[inline]
pub fn own(j: impl Into<J>) -> Own {
    Own(std::cell::Cell::new(j.into()))
}
impl ToJ for Own {
    #[inline]
    fn to_j(&self) -> J {
        self.0.take()
    }
}
impl From<Vec<J>> for J {
    #[inline]
    fn from(v: Vec<J>) -> J {
        J::Array(v)
    }
}

/// Conversion used by `jv!(expr)` (the counterpart of `serde_json::to_value(&expr)`).
pub trait ToJ {
    fn to_j(&self) -> J;
}
impl ToJ for J {
    #[inline]
    fn to_j(&self) -> J {
        self.clone()
    }
}
impl ToJ for f64 {
    #[inline]
    fn to_j(&self) -> J {
        if self.is_finite() { J::F(*self) } else { J::Null }
    }
}
impl ToJ for bool {
    #[inline]
    fn to_j(&self) -> J {
        J::Bool(*self)
    }
}
macro_rules! uint_to_j {
    ($($t:ty)*) => {$(impl ToJ for $t { #[inline] fn to_j(&self) -> J { J::U(*self as u64) } })*};
}
uint_to_j!(u8 u16 u32 u64 usize);
macro_rules! int_to_j {
    ($($t:ty)*) => {$(impl ToJ for $t { #[inline] fn to_j(&self) -> J { if *self < 0 { J::I(*self as i64) } else { J::U(*self as u64) } } })*};
}
int_to_j!(i8 i16 i32 i64 isize);
impl ToJ for str {
    #[inline]
    fn to_j(&self) -> J {
        J::Str(Cow::Owned(self.to_owned()))
    }
}
impl ToJ for String {
    #[inline]
    fn to_j(&self) -> J {
        J::Str(Cow::Owned(self.clone()))
    }
}
impl<T: ToJ + ?Sized> ToJ for &T {
    #[inline]
    fn to_j(&self) -> J {
        (**self).to_j()
    }
}
impl<T: ToJ> ToJ for Option<T> {
    #[inline]
    fn to_j(&self) -> J {
        match self {
            Some(v) => v.to_j(),
            None => J::Null,
        }
    }
}
impl<T: ToJ> ToJ for Vec<T> {
    fn to_j(&self) -> J {
        J::Array(self.iter().map(ToJ::to_j).collect())
    }
}
impl<T: ToJ> ToJ for [T] {
    fn to_j(&self) -> J {
        J::Array(self.iter().map(ToJ::to_j).collect())
    }
}

/// `serde_json::json!` building a [`J`].
#[macro_export]
macro_rules! jv {
    ($($json:tt)+) => { $crate::jv_internal!($($json)+) };
}

#[macro_export]
#[doc(hidden)]
macro_rules! jv_internal {
    (@array [$($elems:expr,)*]) => { vec![$($elems,)*] };
    (@array [$($elems:expr),*]) => { vec![$($elems),*] };
    (@array [$($elems:expr,)*] null $($rest:tt)*) => { $crate::jv_internal!(@array [$($elems,)* $crate::jv_internal!(null)] $($rest)*) };
    (@array [$($elems:expr,)*] true $($rest:tt)*) => { $crate::jv_internal!(@array [$($elems,)* $crate::jv_internal!(true)] $($rest)*) };
    (@array [$($elems:expr,)*] false $($rest:tt)*) => { $crate::jv_internal!(@array [$($elems,)* $crate::jv_internal!(false)] $($rest)*) };
    (@array [$($elems:expr,)*] [$($array:tt)*] $($rest:tt)*) => { $crate::jv_internal!(@array [$($elems,)* $crate::jv_internal!([$($array)*])] $($rest)*) };
    (@array [$($elems:expr,)*] {$($map:tt)*} $($rest:tt)*) => { $crate::jv_internal!(@array [$($elems,)* $crate::jv_internal!({$($map)*})] $($rest)*) };
    (@array [$($elems:expr,)*] $next:expr, $($rest:tt)*) => { $crate::jv_internal!(@array [$($elems,)* $crate::jv_internal!($next),] $($rest)*) };
    (@array [$($elems:expr,)*] $last:expr) => { $crate::jv_internal!(@array [$($elems,)* $crate::jv_internal!($last)]) };
    (@array [$($elems:expr),*] , $($rest:tt)*) => { $crate::jv_internal!(@array [$($elems,)*] $($rest)*) };

    (@object $object:ident () () ()) => {};
    (@object $object:ident [$($key:tt)+] ($value:expr) , $($rest:tt)*) => {
        let _ = $object.insert($($key)+, $value);
        $crate::jv_internal!(@object $object () ($($rest)*) ($($rest)*));
    };
    (@object $object:ident [$($key:tt)+] ($value:expr)) => { let _ = $object.insert($($key)+, $value); };
    (@object $object:ident ($($key:tt)+) (: null $($rest:tt)*) $copy:tt) => { $crate::jv_internal!(@object $object [$($key)+] ($crate::jv_internal!(null)) $($rest)*); };
    (@object $object:ident ($($key:tt)+) (: true $($rest:tt)*) $copy:tt) => { $crate::jv_internal!(@object $object [$($key)+] ($crate::jv_internal!(true)) $($rest)*); };
    (@object $object:ident ($($key:tt)+) (: false $($rest:tt)*) $copy:tt) => { $crate::jv_internal!(@object $object [$($key)+] ($crate::jv_internal!(false)) $($rest)*); };
    (@object $object:ident ($($key:tt)+) (: [$($array:tt)*] $($rest:tt)*) $copy:tt) => { $crate::jv_internal!(@object $object [$($key)+] ($crate::jv_internal!([$($array)*])) $($rest)*); };
    (@object $object:ident ($($key:tt)+) (: {$($map:tt)*} $($rest:tt)*) $copy:tt) => { $crate::jv_internal!(@object $object [$($key)+] ($crate::jv_internal!({$($map)*})) $($rest)*); };
    (@object $object:ident ($($key:tt)+) (: $value:expr , $($rest:tt)*) $copy:tt) => { $crate::jv_internal!(@object $object [$($key)+] ($crate::jv_internal!($value)) , $($rest)*); };
    (@object $object:ident ($($key:tt)+) (: $value:expr) $copy:tt) => { $crate::jv_internal!(@object $object [$($key)+] ($crate::jv_internal!($value))); };
    (@object $object:ident () (($key:expr) : $($rest:tt)*) $copy:tt) => { $crate::jv_internal!(@object $object ($key) (: $($rest)*) (: $($rest)*)); };
    (@object $object:ident ($($key:tt)*) ($tt:tt $($rest:tt)*) $copy:tt) => { $crate::jv_internal!(@object $object ($($key)* $tt) ($($rest)*) ($($rest)*)); };

    (null) => { $crate::out::J::Null };
    (true) => { $crate::out::J::Bool(true) };
    (false) => { $crate::out::J::Bool(false) };
    ([]) => { $crate::out::J::Array(vec![]) };
    ([ $($tt:tt)+ ]) => { $crate::out::J::Array($crate::jv_internal!(@array [] $($tt)+)) };
    ({}) => { $crate::out::J::Object($crate::out::Obj::new()) };
    ({ $($tt:tt)+ }) => {
        $crate::out::J::Object({
            let mut object = $crate::out::Obj::new();
            $crate::jv_internal!(@object object () ($($tt)+) ($($tt)+));
            object
        })
    };
    ($other:expr) => { $crate::out::ToJ::to_j(&$other) };
}

// request enums as their serde (snake_case) names, without an allocation
impl ToJ for crate::request::Slot {
    fn to_j(&self) -> J {
        use crate::request::Slot::*;
        J::Str(Cow::Borrowed(match self {
            High => "high",
            Mid => "mid",
            Low => "low",
            Rig => "rig",
            Subsystem => "subsystem",
            Service => "service",
        }))
    }
}
impl ToJ for crate::request::State {
    fn to_j(&self) -> J {
        use crate::request::State::*;
        J::Str(Cow::Borrowed(match self {
            Offline => "offline",
            Online => "online",
            Active => "active",
            Overheated => "overheated",
        }))
    }
}

#[cfg(test)]
#[path = "tests/out.rs"]
mod tests;
