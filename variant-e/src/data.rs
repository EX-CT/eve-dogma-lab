//! EXCT dataset (eve-sde-pipeline `exct-eve-dataset` v1) loader.
use rustc_hash::FxHashMap;
use serde::Deserialize;
use std::io::Read;

#[derive(Debug, Clone)]
pub struct AttrInfo {
    pub name: String,
    pub default: f64,
    pub high_is_good: bool,
    pub min_attr: Option<u32>,
    pub max_attr: Option<u32>,
    pub stackable: bool,
}

#[derive(Debug, Clone)]
pub struct EffectInfo {
    pub name: String,
    pub category: u32,
    pub resistance_attr: Option<u32>,
    pub range_attr: Option<u32>,
    pub falloff_attr: Option<u32>,
    pub is_offensive: bool,
    pub is_assistance: bool,
}

#[derive(Debug, Clone)]
pub struct TypeInfo {
    pub id: u32,
    pub name: String,
    pub group: u32,
    pub category: u32,
    pub published: bool,
    /// sorted by attribute id; includes mass/capacity/volume/radius like Pyfa's item.attributes
    pub attrs: Vec<(u32, f64)>,
    /// (effect id, is_default) in dataset order
    pub effects: Vec<(u32, bool)>,
    /// requiredSkill1..6 -> (skill type id, level)
    pub req_skills: Vec<(u32, u8)>,
}

impl TypeInfo {
    pub fn attr(&self, a: u32) -> Option<f64> {
        self.attrs.binary_search_by_key(&a, |x| x.0).ok().map(|i| self.attrs[i].1)
    }
    pub fn has_effect(&self, e: u32) -> bool {
        self.effects.iter().any(|x| x.0 == e)
    }
}

#[derive(Debug, Clone)]
pub struct DbuffInfo {
    pub aggregate_max: bool,
    pub op: i32,
    pub item: Vec<u32>,
    pub location: Vec<u32>,
    pub location_group: Vec<(u32, u32)>,
    pub location_skill: Vec<(u32, u32)>,
}

#[derive(Debug, Clone)]
pub struct Mutaplasmid {
    pub attrs: Vec<(u32, f64, f64)>,
    pub mapping: Vec<(Vec<u32>, u32)>,
}

pub struct Dataset {
    pub build: u64,
    pub sha256: String,
    pub attrs: FxHashMap<u32, AttrInfo>,
    pub attr_by_name: FxHashMap<String, u32>,
    pub effects: FxHashMap<u32, EffectInfo>,
    pub effect_by_name: FxHashMap<String, u32>,
    pub types: FxHashMap<u32, TypeInfo>,
    pub type_by_name: FxHashMap<String, u32>,
    pub group_names: FxHashMap<u32, String>,
    pub group_category: FxHashMap<u32, u32>,
    pub dbuffs: FxHashMap<u32, DbuffInfo>,
    pub mutaplasmids: FxHashMap<u32, Mutaplasmid>,
    /// published skills, ascending type id (Pyfa Character.getSkillList)
    pub skills: Vec<u32>,
}

