//! EXCT dataset (eve-sde-pipeline `exct-eve-dataset` v1) loader.
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::io::Read;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttrInfo {
    pub name: String,
    pub default: f64,
    pub high_is_good: bool,
    pub min_attr: Option<u32>,
    pub max_attr: Option<u32>,
    pub stackable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectInfo {
    pub name: String,
    pub category: u32,
    pub resistance_attr: Option<u32>,
    pub range_attr: Option<u32>,
    pub falloff_attr: Option<u32>,
    pub is_offensive: bool,
    pub is_assistance: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// effect ids only (same order)
    pub effect_ids: Vec<u32>,
    /// requiredSkill1..6 -> (skill type id, level)
    pub req_skills: Vec<(u32, u8)>,
    pub market_group: Option<u32>,
    pub variation_parent: Option<u32>,
    /// Chinese name (dataset `names.zh`), for search / type lookups
    pub name_zh: Option<String>,
    pub meta_level: Option<i32>,
}

impl TypeInfo {
    pub fn attr(&self, a: u32) -> Option<f64> {
        self.attrs.binary_search_by_key(&a, |x| x.0).ok().map(|i| self.attrs[i].1)
    }
    pub fn has_effect(&self, e: u32) -> bool {
        self.effects.iter().any(|x| x.0 == e)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbuffInfo {
    pub aggregate_max: bool,
    pub op: i32,
    pub item: Vec<u32>,
    pub location: Vec<u32>,
    pub location_group: Vec<(u32, u32)>,
    pub location_skill: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mutaplasmid {
    pub attrs: Vec<(u32, f64, f64)>,
    pub mapping: Vec<(Vec<u32>, u32)>,
}

#[derive(Serialize, Deserialize)]
pub struct Dataset {
    pub build: u64,
    pub sha256: String,
    pub attrs: FxHashMap<u32, AttrInfo>,
    pub attr_by_name: FxHashMap<String, u32>,
    pub effects: FxHashMap<u32, EffectInfo>,
    pub effect_by_name: FxHashMap<String, u32>,
    /// lazily decoded types (cache: per-type bincode records, decoded on first access)
    #[serde(skip)]
    pub types: Types,
    /// positions of `skills` in `types` (built on first fit)
    #[serde(skip)]
    pub skill_pos: std::sync::OnceLock<Vec<Option<usize>>>,
    pub group_names: FxHashMap<u32, String>,
    pub group_category: FxHashMap<u32, u32>,
    pub category_names: FxHashMap<u32, String>,
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
    #[serde(default)]
    market_group: Option<u32>,
    #[serde(default)]
    variation_parent: Option<u32>,
    #[serde(default)]
    meta_level: Option<i32>,
}
#[derive(Deserialize)]
struct RawCategory {
    name: String,
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
    categories: FxHashMap<String, RawCategory>,
    #[serde(default)]
    dbuffs: FxHashMap<String, RawDbuff>,
    #[serde(default)]
    mutaplasmids: FxHashMap<String, RawMuta>,
    /// localised names: language -> type id -> name
    #[serde(default)]
    names: FxHashMap<String, FxHashMap<String, String>>,
    sde: RawSde,
}

pub const A_MASS: u32 = 4;
pub const A_CAPACITY: u32 = 38;
pub const A_VOLUME: u32 = 161;
pub const A_RADIUS: u32 = 162;
const REQ_SKILL_ATTRS: [(u32, u32); 6] = [(182, 277), (183, 278), (184, 279), (1285, 1286), (1289, 1287), (1290, 1288)];

fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    let d = sha2::Sha256::digest(data);
    d.iter().map(|x| format!("{x:02x}")).collect()
}

impl Dataset {
    /// Load the dataset, using a derived binary cache (bincode) keyed by a hash of the dataset file.
    /// Cache dir: $EVE_DOGMA_E_CACHE, else $XDG_CACHE_HOME/eve-dogma-e, else ~/.cache/eve-dogma-e, else /tmp.
    pub fn load(path: &str) -> Result<Dataset, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read dataset {path}: {e}"))?;
        let key = {
            use std::hash::Hasher;
            let mut h = rustc_hash::FxHasher::default();
            h.write(&bytes);
            h.write(concat!(env!("CARGO_PKG_VERSION"), "-cache-v4").as_bytes());
            format!("{:016x}-{}", h.finish(), bytes.len())
        };
        let dir = std::env::var_os("EVE_DOGMA_E_CACHE")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("XDG_CACHE_HOME").map(|x| std::path::PathBuf::from(x).join("eve-dogma-e")))
            .or_else(|| std::env::var_os("HOME").map(|x| std::path::PathBuf::from(x).join(".cache/eve-dogma-e")))
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/eve-dogma-e"));
        let cpath = dir.join(format!("dataset-{key}.bin"));
        if std::env::var_os("EVE_DOGMA_E_NO_CACHE").is_none() {
            if let Ok(c) = std::fs::read(&cpath) {
                if let Some(ds) = Self::from_cache(c) {
                    return Ok(ds);
                }
            }
        }
        let ds = Self::from_bytes(bytes)?;
        if std::env::var_os("EVE_DOGMA_E_NO_CACHE").is_none() {
            if let Some(enc) = ds.to_cache() {
                let _ = std::fs::create_dir_all(&dir);
                let tmp = dir.join(format!(".tmp-{}-{key}", std::process::id()));
                if std::fs::write(&tmp, enc).is_ok() {
                    let _ = std::fs::rename(&tmp, &cpath);
                }
            }
        }
        Ok(ds)
    }

    /// cache layout: u64 len A, bincode(Dataset) [A bytes], u64 len B, bincode(TypesIndex) [B bytes], raw type records
    fn to_cache(&self) -> Option<Vec<u8>> {
        let a = bincode::serialize(self).ok()?;
        let b = bincode::serialize(&TypesIndex { ids: self.types.ids.clone(), groups: self.types.groups.clone(), offs: self.types.offs.clone() }).ok()?;
        let mut out = Vec::with_capacity(16 + a.len() + b.len() + self.types.blob.len());
        out.extend((a.len() as u64).to_le_bytes());
        out.extend(a);
        out.extend((b.len() as u64).to_le_bytes());
        out.extend(b);
        out.extend(&self.types.blob[self.types.base..]);
        Some(out)
    }

    fn from_cache(c: Vec<u8>) -> Option<Dataset> {
        let rd = |p: usize| -> Option<usize> { Some(u64::from_le_bytes(c.get(p..p + 8)?.try_into().ok()?) as usize) };
        let la = rd(0)?;
        let mut ds: Dataset = bincode::deserialize(c.get(8..8 + la)?).ok()?;
        let lb = rd(8 + la)?;
        let ix: TypesIndex = bincode::deserialize(c.get(16 + la..16 + la + lb)?).ok()?;
        let base = 16 + la + lb;
        if ix.offs.len() != ix.ids.len() + 1 || base + *ix.offs.last()? as usize != c.len() {
            return None;
        }
        let n = ix.ids.len();
        ds.types = Types { ids: ix.ids, groups: ix.groups, offs: ix.offs, blob: c, base, cells: (0..n).map(|_| std::sync::OnceLock::new()).collect(), by_name: std::sync::OnceLock::new(), by_lname: std::sync::OnceLock::new() };
        Some(ds)
    }

    fn from_bytes(bytes: Vec<u8>) -> Result<Dataset, String> {
        let mut json = Vec::with_capacity(bytes.len() * 11);
        if bytes.starts_with(&[0x1f, 0x8b]) {
            flate2::read::GzDecoder::new(&bytes[..]).read_to_end(&mut json).map_err(|e| format!("gunzip: {e}"))?;
        } else {
            json = bytes;
        }
        Self::from_json(&json)
    }

    pub fn from_json(json: &[u8]) -> Result<Dataset, String> {
        let mut raw: Raw = serde_json::from_slice(json).map_err(|e| format!("dataset json: {e}"))?;
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
        let mut skills = Vec::new();
        let mut zh = raw.names.remove("zh").unwrap_or_default();
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
            types.insert(
                id,
                TypeInfo {
                    id,
                    name: t.name,
                    group: t.group,
                    category: t.category,
                    published: t.published.unwrap_or(false),
                    attrs: av,
                    effect_ids: t.effects.iter().map(|(e, _)| *e).collect(),
                    effects: t.effects.into_iter().map(|(e, d)| (e, d != 0)).collect(),
                    req_skills: req,
                    market_group: t.market_group,
                    variation_parent: t.variation_parent,
                    name_zh: zh.remove(&k),
                    meta_level: t.meta_level,
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
        let category_names: FxHashMap<u32, String> = raw.categories.into_iter().map(|(k, c)| (k.parse().unwrap_or(0), c.name)).collect();
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
            types: Types::from_map(types),
            skill_pos: Default::default(),
            group_names,
            group_category,
            category_names,
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

/// Type table with lazy decoding: ids sorted, each type stored as its own bincode record in `blob`.
#[derive(Default)]
pub struct Types {
    ids: Vec<u32>,
    groups: Vec<u32>,
    offs: Vec<u32>,
    blob: Vec<u8>,
    /// start of the type records inside `blob` (the cache file is kept whole, no copy)
    base: usize,
    cells: Vec<std::sync::OnceLock<TypeInfo>>,
    by_name: std::sync::OnceLock<FxHashMap<String, u32>>,
    by_lname: std::sync::OnceLock<FxHashMap<String, u32>>,
}

#[derive(Serialize, Deserialize)]
struct TypesIndex {
    ids: Vec<u32>,
    groups: Vec<u32>,
    offs: Vec<u32>,
}

impl Types {
    fn from_map(m: FxHashMap<u32, TypeInfo>) -> Types {
        let mut v: Vec<TypeInfo> = m.into_values().collect();
        v.sort_by_key(|t| t.id);
        let mut t = Types::default();
        for ti in v {
            t.ids.push(ti.id);
            t.groups.push(ti.group);
            t.offs.push(t.blob.len() as u32);
            t.blob.extend(bincode::serialize(&ti).unwrap_or_default());
            t.cells.push(std::sync::OnceLock::from(ti));
        }
        t.offs.push(t.blob.len() as u32);
        t
    }
    #[inline]
    fn at(&self, i: usize) -> &TypeInfo {
        self.cells[i].get_or_init(|| {
            let (a, b) = (self.base + self.offs[i] as usize, self.base + self.offs[i + 1] as usize);
            bincode::deserialize(&self.blob[a..b]).expect("corrupt dataset cache record")
        })
    }
    /// position of `id` in the type table (for repeated lookups via `get_at`)
    pub fn index_of(&self, id: u32) -> Option<usize> {
        self.ids.binary_search(&id).ok()
    }
    #[inline]
    pub fn get_at(&self, i: usize) -> &TypeInfo {
        self.at(i)
    }
    pub fn get(&self, id: &u32) -> Option<&TypeInfo> {
        self.ids.binary_search(id).ok().map(|i| self.at(i))
    }
    pub fn contains_key(&self, id: &u32) -> bool {
        self.ids.binary_search(id).is_ok()
    }
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    /// types of one group (ascending id)
    pub fn in_group(&self, g: u32) -> impl Iterator<Item = &TypeInfo> {
        (0..self.ids.len()).filter(move |&i| self.groups[i] == g).map(move |i| self.at(i))
    }
    /// all types, ascending id
    pub fn iter(&self) -> impl Iterator<Item = &TypeInfo> {
        (0..self.ids.len()).map(move |i| self.at(i))
    }
    /// case-insensitive (trimmed) English name -> type id; a published type wins, else the lowest id
    /// (EFT import / `type` by name)
    pub fn by_name_ci(&self, name: &str) -> Option<u32> {
        self.by_lname
            .get_or_init(|| {
                let mut m: FxHashMap<String, (bool, u32)> = FxHashMap::default();
                for t in self.iter() {
                    let e = m.entry(t.name.to_lowercase()).or_insert((t.published, t.id));
                    if t.published && !e.0 {
                        *e = (true, t.id);
                    }
                }
                m.into_iter().map(|(k, v)| (k, v.1)).collect()
            })
            .get(&name.trim().to_lowercase())
            .copied()
    }
    /// name -> lowest type id with that name (built on first use)
    pub fn by_name(&self, name: &str) -> Option<u32> {
        self.by_name
            .get_or_init(|| {
                let mut m: FxHashMap<String, u32> = FxHashMap::default();
                for i in 0..self.ids.len() {
                    let t = self.at(i);
                    m.entry(t.name.clone()).or_insert(t.id); // ids ascending: first is lowest
                }
                m
            })
            .get(name)
            .copied()
    }
}
