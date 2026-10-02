//! Engine dataset (format v1, produced by `eve-sde-pipeline`).
use rustc_hash::FxHashMap;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct AttrInfo {
    pub id: u32,
    pub name: String,
    pub default: f64,
    pub stackable: bool,
    pub high_is_good: bool,
    pub min_attr: Option<u32>,
    pub max_attr: Option<u32>,
    pub unit: Option<u32>,
    pub display: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
pub enum Func {
    Item,
    Location,
    LocationGroup,
    LocationRequiredSkill,
    OwnerRequiredSkill,
    EffectStopper,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, serde::Serialize, Deserialize)]
pub struct Modifier {
    pub func: Func,
    pub domain: Domain,
    pub modified: u32,
    pub modifying: u32,
    pub op: i32,
    /// group id (LocationGroup) or skill type id (…RequiredSkill)
    pub extra: u32,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct EffectInfo {
    pub id: u32,
    pub name: String,
    pub category: u8,
    pub duration_attr: Option<u32>,
    pub discharge_attr: Option<u32>,
    pub range_attr: Option<u32>,
    pub falloff_attr: Option<u32>,
    pub tracking_attr: Option<u32>,
    pub resistance_attr: Option<u32>,
    pub fitting_usage_chance_attr: Option<u32>,
    pub is_offensive: bool,
    pub is_assistance: bool,
    pub mods: Vec<Modifier>,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct TypeInfo {
    pub id: u32,
    pub name: String,
    pub group: u32,
    pub category: u32,
    pub published: bool,
    pub mass: f64,
    pub volume: f64,
    pub capacity: f64,
    pub radius: f64,
    pub market_group: Option<u32>,
    pub meta_group: Option<u32>,
    pub meta_level: Option<i32>,
    pub variation_parent: Option<u32>,
    pub attrs: Vec<(u32, f64)>,
    pub effects: Vec<(u32, bool)>,
}

impl TypeInfo {
    pub fn attr(&self, id: u32) -> Option<f64> {
        self.attrs.iter().find(|(a, _)| *a == id).map(|(_, v)| *v)
    }
    pub fn has_effect(&self, id: u32) -> bool {
        self.effects.iter().any(|(e, _)| *e == id)
    }
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct GroupInfo {
    pub name: String,
    pub category: u32,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct DbuffInfo {
    pub name: Option<String>,
    pub aggregate: Option<String>,
    pub op: i32,
    pub item: Vec<u32>,
    pub location: Vec<u32>,
    pub location_group: Vec<(u32, u32)>,
    pub location_skill: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct MutaMapping {
    pub inputs: Vec<u32>,
    pub output: u32,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct MutaInfo {
    pub attrs: HashMap<String, (f64, f64)>,
    pub mapping: Vec<MutaMapping>,
}

pub struct Dataset {
    pub build: u64,
    pub release_date: Option<String>,
    pub sha256: String,
    pub types: TypeTable,
    pub groups: FxHashMap<u32, GroupInfo>,
    pub attrs: FxHashMap<u32, AttrInfo>,
    pub effects: FxHashMap<u32, EffectInfo>,
    pub dbuffs: FxHashMap<u32, DbuffInfo>,
    pub mutaplasmids: FxHashMap<u32, MutaInfo>,
    attr_by_name: FxHashMap<String, u32>,
    effect_by_name: FxHashMap<String, u32>,
    /// (names_zh, type_by_name): only needed by search/type/EFT, so a snapshot load decodes them on first use
    names: std::sync::OnceLock<Names>,
    names_blob: Vec<u8>,
    /// all skill type ids (category 16)
    pub skills: Vec<u32>,
    /// Variant B: dataset-derived precomputation (skill folding tables), built once on first use.
    pub prepared: std::sync::OnceLock<crate::engine::Prepared>,
}

// ---------- raw serde shapes ----------
#[derive(Deserialize)]
struct RawDs {
    format: String,
    format_version: u32,
    sde: RawSde,
    groups: IdVec<RawGroup>,
    attributes: IdVec<RawAttr>,
    effects: IdVec<RawEffect>,
    types: IdVec<RawType>,
    #[serde(default)]
    dbuffs: IdVec<DbuffInfo>,
    #[serde(default)]
    mutaplasmids: IdVec<MutaInfo>,
    #[serde(default)]
    names: HashMap<String, IdVec<String>>,
}
#[derive(Deserialize)]
struct RawSde {
    build: u64,
    release_date: Option<String>,
}
#[derive(Deserialize)]
struct RawGroup {
    name: Option<String>,
    category: u32,
}
#[derive(Deserialize)]
struct RawAttr {
    name: String,
    #[serde(default)]
    default: f64,
    #[serde(default = "t")]
    stackable: bool,
    #[serde(default = "t")]
    high_is_good: bool,
    min_attr: Option<u32>,
    max_attr: Option<u32>,
    unit: Option<u32>,
    display: Option<String>,
}
fn t() -> bool {
    true
}
#[derive(Deserialize)]
struct RawEffect {
    name: String,
    #[serde(default)]
    category: u8,
    duration_attr: Option<u32>,
    discharge_attr: Option<u32>,
    range_attr: Option<u32>,
    falloff_attr: Option<u32>,
    tracking_attr: Option<u32>,
    resistance_attr: Option<u32>,
    fitting_usage_chance_attr: Option<u32>,
    #[serde(default)]
    is_offensive: bool,
    #[serde(default)]
    is_assistance: bool,
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
    market_group: Option<u32>,
    meta_group: Option<u32>,
    meta_level: Option<i32>,
    variation_parent: Option<u32>,
    #[serde(default)]
    attrs: IdVec<f64>,
    #[serde(default)]
    effects: Vec<(u32, u8)>,
}

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

impl Dataset {
    /// Load a dataset file. Variant B keeps a derived binary snapshot (bincode of the parsed dataset) in a cache
    /// directory keyed by the SHA-256 of the file bytes, so later processes skip gunzip + JSON parsing.
    /// `EVE_DOGMA_NO_CACHE=1` disables it; `EVE_DOGMA_CACHE=DIR` sets the directory.
    pub fn load_path(path: &str) -> Result<Dataset, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
        if std::env::var_os("EVE_DOGMA_NO_CACHE").is_some() {
            return Self::load_bytes(&bytes);
        }
        let tk = std::time::Instant::now();
        let key = {
            use sha2::Digest;
            let d = sha2::Sha256::digest(&bytes);
            d.iter().map(|b| format!("{b:02x}")).collect::<String>()
        };
        let dir = std::env::var_os("EVE_DOGMA_CACHE")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("XDG_CACHE_HOME").map(|d| std::path::PathBuf::from(d).join("eve-dogma-vb")))
            .or_else(|| std::env::var_os("HOME").map(|d| std::path::PathBuf::from(d).join(".cache/eve-dogma-vb")))
            .unwrap_or_else(|| std::env::temp_dir().join("eve-dogma-vb"));
        let file = dir.join(format!("{key}-v{}-{SNAPSHOT_VERSION}.bin", env!("CARGO_PKG_VERSION")));
        let timing = std::env::var_os("VB_LOAD_TIMING").is_some();
        if timing {
            eprintln!("key {:?}", tk.elapsed());
        }
        if let Ok(snap) = std::fs::read(&file) {
            let t0 = std::time::Instant::now();
            if let Some(ds) = Self::from_snapshot(snap) {
                if timing {
                    eprintln!("snapshot {:?}", t0.elapsed());
                }
                return Ok(ds);
            }
        }
        let ds = Self::load_bytes(&bytes)?;
        // best effort: never fail a calculation because the cache is not writable
        let _ = std::fs::create_dir_all(&dir).and_then(|_| {
            let tmp = dir.join(format!("{key}.{}.tmp", std::process::id()));
            let main = bincode::serialize(&Snapshot::of(&ds)).map_err(std::io::Error::other)?;
            let names = bincode::serialize(ds.names()).map_err(std::io::Error::other)?;
            let types = ds.types.encode().map_err(std::io::Error::other)?;
            let mut data = Vec::with_capacity(16 + main.len() + names.len() + types.len());
            data.extend_from_slice(&(main.len() as u64).to_le_bytes());
            data.extend_from_slice(&main);
            data.extend_from_slice(&(names.len() as u64).to_le_bytes());
            data.extend_from_slice(&names);
            data.extend_from_slice(&types);
            std::fs::write(&tmp, data)?;
            std::fs::rename(&tmp, &file)
        });
        Ok(ds)
    }

    /// Snapshot file: u64 len + bincode(Snapshot), u64 len + bincode(Names), then the lazily decoded type table.
    fn from_snapshot(snap: Vec<u8>) -> Option<Dataset> {
        let rd = |at: usize| -> Option<usize> { Some(u64::from_le_bytes(snap.get(at..at + 8)?.try_into().ok()?) as usize) };
        let main_len = rd(0)?;
        let main_end = 8usize.checked_add(main_len)?;
        let ds: Snapshot = bincode::deserialize(snap.get(8..main_end)?).ok()?;
        let names_len = rd(main_end)?;
        let names_end = (main_end + 8).checked_add(names_len)?;
        let names = snap.get(main_end + 8..names_end)?.to_vec();
        let types = TypeTable::decode(snap, names_end)?;
        Some(ds.into_dataset(names, types))
    }

    pub fn load_bytes(bytes: &[u8]) -> Result<Dataset, String> {
        let json: Vec<u8> = if bytes.len() > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
            let mut d = flate2::read::GzDecoder::new(bytes);
            let mut out = Vec::with_capacity(bytes.len() * 12);
            d.read_to_end(&mut out).map_err(|e| format!("gunzip: {e}"))?;
            out
        } else {
            bytes.to_vec()
        };
        // hash on a second thread while parsing (the hash is only reported in `meta`)
        let t0 = std::time::Instant::now();
        let (sha256, raw) = std::thread::scope(|sc| {
            let h = sc.spawn(|| {
                use sha2::Digest;
                let d = sha2::Sha256::digest(&json);
                d.iter().map(|b| format!("{b:02x}")).collect::<String>()
            });
            let raw: Result<RawDs, String> = serde_json::from_slice(&json).map_err(|e| format!("dataset json: {e}"));
            (h.join().expect("sha thread"), raw)
        });
        let raw = raw?;
        if std::env::var("VB_LOAD_TIMING").is_ok() {
            eprintln!("sha+parse {:?}", t0.elapsed());
        }
        if raw.format != "exct-eve-dataset" || raw.format_version != 1 {
            return Err(format!("unsupported dataset format {} v{}", raw.format, raw.format_version));
        }
        let mut attrs = FxHashMap::default();
        let mut attr_by_name = FxHashMap::default();
        for (id, a) in raw.attributes.0 {
            attr_by_name.insert(a.name.clone(), id);
            attrs.insert(
                id,
                AttrInfo {
                    id,
                    name: a.name,
                    default: a.default,
                    stackable: a.stackable,
                    high_is_good: a.high_is_good,
                    min_attr: a.min_attr,
                    max_attr: a.max_attr,
                    unit: a.unit,
                    display: a.display,
                },
            );
        }
        let mut effects = FxHashMap::default();
        let mut effect_by_name = FxHashMap::default();
        for (id, e) in raw.effects.0 {
            effect_by_name.insert(e.name.clone(), id);
            let mods = e
                .mods
                .iter()
                .map(|&(f, d, modified, modifying, op, extra)| Modifier {
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
                    id,
                    name: e.name,
                    category: e.category,
                    duration_attr: e.duration_attr,
                    discharge_attr: e.discharge_attr,
                    range_attr: e.range_attr,
                    falloff_attr: e.falloff_attr,
                    tracking_attr: e.tracking_attr,
                    resistance_attr: e.resistance_attr,
                    fitting_usage_chance_attr: e.fitting_usage_chance_attr,
                    is_offensive: e.is_offensive,
                    is_assistance: e.is_assistance,
                    mods,
                },
            );
        }
        let mut groups = FxHashMap::default();
        for (k, g) in raw.groups.0 {
            groups.insert(k, GroupInfo { name: g.name.unwrap_or_default(), category: g.category });
        }
        let mut types = FxHashMap::default();
        let mut type_by_name = FxHashMap::default();
        let mut skills = Vec::new();
        for (id, t) in raw.types.0 {
            let name = t.name.unwrap_or_default();
            if t.published || !type_by_name.contains_key(&name.to_lowercase()) {
                type_by_name.insert(name.to_lowercase(), id);
            }
            if t.category == 16 {
                skills.push(id);
            }
            let mut a: Vec<(u32, f64)> = t.attrs.0;
            a.sort_by_key(|x| x.0);
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
                    radius: t.radius,
                    market_group: t.market_group,
                    meta_group: t.meta_group,
                    meta_level: t.meta_level,
                    variation_parent: t.variation_parent,
                    attrs: a,
                    effects: t.effects.into_iter().map(|(e, d)| (e, d != 0)).collect(),
                },
            );
        }
        skills.sort();
        let dbuffs = raw.dbuffs.0.into_iter().collect();
        let mutaplasmids = raw.mutaplasmids.0.into_iter().collect();
        let names_zh = raw
            .names
            .get("zh")
            .map(|m| m.0.iter().map(|(k, v)| (*k, v.clone())).collect())
            .unwrap_or_default();
        Ok(Dataset {
            build: raw.sde.build,
            release_date: raw.sde.release_date,
            sha256,
            types: TypeTable::from_map(types),
            groups,
            attrs,
            effects,
            dbuffs,
            mutaplasmids,
            attr_by_name,
            effect_by_name,
            names: std::sync::OnceLock::from(Names { zh: names_zh, type_by_name }),
            names_blob: Vec::new(),
            skills,
            prepared: std::sync::OnceLock::new(),
        })
    }

    pub fn attr_id(&self, name: &str) -> u32 {
        *self.attr_by_name.get(name).unwrap_or(&0)
    }
    pub fn effect_id(&self, name: &str) -> u32 {
        *self.effect_by_name.get(name).unwrap_or(&0)
    }
    fn names(&self) -> &Names {
        self.names.get_or_init(|| bincode::deserialize(&self.names_blob).unwrap_or_default())
    }
    pub fn names_zh(&self) -> &FxHashMap<u32, String> {
        &self.names().zh
    }
    pub fn type_by_name(&self, name: &str) -> Option<u32> {
        self.names().type_by_name.get(&name.trim().to_lowercase()).copied()
    }
    pub fn attr_default(&self, id: u32) -> f64 {
        self.attrs.get(&id).map(|a| a.default).unwrap_or(0.0)
    }
}

// Small self-contained SHA-256 (avoids an extra dependency).
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    let bitlen = (data.len() as u64).wrapping_mul(8);
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

/// JSON object with integer-string keys, deserialised straight into a Vec without allocating key Strings.
struct IdVec<T>(Vec<(u32, T)>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for IdVec<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for V<T> {
            type Value = IdVec<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("map with integer keys")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(self, mut m: M) -> Result<IdVec<T>, M::Error> {
                let mut v = Vec::with_capacity(m.size_hint().unwrap_or(16));
                while let Some(IdKey(k)) = m.next_key()? {
                    v.push((k, m.next_value()?));
                }
                Ok(IdVec(v))
            }
        }
        d.deserialize_map(V(std::marker::PhantomData))
    }
}

