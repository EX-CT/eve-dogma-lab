//! Derived binary cache of the decoded dataset (allowed by the brief: "you may add a derived cache").
//! Key = FxHash64 + length of the raw dataset file bytes; stored in `$EVE_I_CACHE_DIR` or the OS temp dir.
//! Disable with `EVE_I_NO_CACHE=1`. A corrupt or stale cache is ignored and rewritten.
use crate::data::*;
use rustc_hash::FxHashMap;
use std::collections::HashMap;
use std::hash::Hasher;

const MAGIC: &[u8; 8] = b"EVEI\x00\x00\x00\x04";

pub fn key_of(bytes: &[u8]) -> u64 {
    let mut h = rustc_hash::FxHasher::default();
    for c in bytes.chunks(8) {
        let mut b = [0u8; 8];
        b[..c.len()].copy_from_slice(c);
        h.write_u64(u64::from_le_bytes(b));
    }
    h.write_usize(bytes.len());
    h.finish()
}

pub fn cache_path(key: u64) -> std::path::PathBuf {
    let dir = std::env::var_os("EVE_I_CACHE_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join(format!("eve-dogma-i-{}-{key:016x}.bin", env!("CARGO_PKG_VERSION")))
}

#[derive(Default)]
struct W(Vec<u8>);
impl W {
    fn u8(&mut self, v: u8) { self.0.push(v) }
    fn u32(&mut self, v: u32) { self.0.extend_from_slice(&v.to_le_bytes()) }
    fn i32(&mut self, v: i32) { self.0.extend_from_slice(&v.to_le_bytes()) }
    fn u64(&mut self, v: u64) { self.0.extend_from_slice(&v.to_le_bytes()) }
    fn f64(&mut self, v: f64) { self.0.extend_from_slice(&v.to_le_bytes()) }
    fn b(&mut self, v: bool) { self.u8(v as u8) }
    fn s(&mut self, v: &str) { self.u32(v.len() as u32); self.0.extend_from_slice(v.as_bytes()) }
    fn os(&mut self, v: &Option<String>) { match v { Some(x) => { self.u8(1); self.s(x) } None => self.u8(0) } }
    fn ou(&mut self, v: Option<u32>) { match v { Some(x) => { self.u8(1); self.u32(x) } None => self.u8(0) } }
    fn oi(&mut self, v: Option<i32>) { match v { Some(x) => { self.u8(1); self.i32(x) } None => self.u8(0) } }
    fn vu(&mut self, v: &[u32]) { self.u32(v.len() as u32); for x in v { self.u32(*x) } }
    fn vp(&mut self, v: &[(u32, u32)]) { self.u32(v.len() as u32); for (a, b) in v { self.u32(*a); self.u32(*b) } }
}

struct R<'a>(&'a [u8], usize);
impl<'a> R<'a> {
    #[inline]
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.0.get(self.1..self.1 + n)?;
        self.1 += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> { Some(self.take(1)?[0]) }
    fn u32(&mut self) -> Option<u32> { Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?)) }
    fn i32(&mut self) -> Option<i32> { Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?)) }
    fn u64(&mut self) -> Option<u64> { Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?)) }
    fn f64(&mut self) -> Option<f64> { Some(f64::from_le_bytes(self.take(8)?.try_into().ok()?)) }
    fn b(&mut self) -> Option<bool> { Some(self.u8()? != 0) }
    fn s(&mut self) -> Option<String> { let n = self.u32()? as usize; String::from_utf8(self.take(n)?.to_vec()).ok() }
    fn os(&mut self) -> Option<Option<String>> { Some(if self.u8()? == 1 { Some(self.s()?) } else { None }) }
    fn ou(&mut self) -> Option<Option<u32>> { Some(if self.u8()? == 1 { Some(self.u32()?) } else { None }) }
    fn oi(&mut self) -> Option<Option<i32>> { Some(if self.u8()? == 1 { Some(self.i32()?) } else { None }) }
    fn n(&mut self) -> Option<usize> { Some(self.u32()? as usize) }
    fn vu(&mut self) -> Option<Vec<u32>> { let n = self.n()?; (0..n).map(|_| self.u32()).collect() }
    fn vp(&mut self) -> Option<Vec<(u32, u32)>> { let n = self.n()?; (0..n).map(|_| Some((self.u32()?, self.u32()?))).collect() }
}