#[derive(Deserialize)]
struct RawAttr {
    default: Option<f64>,
    #[serde(default)]
    high_is_good: Option<bool>,
    max_attr: Option<u32>,
    min_attr: Option<u32>,
    name: String,
    #[serde(default)]
    stackable: Option<bool>,
}
#[derive(Deserialize)]
struct RawEffect {
    category: Option<u32>,
    name: String,
    resistance_attr: Option<u32>,
    range_attr: Option<u32>,
    falloff_attr: Option<u32>,
    #[serde(default)]
    is_offensive: Option<bool>,
    #[serde(default)]
    is_assistance: Option<bool>,
}
#[derive(Deserialize)]
struct RawType {
    #[serde(default)]
    attrs: FxHashMap<String, f64>,
    #[serde(default)]
    capacity: Option<f64>,
    category: u32,
    #[serde(default)]
    effects: Vec<(u32, u8)>,
    group: u32,
    #[serde(default)]
    mass: Option<f64>,
    name: String,
    #[serde(default)]
    published: Option<bool>,
    #[serde(default)]
    radius: Option<f64>,
    #[serde(default)]
    volume: Option<f64>,
}
#[derive(Deserialize)]
struct RawGroup {
    category: u32,
    name: String,
}
#[derive(Deserialize)]
struct RawDbuff {
    aggregate: Option<String>,
    #[serde(default)]
    item: Vec<u32>,
    #[serde(default)]
    location: Vec<u32>,
    #[serde(default)]
    location_group: Vec<(u32, u32)>,
    #[serde(default)]
    location_skill: Vec<(u32, u32)>,
    op: i32,
}
#[derive(Deserialize)]
struct RawMapping {
    inputs: Vec<u32>,
    output: u32,
}
#[derive(Deserialize)]
struct RawMuta {
    #[serde(default)]
    attrs: FxHashMap<String, (f64, f64)>,
    #[serde(default)]
    mapping: Vec<RawMapping>,
}
#[derive(Deserialize)]
struct RawSde {
    build: u64,
}
#[derive(Deserialize)]
struct Raw {
    attributes: FxHashMap<String, RawAttr>,
    effects: FxHashMap<String, RawEffect>,
    types: FxHashMap<String, RawType>,
    groups: FxHashMap<String, RawGroup>,
    #[serde(default)]
    dbuffs: FxHashMap<String, RawDbuff>,
    #[serde(default)]
    mutaplasmids: FxHashMap<String, RawMuta>,
    sde: RawSde,
}

pub const A_MASS: u32 = 4;
pub const A_CAPACITY: u32 = 38;
pub const A_VOLUME: u32 = 161;
pub const A_RADIUS: u32 = 162;
const REQ_SKILL_ATTRS: [(u32, u32); 6] = [(182, 277), (183, 278), (184, 279), (1285, 1286), (1289, 1287), (1290, 1288)];