impl<T> Default for IdVec<T> {
    fn default() -> Self {
        IdVec(Vec::new())
    }
}

struct IdKey(u32);
impl<'de> Deserialize<'de> for IdKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct K;
        impl serde::de::Visitor<'_> for K {
            type Value = IdKey;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("integer string key")
            }
            fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<IdKey, E> {
                Ok(IdKey(s.parse().unwrap_or(0)))
            }
        }
        d.deserialize_str(K)
    }
}

const SNAPSHOT_VERSION: u32 = 3;

/// Type table with the FxHashMap-like API the engine uses. Loaded from a snapshot, each TypeInfo stays
/// bincode-encoded until first use (a calc touches a few hundred of ~10k types).
pub struct TypeTable {
    /// sorted type ids
    ids: Vec<u32>,
    groups: Vec<u32>,
    /// type id -> position + 1 (0 = absent); empty when ids are too sparse (binary search instead)
    dense: Vec<u32>,
    slots: Vec<std::sync::OnceLock<TypeInfo>>,
    blob: Vec<u8>,
    /// (start, end) of each encoded TypeInfo in `blob`
    spans: Vec<(u32, u32)>,
}

impl TypeTable {
    fn build_index(ids: &[u32]) -> Vec<u32> {
        let max = ids.last().copied().unwrap_or(0) as usize;
        if max >= 1 << 24 {
            return Vec::new();
        }
        let mut dense = vec![0u32; max + 1];
        for (p, &id) in ids.iter().enumerate() {
            dense[id as usize] = p as u32 + 1;
        }
        dense
    }

