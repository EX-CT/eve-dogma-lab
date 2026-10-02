//! Collects the type ids of a request without building a `serde_json::Value`: a serde `Serializer` that applies
//! the same rule as walking `serde_json::to_value(req)` (a non-negative integer whose nearest object key is
//! `implants` or ends in `type_id` / `type_ids`; arrays keep their key, objects and structs set it per field).
use serde::ser::{self, Serialize};

pub fn type_ids<T: Serialize + ?Sized>(v: &T, out: &mut Vec<u32>) {
    let _ = v.serialize(Walk { key: "", out });
}

fn wanted(key: &str) -> bool {
    key == "implants" || key.ends_with("type_id") || key.ends_with("type_ids")
}

#[derive(Debug)]
pub struct Never;
impl std::fmt::Display for Never {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("unreachable")
    }
}
impl std::error::Error for Never {}
impl ser::Error for Never {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        Never
    }
}

pub struct Walk<'k, 'o> {
    key: &'k str,
    out: &'o mut Vec<u32>,
}

impl Walk<'_, '_> {
    fn num(self, x: Option<u64>) -> Result<(), Never> {
        if let (true, Some(x)) = (wanted(self.key), x) {
            self.out.push(x as u32);
        }
        Ok(())
    }
}

/// sequences: every element under the same key
pub struct Seq<'k, 'o> {
    key: &'k str,
    out: &'o mut Vec<u32>,
}
/// maps: string keys become the key of the value that follows
pub struct Map<'o> {
    key: String,
    out: &'o mut Vec<u32>,
}
/// structs / struct variants: field names are the keys
pub struct Fields<'o> {
    out: &'o mut Vec<u32>,
}

impl<'k, 'o> ser::Serializer for Walk<'k, 'o> {
    type Ok = ();
    type Error = Never;
    type SerializeSeq = Seq<'k, 'o>;
    type SerializeTuple = Seq<'k, 'o>;
    type SerializeTupleStruct = Seq<'k, 'o>;
    type SerializeTupleVariant = Seq<'static, 'o>;
    type SerializeMap = Map<'o>;
    type SerializeStruct = Fields<'o>;
    type SerializeStructVariant = Fields<'o>;
    fn serialize_bool(self, _: bool) -> Result<(), Never> { Ok(()) }
    fn serialize_i8(self, v: i8) -> Result<(), Never> { self.num(u64::try_from(v).ok()) }
    fn serialize_i16(self, v: i16) -> Result<(), Never> { self.num(u64::try_from(v).ok()) }
    fn serialize_i32(self, v: i32) -> Result<(), Never> { self.num(u64::try_from(v).ok()) }
    fn serialize_i64(self, v: i64) -> Result<(), Never> { self.num(u64::try_from(v).ok()) }
    fn serialize_u8(self, v: u8) -> Result<(), Never> { self.num(Some(v as u64)) }
    fn serialize_u16(self, v: u16) -> Result<(), Never> { self.num(Some(v as u64)) }
    fn serialize_u32(self, v: u32) -> Result<(), Never> { self.num(Some(v as u64)) }
    fn serialize_u64(self, v: u64) -> Result<(), Never> { self.num(Some(v)) }
    // serde_json keeps floats as floats: `as_u64` is None for them
    fn serialize_f32(self, _: f32) -> Result<(), Never> { Ok(()) }
    fn serialize_f64(self, _: f64) -> Result<(), Never> { Ok(()) }
    fn serialize_char(self, _: char) -> Result<(), Never> { Ok(()) }
    fn serialize_str(self, _: &str) -> Result<(), Never> { Ok(()) }
    fn serialize_bytes(self, v: &[u8]) -> Result<(), Never> {
        // serde_json writes bytes as an array of integers
        for &b in v {
            Walk { key: self.key, out: &mut *self.out }.num(Some(b as u64))?;
        }
        Ok(())
    }
    fn serialize_none(self) -> Result<(), Never> { Ok(()) }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<(), Never> { v.serialize(self) }
    fn serialize_unit(self) -> Result<(), Never> { Ok(()) }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Never> { Ok(()) }
    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> Result<(), Never> { Ok(()) }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, v: &T) -> Result<(), Never> { v.serialize(self) }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(self, _: &'static str, _: u32, variant: &'static str, v: &T) -> Result<(), Never> {
        // {"Variant": value}
        v.serialize(Walk { key: variant, out: self.out })
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Seq<'k, 'o>, Never> { Ok(Seq { key: self.key, out: self.out }) }
    fn serialize_tuple(self, _: usize) -> Result<Seq<'k, 'o>, Never> { Ok(Seq { key: self.key, out: self.out }) }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Seq<'k, 'o>, Never> { Ok(Seq { key: self.key, out: self.out }) }
    fn serialize_tuple_variant(self, _: &'static str, _: u32, variant: &'static str, _: usize) -> Result<Seq<'static, 'o>, Never> {
        Ok(Seq { key: variant, out: self.out })
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Map<'o>, Never> { Ok(Map { key: String::new(), out: self.out }) }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Fields<'o>, Never> { Ok(Fields { out: self.out }) }
    fn serialize_struct_variant(self, _: &'static str, _: u32, _: &'static str, _: usize) -> Result<Fields<'o>, Never> {
        // {"Variant": {fields}}: the fields set their own keys
        Ok(Fields { out: self.out })
    }
}