fn sha256_hex(data: &[u8]) -> String {
    // small self-contained SHA-256 (dataset fingerprint for the response meta)
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
        0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
        0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
        0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
        0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    let bitlen = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

impl Dataset {
    pub fn load(path: &str) -> Result<Dataset, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read dataset {path}: {e}"))?;
        let mut json = Vec::with_capacity(bytes.len() * 11);
        if bytes.starts_with(&[0x1f, 0x8b]) {
            flate2::read::GzDecoder::new(&bytes[..]).read_to_end(&mut json).map_err(|e| format!("gunzip: {e}"))?;
        } else {
            json = bytes;
        }
        Self::from_json(&json)
    }

    pub fn from_json(json: &[u8]) -> Result<Dataset, String> {
        let raw: Raw = serde_json::from_slice(json).map_err(|e| format!("dataset json: {e}"))?;
        let sha256 = sha256_hex(json);
        let mut attrs = FxHashMap::default();
        let mut attr_by_name = FxHashMap::default();
        for (k, a) in raw.attributes {
            let id: u32 = k.parse().unwrap_or(0);
            attr_by_name.insert(a.name.clone(), id);
            attrs.insert(
                id,
                AttrInfo {
                    name: a.name,
                    default: a.default.unwrap_or(0.0),
                    high_is_good: a.high_is_good.unwrap_or(true),
                    min_attr: a.min_attr,
                    max_attr: a.max_attr,
                    stackable: a.stackable.unwrap_or(true),
                },
            );
        }
        let mut effects = FxHashMap::default();
        let mut effect_by_name = FxHashMap::default();
        for (k, e) in raw.effects {
            let id: u32 = k.parse().unwrap_or(0);
            effect_by_name.insert(e.name.clone(), id);
            effects.insert(
                id,
                EffectInfo {
                    name: e.name,
                    category: e.category.unwrap_or(0),
                    resistance_attr: e.resistance_attr,
                    range_attr: e.range_attr,
                    falloff_attr: e.falloff_attr,
                    is_offensive: e.is_offensive.unwrap_or(false),
                    is_assistance: e.is_assistance.unwrap_or(false),
                },
            );
        }
        let mut types = FxHashMap::default();
        let mut type_by_name: FxHashMap<String, u32> = FxHashMap::default();
        let mut skills = Vec::new();
        for (k, t) in raw.types {
            let id: u32 = k.parse().unwrap_or(0);
            let mut av: Vec<(u32, f64)> = t.attrs.iter().filter_map(|(a, v)| a.parse().ok().map(|a: u32| (a, *v))).collect();
            for (a, v) in [(A_MASS, t.mass), (A_CAPACITY, t.capacity), (A_VOLUME, t.volume), (A_RADIUS, t.radius)] {
                if let Some(v) = v {
                    if let Some(x) = av.iter_mut().find(|x| x.0 == a) {
                        if v != 0.0 {
                            x.1 = v;
                        }
                    } else {
                        av.push((a, v));
                    }
                }
            }
            av.sort_by_key(|x| x.0);
            let get = |a: u32| av.binary_search_by_key(&a, |x| x.0).ok().map(|i| av[i].1);
            let mut req = Vec::new();
            for (sa, la) in REQ_SKILL_ATTRS {
                if let Some(s) = get(sa) {
                    if s > 0.0 {
                        req.push((s as u32, get(la).unwrap_or(0.0) as u8));
                    }
                }
            }
            if t.category == 16 && t.published.unwrap_or(false) {
                skills.push(id);
            }
            let e = type_by_name.entry(t.name.clone()).or_insert(id);
            if id < *e {
                *e = id;
            }
            types.insert(
                id,
                TypeInfo {
                    id,
                    name: t.name,
                    group: t.group,
                    category: t.category,
                    published: t.published.unwrap_or(false),
                    attrs: av,
                    effects: t.effects.into_iter().map(|(e, d)| (e, d != 0)).collect(),
                    req_skills: req,
                },
            );
        }
        skills.sort();
        let mut group_names = FxHashMap::default();
        let mut group_category = FxHashMap::default();
        for (k, g) in raw.groups {
            let id: u32 = k.parse().unwrap_or(0);
            group_names.insert(id, g.name);
            group_category.insert(id, g.category);
        }
        let mut dbuffs = FxHashMap::default();
        for (k, b) in raw.dbuffs {
            dbuffs.insert(
                k.parse().unwrap_or(0),
                DbuffInfo {
                    aggregate_max: b.aggregate.as_deref() != Some("Minimum"),
                    op: b.op,
                    item: b.item,
                    location: b.location,
                    location_group: b.location_group,
                    location_skill: b.location_skill,
                },
            );
        }
        let mut mutaplasmids = FxHashMap::default();
        for (k, m) in raw.mutaplasmids {
            let mut attrs: Vec<(u32, f64, f64)> = m.attrs.iter().filter_map(|(a, (lo, hi))| a.parse().ok().map(|a| (a, *lo, *hi))).collect();
            attrs.sort_by_key(|x| x.0);
            mutaplasmids.insert(k.parse().unwrap_or(0), Mutaplasmid { attrs, mapping: m.mapping.into_iter().map(|x| (x.inputs, x.output)).collect() });
        }
        Ok(Dataset {
            build: raw.sde.build,
            sha256,
            attrs,
            attr_by_name,
            effects,
            effect_by_name,
            types,
            type_by_name,
            group_names,
            group_category,
            dbuffs,
            mutaplasmids,
            skills,
        })
    }

    pub fn attr_id(&self, name: &str) -> u32 {
        self.attr_by_name.get(name).copied().unwrap_or(0)
    }
    pub fn attr_default(&self, a: u32) -> f64 {
        self.attrs.get(&a).map(|x| x.default).unwrap_or(0.0)
    }
    pub fn effect_id(&self, name: &str) -> u32 {
        self.effect_by_name.get(name).copied().unwrap_or(0)
    }
    pub fn group_name(&self, g: u32) -> &str {
        self.group_names.get(&g).map(|s| s.as_str()).unwrap_or("")
    }
}