    fn from_map(m: FxHashMap<u32, TypeInfo>) -> TypeTable {
        let mut v: Vec<(u32, TypeInfo)> = m.into_iter().collect();
        v.sort_unstable_by_key(|x| x.0);
        let ids: Vec<u32> = v.iter().map(|x| x.0).collect();
        let groups = v.iter().map(|x| x.1.group).collect();
        let dense = Self::build_index(&ids);
        let slots = v.into_iter().map(|(_, t)| std::sync::OnceLock::from(t)).collect();
        TypeTable { ids, groups, dense, slots, blob: Vec::new(), spans: Vec::new() }
    }

    /// section: u32 n, n x (id, group, start, end) u32 LE (offsets relative to the section), then encoded types
    fn encode(&self) -> Result<Vec<u8>, String> {
        let n = self.ids.len();
        let mut body = Vec::new();
        let mut spans = Vec::with_capacity(n);
        let head = 4 + n * 16;
        for p in 0..n {
            let t = self.at(p);
            let start = head + body.len();
            bincode::serialize_into(&mut body, t).map_err(|e| e.to_string())?;
            spans.push((start as u32, (head + body.len()) as u32));
        }
        let mut out = Vec::with_capacity(head + body.len());
        out.extend_from_slice(&(n as u32).to_le_bytes());
        for p in 0..n {
            for x in [self.ids[p], self.groups[p], spans[p].0, spans[p].1] {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        out.extend_from_slice(&body);
        Ok(out)
    }

    fn decode(blob: Vec<u8>, at: usize) -> Option<TypeTable> {
        let u = |o: usize| -> Option<u32> { Some(u32::from_le_bytes(blob.get(o..o + 4)?.try_into().ok()?)) };
        let n = u(at)? as usize;
        let mut ids = Vec::with_capacity(n);
        let mut groups = Vec::with_capacity(n);
        let mut spans = Vec::with_capacity(n);
        for p in 0..n {
            let o = at + 4 + p * 16;
            ids.push(u(o)?);
            groups.push(u(o + 4)?);
            let (s, e) = (u(o + 8)? as usize + at, u(o + 12)? as usize + at);
            if s > e || e > blob.len() || s > u32::MAX as usize || e > u32::MAX as usize {
                return None;
            }
            spans.push((s as u32, e as u32));
        }
        if ids.windows(2).any(|w| w[0] >= w[1]) {
            return None;
        }
        let dense = Self::build_index(&ids);
        let slots = (0..n).map(|_| std::sync::OnceLock::new()).collect();
        Some(TypeTable { ids, groups, dense, slots, blob, spans })
    }

    #[inline]
    fn pos(&self, id: u32) -> Option<usize> {
        if !self.dense.is_empty() || self.ids.is_empty() {
            match self.dense.get(id as usize) {
                Some(&p) if p != 0 => Some(p as usize - 1),
                _ => None,
            }
        } else {
            self.ids.binary_search(&id).ok()
        }
    }

    #[inline]
    fn at(&self, p: usize) -> &TypeInfo {
        self.slots[p].get_or_init(|| {
            let (s, e) = self.spans[p];
            bincode::deserialize(&self.blob[s as usize..e as usize]).expect("corrupt dataset snapshot")
        })
    }

    #[inline]
    pub fn get(&self, id: &u32) -> Option<&TypeInfo> {
        self.pos(*id).map(|p| self.at(p))
    }
    pub fn contains_key(&self, id: &u32) -> bool {
        self.pos(*id).is_some()
    }
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    /// all types in id order (decodes every type)
    pub fn iter(&self) -> impl Iterator<Item = (&u32, &TypeInfo)> + '_ {
        self.ids.iter().enumerate().map(move |(p, id)| (id, self.at(p)))
    }
    pub fn values(&self) -> impl Iterator<Item = &TypeInfo> + '_ {
        (0..self.ids.len()).map(move |p| self.at(p))
    }
    /// type ids of one group, without decoding other types
    pub fn ids_in_group(&self, group: u32) -> impl Iterator<Item = u32> + '_ {
        self.ids.iter().zip(&self.groups).filter(move |(_, g)| **g == group).map(|(id, _)| *id)
    }
}