impl ser::SerializeSeq for Seq<'_, '_> {
    type Ok = ();
    type Error = Never;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Never> { v.serialize(Walk { key: self.key, out: &mut *self.out }) }
    fn end(self) -> Result<(), Never> { Ok(()) }
}
impl ser::SerializeTuple for Seq<'_, '_> {
    type Ok = ();
    type Error = Never;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Never> { v.serialize(Walk { key: self.key, out: &mut *self.out }) }
    fn end(self) -> Result<(), Never> { Ok(()) }
}
impl ser::SerializeTupleStruct for Seq<'_, '_> {
    type Ok = ();
    type Error = Never;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Never> { v.serialize(Walk { key: self.key, out: &mut *self.out }) }
    fn end(self) -> Result<(), Never> { Ok(()) }
}
impl ser::SerializeTupleVariant for Seq<'_, '_> {
    type Ok = ();
    type Error = Never;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Never> { v.serialize(Walk { key: self.key, out: &mut *self.out }) }
    fn end(self) -> Result<(), Never> { Ok(()) }
}

/// map keys: serde_json turns string / integer keys into strings
struct KeyText<'a>(&'a mut String);
macro_rules! key_num {
    ($($f:ident: $t:ty),*) => { $(fn $f(self, v: $t) -> Result<(), Never> { *self.0 = v.to_string(); Ok(()) })* };
}
impl ser::Serializer for KeyText<'_> {
    type Ok = ();
    type Error = Never;
    type SerializeSeq = ser::Impossible<(), Never>;
    type SerializeTuple = ser::Impossible<(), Never>;
    type SerializeTupleStruct = ser::Impossible<(), Never>;
    type SerializeTupleVariant = ser::Impossible<(), Never>;
    type SerializeMap = ser::Impossible<(), Never>;
    type SerializeStruct = ser::Impossible<(), Never>;
    type SerializeStructVariant = ser::Impossible<(), Never>;
    key_num!(serialize_i8: i8, serialize_i16: i16, serialize_i32: i32, serialize_i64: i64, serialize_u8: u8, serialize_u16: u16,
        serialize_u32: u32, serialize_u64: u64, serialize_bool: bool, serialize_char: char);
    fn serialize_f32(self, _: f32) -> Result<(), Never> { Err(Never) }
    fn serialize_f64(self, _: f64) -> Result<(), Never> { Err(Never) }
    fn serialize_str(self, v: &str) -> Result<(), Never> { *self.0 = v.to_string(); Ok(()) }
    fn serialize_bytes(self, _: &[u8]) -> Result<(), Never> { Err(Never) }
    fn serialize_none(self) -> Result<(), Never> { Err(Never) }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<(), Never> { v.serialize(self) }
    fn serialize_unit(self) -> Result<(), Never> { Err(Never) }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Never> { Err(Never) }
    fn serialize_unit_variant(self, _: &'static str, _: u32, v: &'static str) -> Result<(), Never> { *self.0 = v.to_string(); Ok(()) }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, v: &T) -> Result<(), Never> { v.serialize(self) }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(self, _: &'static str, _: u32, _: &'static str, _: &T) -> Result<(), Never> { Err(Never) }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Never> { Err(Never) }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Never> { Err(Never) }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeTupleStruct, Never> { Err(Never) }
    fn serialize_tuple_variant(self, _: &'static str, _: u32, _: &'static str, _: usize) -> Result<Self::SerializeTupleVariant, Never> { Err(Never) }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Never> { Err(Never) }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, Never> { Err(Never) }
    fn serialize_struct_variant(self, _: &'static str, _: u32, _: &'static str, _: usize) -> Result<Self::SerializeStructVariant, Never> { Err(Never) }
}