fn func_code(f: Func) -> u8 {
    match f { Func::Item => 0, Func::Location => 1, Func::LocationGroup => 2, Func::LocationRequiredSkill => 3, Func::OwnerRequiredSkill => 4, Func::EffectStopper => 5 }
}
fn func_dec(c: u8) -> Func {
    match c { 0 => Func::Item, 1 => Func::Location, 2 => Func::LocationGroup, 3 => Func::LocationRequiredSkill, 4 => Func::OwnerRequiredSkill, _ => Func::EffectStopper }
}
fn dom_code(d: Domain) -> u8 {
    match d { Domain::Item => 0, Domain::Ship => 1, Domain::Char => 2, Domain::Other => 3, Domain::Structure => 4, Domain::TargetId => 5, Domain::Target => 6, Domain::None => 7 }
}
fn dom_dec(c: u8) -> Domain {
    match c { 0 => Domain::Item, 1 => Domain::Ship, 2 => Domain::Char, 3 => Domain::Other, 4 => Domain::Structure, 5 => Domain::TargetId, 6 => Domain::Target, _ => Domain::None }
}

fn sorted<'a, V>(m: &'a FxHashMap<u32, V>) -> Vec<(&'a u32, &'a V)> {
    let mut v: Vec<_> = m.iter().collect();
    v.sort_by_key(|x| *x.0);
    v
}

pub fn encode(ds: &Dataset) -> Vec<u8> {
    let mut w = W::default();
    w.0.extend_from_slice(MAGIC);
    w.u64(ds.build);
    w.os(&ds.release_date);
    w.s(&ds.sha256);
    w.u32(ds.attrs.len() as u32);
    for (_, a) in sorted(&ds.attrs) {
        w.u32(a.id); w.s(&a.name); w.f64(a.default); w.b(a.stackable); w.b(a.high_is_good);
        w.ou(a.min_attr); w.ou(a.max_attr); w.ou(a.unit); w.os(&a.display);
    }
    w.u32(ds.effects.len() as u32);
    for (_, e) in sorted(&ds.effects) {
        w.u32(e.id); w.s(&e.name); w.u8(e.category);
        for x in [e.duration_attr, e.discharge_attr, e.range_attr, e.falloff_attr, e.tracking_attr, e.resistance_attr, e.fitting_usage_chance_attr] { w.ou(x) }
        w.b(e.is_offensive); w.b(e.is_assistance);
        w.u32(e.mods.len() as u32);
        for m in &e.mods { w.u8(func_code(m.func)); w.u8(dom_code(m.domain)); w.u32(m.modified); w.u32(m.modifying); w.i32(m.op); w.u32(m.extra) }
    }
    w.u32(ds.groups.len() as u32);
    for (k, g) in sorted(&ds.groups) { w.u32(*k); w.s(&g.name); w.u32(g.category) }
    w.u32(ds.categories.len() as u32);
    for (k, c) in sorted(&ds.categories) { w.u32(*k); w.s(c) }
    w.u32(ds.types.len() as u32);
    for (_, t) in sorted(&ds.types) {
        w.u32(t.id); w.s(&t.name); w.u32(t.group); w.u32(t.category); w.b(t.published);
        w.f64(t.mass); w.f64(t.volume); w.f64(t.capacity); w.f64(t.radius);
        w.ou(t.market_group); w.ou(t.meta_group); w.oi(t.meta_level); w.ou(t.variation_parent);
        w.u32(t.attrs.len() as u32);
        for (a, v) in &t.attrs { w.u32(*a); w.f64(*v) }
        w.u32(t.effects.len() as u32);
        for (e, d) in &t.effects { w.u32(*e); w.b(*d) }
    }
    w.u32(ds.dbuffs.len() as u32);
    for (k, d) in sorted(&ds.dbuffs) {
        w.u32(*k); w.os(&d.name); w.os(&d.aggregate); w.i32(d.op);
        w.vu(&d.item); w.vu(&d.location); w.vp(&d.location_group); w.vp(&d.location_skill);
    }
    w.u32(ds.mutaplasmids.len() as u32);
    for (k, m) in sorted(&ds.mutaplasmids) {
        w.u32(*k);
        let mut at: Vec<_> = m.attrs.iter().collect();
        at.sort_by(|a, b| a.0.cmp(b.0));
        w.u32(at.len() as u32);
        for (n, (lo, hi)) in at { w.s(n); w.f64(*lo); w.f64(*hi) }
        w.u32(m.mapping.len() as u32);
        for mp in &m.mapping { w.vu(&mp.inputs); w.u32(mp.output) }
    }
    w.u32(ds.names_zh.len() as u32);
    for (k, n) in sorted(&ds.names_zh) { w.u32(*k); w.s(n) }
    let tbn = ds.type_by_name_map();
    let mut tv: Vec<_> = tbn.iter().collect();
    tv.sort();
    w.u32(tv.len() as u32);
    for (n, id) in tv { w.s(n); w.u32(*id) }
    w.vu(&ds.skills);
    w.0.extend_from_slice(MAGIC);
    w.0
}