impl std::ops::Index<&u32> for TypeTable {
    type Output = TypeInfo;
    fn index(&self, id: &u32) -> &TypeInfo {
        self.get(id).expect("unknown type id")
    }
}

#[derive(Default, serde::Serialize, Deserialize)]
struct Names {
    zh: FxHashMap<u32, String>,
    type_by_name: FxHashMap<String, u32>,
}

#[derive(serde::Serialize, Deserialize)]
struct Snapshot {
    build: u64,
    release_date: Option<String>,
    sha256: String,
    groups: FxHashMap<u32, GroupInfo>,
    attrs: FxHashMap<u32, AttrInfo>,
    effects: FxHashMap<u32, EffectInfo>,
    dbuffs: FxHashMap<u32, DbuffInfo>,
    mutaplasmids: FxHashMap<u32, MutaInfo>,
    attr_by_name: FxHashMap<String, u32>,
    effect_by_name: FxHashMap<String, u32>,
    skills: Vec<u32>,
}

impl Snapshot {
    fn of(d: &Dataset) -> Snapshot {
        Snapshot {
            build: d.build,
            release_date: d.release_date.clone(),
            sha256: d.sha256.clone(),
            groups: d.groups.clone(),
            attrs: d.attrs.clone(),
            effects: d.effects.clone(),
            dbuffs: d.dbuffs.clone(),
            mutaplasmids: d.mutaplasmids.clone(),
            attr_by_name: d.attr_by_name.clone(),
            effect_by_name: d.effect_by_name.clone(),
            skills: d.skills.clone(),
        }
    }
    fn into_dataset(self, names_blob: Vec<u8>, types: TypeTable) -> Dataset {
        Dataset {
            build: self.build,
            release_date: self.release_date,
            sha256: self.sha256,
            types,
            groups: self.groups,
            attrs: self.attrs,
            effects: self.effects,
            dbuffs: self.dbuffs,
            mutaplasmids: self.mutaplasmids,
            attr_by_name: self.attr_by_name,
            effect_by_name: self.effect_by_name,
            names: std::sync::OnceLock::new(),
            names_blob,
            skills: self.skills,
            prepared: std::sync::OnceLock::new(),
        }
    }
}