impl ser::SerializeMap for Map<'_> {
    type Ok = ();
    type Error = Never;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, k: &T) -> Result<(), Never> {
        self.key.clear();
        k.serialize(KeyText(&mut self.key))
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Never> { v.serialize(Walk { key: &self.key, out: &mut *self.out }) }
    fn end(self) -> Result<(), Never> { Ok(()) }
}
impl ser::SerializeStruct for Fields<'_> {
    type Ok = ();
    type Error = Never;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, k: &'static str, v: &T) -> Result<(), Never> { v.serialize(Walk { key: k, out: &mut *self.out }) }
    fn end(self) -> Result<(), Never> { Ok(()) }
}
impl ser::SerializeStructVariant for Fields<'_> {
    type Ok = ();
    type Error = Never;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, k: &'static str, v: &T) -> Result<(), Never> { v.serialize(Walk { key: k, out: &mut *self.out }) }
    fn end(self) -> Result<(), Never> { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    // the Value walk this replaces (kept here as the reference)
    fn reference(v: &serde_json::Value, key: &str, out: &mut Vec<u32>) {
        match v {
            serde_json::Value::Number(n) if wanted(key) => {
                if let Some(x) = n.as_u64() {
                    out.push(x as u32)
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| reference(x, key, out)),
            serde_json::Value::Object(o) => o.iter().for_each(|(k, x)| reference(x, k, out)),
            _ => {}
        }
    }
    #[test]
    fn same_ids_as_value_walk() {
        let req: crate::FitRequest = serde_json::from_value(serde_json::json!({
            "ship": {"type_id": 587, "mode_type_id": 34319},
            "modules": [{"type_id": 2048, "charge_type_id": 178, "slot": "low", "state": "active",
                         "mutation": {"base_type_id": 448, "mutaplasmid_type_id": 47702, "attributes": {"50": 30.0}}}],
            "drones": [{"type_id": 2488, "quantity": 2}], "implants": [13219], "boosters": [{"type_id": 10151, "side_effects": [1]}],
            "environment": {"effect_type_ids": [30844]},
            "projected": [{"kind": "fit", "fit": {"ship": {"type_id": 24690}, "modules": [{"type_id": 527}]}, "amount": 1}]
        })).unwrap();
        let (mut a, mut b) = (Vec::new(), Vec::new());
        type_ids(&req, &mut a);
        reference(&serde_json::to_value(&req).unwrap(), "", &mut b);
        // the Value walk visits object keys alphabetically; only the set matters to the caller
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b);
        assert!(a.contains(&24690) && a.contains(&13219) && a.contains(&47702));
    }
}