pub fn decode(b: &[u8]) -> Option<Dataset> {
    let mut r = R(b, 0);
    if r.take(8)? != MAGIC || b.get(b.len().checked_sub(8)?..)? != MAGIC {
        return None;
    }
    let build = r.u64()?;
    let release_date = r.os()?;
    let sha256 = r.s()?;
    let n = r.n()?;
    let mut attrs = FxHashMap::with_capacity_and_hasher(n, Default::default());
    for _ in 0..n {
        let a = AttrInfo {
            id: r.u32()?, name: r.s()?, default: r.f64()?, stackable: r.b()?, high_is_good: r.b()?,
            min_attr: r.ou()?, max_attr: r.ou()?, unit: r.ou()?, display: r.os()?,
        };
        attrs.insert(a.id, a);
    }
    let n = r.n()?;
    let mut effects = FxHashMap::with_capacity_and_hasher(n, Default::default());
    for _ in 0..n {
        let id = r.u32()?; let name = r.s()?; let category = r.u8()?;
        let mut o = [None; 7];
        for x in o.iter_mut() { *x = r.ou()? }
        let is_offensive = r.b()?; let is_assistance = r.b()?;
        let m = r.n()?;
        let mut mods = Vec::with_capacity(m);
        for _ in 0..m {
            mods.push(Modifier { func: func_dec(r.u8()?), domain: dom_dec(r.u8()?), modified: r.u32()?, modifying: r.u32()?, op: r.i32()?, extra: r.u32()? });
        }
        effects.insert(id, EffectInfo {
            id, name, category, duration_attr: o[0], discharge_attr: o[1], range_attr: o[2], falloff_attr: o[3],
            tracking_attr: o[4], resistance_attr: o[5], fitting_usage_chance_attr: o[6], is_offensive, is_assistance, mods,
        });
    }
    let n = r.n()?;
    let mut groups = FxHashMap::with_capacity_and_hasher(n, Default::default());
    for _ in 0..n { let k = r.u32()?; groups.insert(k, GroupInfo { name: r.s()?, category: r.u32()? }); }
    let n = r.n()?;
    let mut categories = FxHashMap::with_capacity_and_hasher(n, Default::default());
    for _ in 0..n { let k = r.u32()?; categories.insert(k, r.s()?); }
    let n = r.n()?;
    let mut types = FxHashMap::with_capacity_and_hasher(n, Default::default());
    for _ in 0..n {
        let id = r.u32()?; let name = r.s()?; let group = r.u32()?; let category = r.u32()?; let published = r.b()?;
        let mass = r.f64()?; let volume = r.f64()?; let capacity = r.f64()?; let radius = r.f64()?;
        let market_group = r.ou()?; let meta_group = r.ou()?; let meta_level = r.oi()?; let variation_parent = r.ou()?;
        let na = r.n()?;
        let mut a = Vec::with_capacity(na);
        for _ in 0..na { a.push((r.u32()?, r.f64()?)) }
        let ne = r.n()?;
        let mut e = Vec::with_capacity(ne);
        for _ in 0..ne { e.push((r.u32()?, r.b()?)) }
        types.insert(id, TypeInfo { id, name, group, category, published, mass, volume, capacity, radius, market_group, meta_group, meta_level, variation_parent, attrs: a, effects: e });
    }
    let n = r.n()?;
    let mut dbuffs = FxHashMap::with_capacity_and_hasher(n, Default::default());
    for _ in 0..n {
        let k = r.u32()?;
        dbuffs.insert(k, DbuffInfo { name: r.os()?, aggregate: r.os()?, op: r.i32()?, item: r.vu()?, location: r.vu()?, location_group: r.vp()?, location_skill: r.vp()? });
    }
    let n = r.n()?;
    let mut mutaplasmids = FxHashMap::with_capacity_and_hasher(n, Default::default());
    for _ in 0..n {
        let k = r.u32()?;
        let na = r.n()?;
        let mut at = HashMap::with_capacity(na);
        for _ in 0..na { let s = r.s()?; at.insert(s, (r.f64()?, r.f64()?)); }
        let nm = r.n()?;
        let mut mapping = Vec::with_capacity(nm);
        for _ in 0..nm { mapping.push(MutaMapping { inputs: r.vu()?, output: r.u32()? }) }
        mutaplasmids.insert(k, MutaInfo { attrs: at, mapping });
    }
    let n = r.n()?;
    let mut names_zh = FxHashMap::with_capacity_and_hasher(n, Default::default());
    for _ in 0..n { let k = r.u32()?; names_zh.insert(k, r.s()?); }
    let n = r.n()?;
    let mut type_by_name = FxHashMap::with_capacity_and_hasher(n, Default::default());
    for _ in 0..n { let s = r.s()?; type_by_name.insert(s, r.u32()?); }
    let skills = r.vu()?;
    if r.1 + 8 != b.len() {
        return None;
    }
    Some(Dataset::from_parts(build, release_date, sha256, types, groups, categories, attrs, effects, dbuffs, mutaplasmids, names_zh, type_by_name, skills))
}
