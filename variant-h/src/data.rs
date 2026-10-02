//! Static dataset (exct-eve-dataset v1, produced by EX-CT/eve-sde-pipeline) plus derived lookup tables.
//! Loaded once per process; every request is evaluated against this immutable store.
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttrInfo {
    pub name: String,
    pub default: f64,
    pub stackable: bool,
    pub high_is_good: bool,
    pub min_attr: Option<u32>,
    pub max_attr: Option<u32>,
    /// cpu / power / cpuOutput / powerOutput are rounded to 2 decimals after calculation
    pub round2: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Func {
    Item,
    Location,
    LocationGroup,
    LocationRequiredSkill,
    OwnerRequiredSkill,
    EffectStopper,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Domain {
    Item,
    Ship,
    Char,
    Other,
    Structure,
    TargetId,
    Target,
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ModInfo {
    pub func: Func,
    pub domain: Domain,
    pub modified: u32,
    pub modifying: u32,
    pub op: i32,
    pub extra: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectInfo {
    pub name: String,
    pub category: u8,
    pub range_attr: Option<u32>,
    pub falloff_attr: Option<u32>,
    pub resistance_attr: Option<u32>,
    pub fitting_usage_chance_attr: Option<u32>,
    pub mods: Vec<ModInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeInfo {
    pub id: u32,
    pub name: String,
    pub group: u32,
    pub category: u32,
    pub published: bool,
    pub mass: f64,
    pub volume: f64,
    pub capacity: f64,
    /// sorted by attribute id; includes mass/capacity/volume/radius type fields
    pub attrs: Vec<(u32, f64)>,
    pub effects: Vec<(u32, bool)>,
    pub req_skills: Vec<u32>,
}

impl TypeInfo {
    #[inline]
    pub fn attr(&self, id: u32) -> Option<f64> {
        self.attrs.binary_search_by_key(&id, |x| x.0).ok().map(|i| self.attrs[i].1)
    }
    pub fn has_effect(&self, id: u32) -> bool {
        self.effects.iter().any(|(e, _)| *e == id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbuffInfo {
    pub aggregate: Option<String>,
    pub op: i32,
    pub item: Vec<u32>,
    pub location: Vec<u32>,
    pub location_group: Vec<(u32, u32)>,
    pub location_skill: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutaInfo {
    pub attrs: HashMap<String, (f64, f64)>,
}

#[derive(Serialize, Deserialize)]
pub struct Dataset {
    pub build: u64,
    pub sha256: String,
    pub types: FxHashMap<u32, TypeInfo>,
    pub group_names: FxHashMap<u32, String>,
    pub attrs: FxHashMap<u32, AttrInfo>,
    pub effects: FxHashMap<u32, EffectInfo>,
    pub dbuffs: FxHashMap<u32, DbuffInfo>,
    pub mutaplasmids: FxHashMap<u32, MutaInfo>,
    attr_by_name: FxHashMap<String, u32>,
    effect_by_name: FxHashMap<String, u32>,
    type_by_name: FxHashMap<String, u32>,
    /// published skills (category 16), sorted
    pub published_skills: Vec<u32>,
    /// tactical destroyer modes (group 1306): (lowercased name, id), sorted by id
    pub t3d_modes: Vec<(String, u32)>,
    pub a: crate::ids::AttrIds,
    pub e: crate::ids::EffectIds,
}

#[derive(Deserialize)]
struct RawDs {
    format: String,
    format_version: u32,
    sde: RawSde,
    groups: HashMap<String, RawGroup>,
    attributes: HashMap<String, RawAttr>,
    effects: HashMap<String, RawEffect>,
    types: HashMap<String, RawType>,
    #[serde(default)]
    dbuffs: HashMap<String, DbuffInfo>,
    #[serde(default)]
    mutaplasmids: HashMap<String, MutaInfo>,
}
#[derive(Deserialize)]
struct RawSde {
    build: u64,
}
#[derive(Deserialize)]
struct RawGroup {
    name: Option<String>,
}
fn yes() -> bool {
    true
}
#[derive(Deserialize)]
struct RawAttr {
    name: String,
    #[serde(default)]
    default: f64,
    #[serde(default = "yes")]
    stackable: bool,
    #[serde(default = "yes")]
    high_is_good: bool,
    min_attr: Option<u32>,
    max_attr: Option<u32>,
}
#[derive(Deserialize)]
struct RawEffect {
    name: String,
    #[serde(default)]
    category: u8,
    range_attr: Option<u32>,
    falloff_attr: Option<u32>,
    resistance_attr: Option<u32>,
    fitting_usage_chance_attr: Option<u32>,
    #[serde(default)]
    mods: Vec<(i32, i32, u32, u32, i32, u32)>,
}
#[derive(Deserialize)]
struct RawType {
    name: Option<String>,
    group: u32,
    category: u32,
    #[serde(default)]
    published: bool,
    #[serde(default)]
    mass: f64,
    #[serde(default)]
    volume: f64,
    #[serde(default)]
    capacity: f64,
    #[serde(default)]
    radius: f64,
    #[serde(default)]
    attrs: HashMap<String, f64>,
    #[serde(default)]
    effects: Vec<(u32, u8)>,
}

/// requiredSkill1..6
pub const REQ_SKILL_ATTRS: [u32; 6] = [182, 183, 184, 1285, 1289, 1290];

fn func_of(c: i32) -> Func {
    match c {
        0 => Func::Item,
        1 => Func::Location,
        2 => Func::LocationGroup,
        3 => Func::LocationRequiredSkill,
        4 => Func::OwnerRequiredSkill,
        _ => Func::EffectStopper,
    }
}
fn domain_of(c: i32) -> Domain {
    match c {
        0 => Domain::Item,
        1 => Domain::Ship,
        2 => Domain::Char,
        3 => Domain::Other,
        4 => Domain::Structure,
        5 => Domain::TargetId,
        6 => Domain::Target,
        _ => Domain::None,
    }
}

pub fn req_skills_of(attrs: &[(u32, f64)]) -> Vec<u32> {
    REQ_SKILL_ATTRS
        .iter()
        .filter_map(|a| attrs.binary_search_by_key(a, |x| x.0).ok().map(|i| attrs[i].1))
        .map(|v| v as u32)
        .filter(|v| *v != 0)
        .collect()
}

impl Dataset {
    pub fn load_path(path: &str) -> Result<Dataset, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::load_bytes(&bytes)
    }

    pub fn load_bytes(bytes: &[u8]) -> Result<Dataset, String> {
        let json: Vec<u8> = if bytes.len() > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
            let mut d = flate2::read::GzDecoder::new(bytes);
            let mut out = Vec::with_capacity(bytes.len() * 11);
            d.read_to_end(&mut out).map_err(|e| format!("gunzip: {e}"))?;
            out
        } else {
            bytes.to_vec()
        };
        let sha256 = crate::sha256::hex(&json);
        let raw: RawDs = serde_json::from_slice(&json).map_err(|e| format!("dataset json: {e}"))?;
        if raw.format != "exct-eve-dataset" || raw.format_version != 1 {
            return Err(format!("unsupported dataset format {} v{}", raw.format, raw.format_version));
        }
        let mut attrs = FxHashMap::default();
        let mut attr_by_name = FxHashMap::default();
        for (k, a) in raw.attributes {
            let id: u32 = k.parse().unwrap_or(0);
            attr_by_name.insert(a.name.clone(), id);
            let round2 = matches!(a.name.as_str(), "cpu" | "power" | "cpuOutput" | "powerOutput");
            attrs.insert(
                id,
                AttrInfo {
                    name: a.name,
                    default: a.default,
                    stackable: a.stackable,
                    high_is_good: a.high_is_good,
                    min_attr: a.min_attr,
                    max_attr: a.max_attr,
                    round2,
                },
            );
        }
        let mut effects = FxHashMap::default();
        let mut effect_by_name = FxHashMap::default();
        for (k, e) in raw.effects {
            let id: u32 = k.parse().unwrap_or(0);
            effect_by_name.insert(e.name.clone(), id);
            let mods = e
                .mods
                .iter()
                .map(|&(f, d, modified, modifying, op, extra)| ModInfo {
                    func: func_of(f),
                    domain: domain_of(d),
                    modified,
                    modifying,
                    op,
                    extra,
                })
                .collect();
            effects.insert(
                id,
                EffectInfo {
                    name: e.name,
                    category: e.category,
                    range_attr: e.range_attr,
                    falloff_attr: e.falloff_attr,
                    resistance_attr: e.resistance_attr,
                    fitting_usage_chance_attr: e.fitting_usage_chance_attr,
                    mods,
                },
            );
        }
        let group_names = raw.groups.into_iter().map(|(k, g)| (k.parse().unwrap_or(0), g.name.unwrap_or_default())).collect();
        let mut types = FxHashMap::default();
        let mut type_by_name = FxHashMap::default();
        let mut published_skills = Vec::new();
        let mut t3d_modes = Vec::new();
        let mut raw_types: Vec<(u32, RawType)> = raw.types.into_iter().map(|(k, t)| (k.parse().unwrap_or(0), t)).collect();
        raw_types.sort_by_key(|x| x.0);
        for (id, t) in raw_types {
            let name = t.name.unwrap_or_default();
            let lname = name.to_lowercase();
            if t.published || !type_by_name.contains_key(&lname) {
                type_by_name.insert(lname.clone(), id);
            }
            if t.category == 16 && t.published {
                published_skills.push(id);
            }
            if t.group == 1306 {
                t3d_modes.push((lname, id));
            }
            let mut a: FxHashMap<u32, f64> = t.attrs.into_iter().map(|(k, v)| (k.parse().unwrap_or(0), v)).collect();
            // type-level fields are authoritative (mass/capacity/volume/radius)
            for (aid, v) in [(4u32, t.mass), (38, t.capacity), (161, t.volume), (162, t.radius)] {
                if v != 0.0 || !a.contains_key(&aid) {
                    a.insert(aid, v);
                }
            }
            let mut a: Vec<(u32, f64)> = a.into_iter().collect();
            a.sort_by_key(|x| x.0);
            let req_skills = req_skills_of(&a);
            types.insert(
                id,
                TypeInfo {
                    id,
                    name,
                    group: t.group,
                    category: t.category,
                    published: t.published,
                    mass: t.mass,
                    volume: t.volume,
                    capacity: t.capacity,
                    attrs: a,
                    effects: t.effects.into_iter().map(|(e, d)| (e, d != 0)).collect(),
                    req_skills,
                },
            );
        }
        published_skills.sort();
        t3d_modes.sort_by_key(|x| x.1);
        let a = crate::ids::AttrIds::resolve(&attr_by_name);
        let e = crate::ids::EffectIds::resolve(&effect_by_name);
        Ok(Dataset {
            build: raw.sde.build,
            sha256,
            types,
            group_names,
            attrs,
            effects,
            dbuffs: raw.dbuffs.into_iter().map(|(k, v)| (k.parse().unwrap_or(0), v)).collect(),
            mutaplasmids: raw.mutaplasmids.into_iter().map(|(k, v)| (k.parse().unwrap_or(0), v)).collect(),
            attr_by_name,
            effect_by_name,
            type_by_name,
            published_skills,
            t3d_modes,
            a,
            e,
        })
    }

    pub fn attr_id(&self, name: &str) -> u32 {
        *self.attr_by_name.get(name).unwrap_or(&0)
    }
    pub fn effect_id(&self, name: &str) -> u32 {
        *self.effect_by_name.get(name).unwrap_or(&0)
    }
    pub fn type_by_name(&self, name: &str) -> Option<u32> {
        self.type_by_name.get(&name.trim().to_lowercase()).copied()
    }
    #[inline]
    pub fn attr_default(&self, id: u32) -> f64 {
        self.attrs.get(&id).map(|a| a.default).unwrap_or(0.0)
    }
}

// ---------------------------------------------------------------- derived binary cache
/// Derived cache of the processed dataset (bincode). Keyed by a hash of the dataset file bytes and of the
/// running executable (size + mtime), so a rebuilt engine or another dataset never reads a stale cache.
/// Location: `$EVE_DOGMA_H_CACHE_DIR`, else the executable's directory. Purely an optimisation: if it is
/// missing, unreadable or unwritable the dataset is parsed normally and results are identical.
const CACHE_MAGIC: &[u8; 8] = b"EXCTHC01";

fn cache_key(dataset_bytes: &[u8]) -> u64 {
    let mut k = xxhash_rust::xxh3::xxh3_64(dataset_bytes) ^ (dataset_bytes.len() as u64).rotate_left(17);
    if let Ok(exe) = std::env::current_exe() {
        if let Ok(md) = std::fs::metadata(&exe) {
            k ^= md.len().rotate_left(31);
            if let Ok(t) = md.modified() {
                if let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) {
                    k ^= d.as_nanos() as u64;
                }
            }
        }
    }
    k
}

fn cache_file() -> Option<std::path::PathBuf> {
    if let Ok(d) = std::env::var("EVE_DOGMA_H_CACHE_DIR") {
        return Some(std::path::PathBuf::from(d).join("dataset.hcache"));
    }
    Some(std::env::current_exe().ok()?.parent()?.join("dataset.hcache"))
}

impl Dataset {
    /// Load with the derived cache (read if valid, else parse and write it best-effort).
    pub fn load_path_cached(path: &str) -> Result<Dataset, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
        let key = cache_key(&bytes);
        let file = cache_file();
        if let Some(f) = &file {
            if let Ok(c) = std::fs::read(f) {
                if c.len() > 16 && &c[..8] == CACHE_MAGIC && c[8..16] == key.to_le_bytes() {
                    if let Ok(ds) = bincode::deserialize::<Dataset>(&c[16..]) {
                        return Ok(ds);
                    }
                }
            }
        }
        let ds = Self::load_bytes(&bytes)?;
        if let Some(f) = &file {
            if let Ok(body) = bincode::serialize(&ds) {
                let mut out = Vec::with_capacity(body.len() + 16);
                out.extend_from_slice(CACHE_MAGIC);
                out.extend_from_slice(&key.to_le_bytes());
                out.extend_from_slice(&body);
                let tmp = f.with_extension(format!("tmp{}", std::process::id()));
                if std::fs::write(&tmp, &out).is_ok() && std::fs::rename(&tmp, f).is_err() {
                    let _ = std::fs::remove_file(&tmp);
                }
            }
        }
        Ok(ds)
    }
}
