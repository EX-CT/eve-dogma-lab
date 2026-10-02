//! Lightweight JSON output tree. Objects are flat vectors (no per-key BTreeMap nodes), keys are
//! serialised sorted so the output is byte-identical to serde_json::Value (BTreeMap-ordered).
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use std::borrow::Cow;

#[derive(Clone, Debug, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    U(u64),
    I(i64),
    F(f64),
    Str(Cow<'static, str>),
    Array(Vec<Value>),
    Object(Map),
}

#[derive(Clone, Debug, Default)]
pub struct Map(pub Vec<(Cow<'static, str>, Value)>);

impl Value {
    pub fn get(&self, k: &str) -> Option<&Value> {
        match self {
            Value::Object(m) => m.0.iter().find(|e| e.0 == k).map(|e| &e.1),
            _ => None,
        }
    }
}

impl Map {
    pub fn new() -> Map {
        Map(Vec::new())
    }
    pub fn with_capacity(n: usize) -> Map {
        Map(Vec::with_capacity(n))
    }
    /// serde_json Map::insert semantics: replaces an existing key
    pub fn insert<K: Into<Cow<'static, str>>>(&mut self, k: K, v: Value) {
        let k = k.into();
        if let Some(e) = self.0.iter_mut().find(|e| e.0 == k) {
            e.1 = v;
        } else {
            self.0.push((k, v));
        }
    }
}

macro_rules! from_u { ($($t:ty),*) => { $(impl From<$t> for Value { fn from(v: $t) -> Value { Value::U(v as u64) } })* } }
macro_rules! from_i { ($($t:ty),*) => { $(impl From<$t> for Value { fn from(v: $t) -> Value { if v < 0 { Value::I(v as i64) } else { Value::U(v as u64) } } })* } }
from_u!(u8, u16, u32, u64, usize);
from_i!(i8, i16, i32, i64, isize);
impl From<f64> for Value {
    fn from(v: f64) -> Value {
        if v.is_finite() { Value::F(v) } else { Value::Null }
    }
}
impl From<f32> for Value {
    fn from(v: f32) -> Value {
        Value::from(v as f64)
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Value {
        Value::Bool(v)
    }
}
impl From<&'static str> for Value {
    fn from(v: &'static str) -> Value {
        Value::Str(Cow::Borrowed(v))
    }
}
impl From<String> for Value {
    fn from(v: String) -> Value {
        Value::Str(Cow::Owned(v))
    }
}
impl From<&String> for Value {
    fn from(v: &String) -> Value {
        Value::Str(Cow::Owned(v.clone()))
    }
}
impl From<Map> for Value {
    fn from(v: Map) -> Value {
        Value::Object(v)
    }
}
impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Value {
        Value::Array(v.into_iter().map(Into::into).collect())
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Value {
        match v {
            Some(x) => x.into(),
            None => Value::Null,
        }
    }
}
impl From<serde_json::Value> for Value {
    fn from(v: serde_json::Value) -> Value {
        use serde_json::Value as S;
        match v {
            S::Null => Value::Null,
            S::Bool(b) => Value::Bool(b),
            S::Number(n) => {
                if let Some(u) = n.as_u64() {
                    Value::U(u)
                } else if let Some(i) = n.as_i64() {
                    Value::I(i)
                } else {
                    Value::from(n.as_f64().unwrap_or(f64::NAN))
                }
            }
            S::String(s) => Value::from(s),
            S::Array(a) => Value::Array(a.into_iter().map(Value::from).collect()),
            S::Object(o) => Value::Object(Map(o.into_iter().map(|(k, v)| (Cow::Owned(k), Value::from(v))).collect())),
        }
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Null => s.serialize_unit(),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::U(u) => s.serialize_u64(*u),
            Value::I(i) => s.serialize_i64(*i),
            Value::F(f) => s.serialize_f64(*f),
            Value::Str(x) => s.serialize_str(x),
            Value::Array(a) => {
                let mut q = s.serialize_seq(Some(a.len()))?;
                for v in a {
                    q.serialize_element(v)?;
                }
                q.end()
            }
            Value::Object(m) => {
                let mut idx: Vec<&(Cow<'static, str>, Value)> = m.0.iter().collect();
                idx.sort_unstable_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
                let mut q = s.serialize_map(Some(idx.len()))?;
                for (k, v) in idx {
                    q.serialize_entry(&**k, v)?;
                }
                q.end()
            }
        }
    }
}

/// build an object from (key, value) pairs
pub fn obj(items: Vec<(&'static str, Value)>) -> Value {
    Value::Object(Map(items.into_iter().map(|(k, v)| (Cow::Borrowed(k), v)).collect()))
}

impl std::ops::Index<&str> for Value {
    type Output = Value;
    fn index(&self, k: &str) -> &Value {
        static NULL: Value = Value::Null;
        self.get(k).unwrap_or(&NULL)
    }
}

/// serde_json IndexMut semantics: Null becomes an object, a missing key is inserted as Null
impl std::ops::IndexMut<&'static str> for Value {
    fn index_mut(&mut self, k: &'static str) -> &mut Value {
        if let Value::Null = self {
            *self = Value::Object(Map::new());
        }
        match self {
            Value::Object(m) => {
                let i = match m.0.iter().position(|e| e.0 == k) {
                    Some(i) => i,
                    None => {
                        m.0.push((Cow::Borrowed(k), Value::Null));
                        m.0.len() - 1
                    }
                };
                &mut m.0[i].1
            }
            _ => panic!("cannot index non-object JSON value"),
        }
    }
}
