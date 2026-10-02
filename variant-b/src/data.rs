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
    pub groups: LazyTable<GroupInfo>,
    /// category id -> English name
    pub categories: FxHashMap<u32, String>,
    pub attrs: LazyTable<AttrInfo>,
    pub effects: LazyTable<EffectInfo>,
    pub dbuffs: LazyTable<DbuffInfo>,
    pub mutaplasmids: LazyTable<MutaInfo>,
    attr_by_name: NameIndex,
    effect_by_name: NameIndex,
    /// no char Location / LocationGroup(skill group) modifier exists, so skills can be folded into constants
    pub skills_foldable: bool,
    /// (names_zh, type_by_name): only needed by search/type/EFT, so a snapshot load decodes them on first use
    names: std::sync::OnceLock<Names>,
    /// bincode(Names) section of the snapshot (blob, start, end), decoded on first `names()`
    names_blob: Option<(std::sync::Arc<Blob>, usize, usize)>,
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
    #[serde(default)]
    categories: IdVec<RawCategory>,
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
struct RawCategory {
    #[serde(default)]
    name: Option<String>,
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
        let read = || std::fs::read(path).map_err(|e| format!("read {path}: {e}"));
        if std::env::var_os("EVE_DOGMA_NO_CACHE").is_some() {
            return Self::load_bytes(&read()?);
        }
        let tk = std::time::Instant::now();
        // cache key from file identity (absolute path, size, mtime, inode, device) instead of hashing the
        // contents on every start: the pipeline always writes a new file (new mtime/inode) for a new dataset.
        let key = {
            use sha2::Digest;
            use std::os::unix::fs::MetadataExt;
            let md = std::fs::metadata(path).map_err(|e| format!("read {path}: {e}"))?;
            let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.into());
            let id = format!("{}|{}|{}.{}|{}|{}", abs.display(), md.len(), md.mtime(), md.mtime_nsec(), md.ino(), md.dev());
            let d = sha2::Sha256::digest(id.as_bytes());
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
        let tr = std::time::Instant::now();
        // the snapshot is replaced only by rename, so a mapping always sees one complete immutable file
        if let Some(snap) = std::fs::File::open(&file).ok().and_then(|f| unsafe { memmap2::Mmap::map(&f) }.ok()) {
            let snap = Blob::Map(snap);
            if timing {
                eprintln!("read snapshot {:?}", tr.elapsed());
            }
            let t0 = std::time::Instant::now();
            if let Some(ds) = Self::from_snapshot(snap) {
                if timing {
                    eprintln!("snapshot {:?}", t0.elapsed());
                }
                return Ok(ds);
            }
        }
        let ds = Self::load_bytes(&read()?)?;
        // best effort: never fail a calculation because the cache is not writable
        let _ = std::fs::create_dir_all(&dir).and_then(|_| {
            let tmp = dir.join(format!("{key}.{}.tmp", std::process::id()));
            let data = ds.snapshot_bytes().map_err(std::io::Error::other)?;
            std::fs::write(&tmp, data)?;
            std::fs::rename(&tmp, &file)
        });
        Ok(ds)
    }

    /// Snapshot file: magic, then length-prefixed (u64 LE) sections: bincode(Snapshot), bincode(Names), then the
    /// lazily decoded types / effects / dbuffs / mutaplasmids tables and the attr / effect name indexes.
    fn from_snapshot(snap: Blob) -> Option<Dataset> {
        let blob = std::sync::Arc::new(snap);
        let b: &[u8] = &blob;
        if b.get(..8)? != SNAP_MAGIC {
            return None;
        }
        let mut secs: Vec<(usize, usize)> = Vec::with_capacity(8);
        let mut at = 8usize;
        while at < b.len() {
            let len = u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?) as usize;
            let s = at + 8;
            let e = s.checked_add(len)?;
            if e > b.len() {
                return None;
            }
            secs.push((s, e));
            at = e;
        }
        if secs.len() != 13 {
            return None;
        }
        let tm = std::time::Instant::now();
        let main: Snapshot = bincode::deserialize(&b[secs[0].0..secs[0].1]).ok()?;
        if std::env::var_os("VB_LOAD_TIMING").is_some() {
            eprintln!("  main section {:?}", tm.elapsed());
        }
        let names_blob = Some((blob.clone(), secs[1].0, secs[1].1));
        let tt = std::time::Instant::now();
        let lt = std::env::var_os("VB_LOAD_TIMING").is_some();
        let lap = |n: &str| if lt { eprintln!("  {n} {:?}", tt.elapsed()); };
        let types = LazyTable::decode(&blob, secs[2].0, secs[2].1)?;
        lap("types");
        let effects = LazyTable::decode(&blob, secs[3].0, secs[3].1)?;
        lap("effects");
        let dbuffs = LazyTable::decode(&blob, secs[4].0, secs[4].1)?;
        let mutaplasmids = LazyTable::decode(&blob, secs[5].0, secs[5].1)?;
        lap("dbuffs+muta");
        let attr_by_name = NameIndex::decode(&blob, secs[6].0, secs[6].1)?;
        let effect_by_name = NameIndex::decode(&blob, secs[7].0, secs[7].1)?;
        lap("names");
        let folds = LazyTable::decode(&blob, secs[9].0, secs[9].1)?;
        let groups = LazyTable::decode(&blob, secs[10].0, secs[10].1)?;
        let attrs = LazyTable::decode(&blob, secs[11].0, secs[11].1)?;
        let prep = crate::engine::Prepared::from_snapshot(&b[secs[8].0..secs[8].1], folds, &blob, secs[12])?;
        lap("prepared");
        let prepared = std::sync::OnceLock::new();
        let _ = prepared.set(prep);
        Some(Dataset {
            build: main.build,
            release_date: main.release_date,
            sha256: main.sha256,
            types,
            groups,
            categories: main.categories,
            attrs,
            effects,
            dbuffs,
            mutaplasmids,
            attr_by_name,
            effect_by_name,
            skills_foldable: main.skills_foldable,
            names: std::sync::OnceLock::new(),
            names_blob,
            skills: main.skills,
            prepared,
        })
    }

    fn snapshot_bytes(&self) -> Result<Vec<u8>, String> {
        let main = Snapshot {
            build: self.build,
            release_date: self.release_date.clone(),
            sha256: self.sha256.clone(),
            categories: self.categories.clone(),
            skills: self.skills.clone(),
            skills_foldable: self.skills_foldable,
        };
        let names_of = |ix: &NameIndex| -> Vec<u8> { ix.blob[ix.at..].to_vec() };
        let prep = self.prepared.get_or_init(|| crate::engine::Prepared::new(self));
        let (prep_core, prep_folds, prep_meta) = prep.snapshot_sections(self)?;
        let secs: Vec<Vec<u8>> = vec![
            bincode::serialize(&main).map_err(|e| e.to_string())?,
            bincode::serialize(self.names()).map_err(|e| e.to_string())?,
            self.types.encode()?,
            self.effects.encode()?,
            self.dbuffs.encode()?,
            self.mutaplasmids.encode()?,
            names_of(&self.attr_by_name),
            names_of(&self.effect_by_name),
            prep_core,
            prep_folds,
            self.groups.encode()?,
            self.attrs.encode()?,
            prep_meta,
        ];
        let mut out = Vec::with_capacity(8 + secs.iter().map(|x| x.len() + 8).sum::<usize>());
        out.extend_from_slice(SNAP_MAGIC);
        for mut x in secs {
            // pad every section to 8 bytes so in-place u32 arrays stay aligned (all decoders ignore the padding)
            x.resize(x.len().div_ceil(8) * 8, 0);
            out.extend_from_slice(&(x.len() as u64).to_le_bytes());
            out.extend_from_slice(&x);
        }
        Ok(out)
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
        let categories = raw.categories.0.into_iter().map(|(k, c)| (k, c.name.unwrap_or_default())).collect();
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
        let dbuffs: FxHashMap<u32, DbuffInfo> = raw.dbuffs.0.into_iter().collect();
        let mutaplasmids: FxHashMap<u32, MutaInfo> = raw.mutaplasmids.0.into_iter().collect();
        let skills_foldable = {
            let skill_groups: Vec<u32> = groups.iter().filter(|(_, g)| g.category == 16).map(|(id, _)| *id).collect();
            !effects.values().any(|e: &EffectInfo| {
                e.mods.iter().any(|m| {
                    m.domain == Domain::Char && (m.func == Func::Location || (m.func == Func::LocationGroup && skill_groups.contains(&m.extra)))
                })
            })
        };
        let names_zh = raw
            .names
            .get("zh")
            .map(|m| m.0.iter().map(|(k, v)| (*k, v.clone())).collect())
            .unwrap_or_default();
        Ok(Dataset {
            build: raw.sde.build,
            release_date: raw.sde.release_date,
            sha256,
            types: TypeTable::from_map(types, |t| t.group),
            groups: LazyTable::from_map(groups, |_| 0),
            categories,
            attrs: LazyTable::from_map(attrs, |_| 0),
            effects: LazyTable::from_map(effects, |_| 0),
            dbuffs: LazyTable::from_map(dbuffs, |_| 0),
            mutaplasmids: LazyTable::from_map(mutaplasmids, |_| 0),
            attr_by_name: NameIndex::from_map(&attr_by_name),
            effect_by_name: NameIndex::from_map(&effect_by_name),
            skills_foldable,
            names: std::sync::OnceLock::from(Names { zh: names_zh, type_by_name }),
            names_blob: None,
            skills,
            prepared: std::sync::OnceLock::new(),
        })
    }

    #[inline]
    pub fn attr_id(&self, name: &str) -> u32 {
        self.attr_by_name.get(name).unwrap_or(0)
    }
    #[inline]
    pub fn effect_id(&self, name: &str) -> u32 {
        self.effect_by_name.get(name).unwrap_or(0)
    }
    fn names(&self) -> &Names {
        self.names.get_or_init(|| match &self.names_blob {
            Some((b, s, e)) => bincode::deserialize(&b[*s..*e]).unwrap_or_default(),
            None => Names::default(),
        })
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

const SNAPSHOT_VERSION: u32 = 10;

/// Snapshot bytes: memory-mapped cache file (pages faulted in on use) or an owned buffer.
pub enum Blob {
    /// owned bytes, stored as u64 words so in-place u32 arrays are aligned; .1 = byte length
    Words(Vec<u64>, usize),
    Map(memmap2::Mmap),
}

impl Blob {
    fn owned(b: &[u8]) -> Blob {
        let mut w = vec![0u64; b.len().div_ceil(8)];
        // SAFETY: w has at least b.len() bytes
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), w.as_mut_ptr() as *mut u8, b.len()) };
        Blob::Words(w, b.len())
    }
}

impl std::ops::Deref for Blob {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            // SAFETY: the Vec holds at least `len` initialised bytes
            Blob::Words(w, len) => unsafe { std::slice::from_raw_parts(w.as_ptr() as *const u8, *len) },
            Blob::Map(m) => m,
        }
    }
}

/// Id-keyed table with the FxHashMap-like API the engine uses, read in place from the snapshot bytes: the header
/// arrays (ids, aux, entry offsets, dense id index) are u32 slices of the mapped file and each entry stays
/// bincode-encoded until first use (a calc touches a few hundred of ~10k types / ~3k effects).
///
/// Section layout (8-aligned, u32 LE): n, dense_len, ids[n] (sorted), aux[n], starts[n+1] (relative to the
/// section), dense[dense_len] (id -> position + 1, 0 = absent; dense_len 0 = binary search), entry bytes.
pub struct LazyTable<T> {
    blob: std::sync::Arc<Blob>,
    ids: *const u32,
    aux: *const u32,
    starts: *const u32,
    dense: *const u32,
    n: usize,
    dense_len: usize,
    at: usize,
    /// decoded entries (null = not yet); zero-initialised, published with a CAS
    slots: Box<[std::sync::atomic::AtomicPtr<T>]>,
}

// SAFETY: the raw pointers point into `blob`, which is immutable and owned (Arc) by the table.
unsafe impl<T: Send + Sync> Send for LazyTable<T> {}
unsafe impl<T: Send + Sync> Sync for LazyTable<T> {}

pub type TypeTable = LazyTable<TypeInfo>;

#[cfg(not(target_endian = "little"))]
compile_error!("the snapshot format reads little-endian u32 arrays in place");

fn zeroed_slots<T>(n: usize) -> Box<[std::sync::atomic::AtomicPtr<T>]> {
    if n == 0 {
        return Box::new([]);
    }
    let layout = std::alloc::Layout::array::<std::sync::atomic::AtomicPtr<T>>(n).expect("layout");
    // SAFETY: AtomicPtr<T> has the representation of *mut T, for which all-zero bytes is a valid (null) value;
    // the allocation has exactly the layout of a [AtomicPtr<T>; n] and is handed to Box with that layout.
    unsafe {
        let p = std::alloc::alloc_zeroed(layout) as *mut std::sync::atomic::AtomicPtr<T>;
        if p.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        Box::from_raw(std::ptr::slice_from_raw_parts_mut(p, n))
    }
}

impl<T> Drop for LazyTable<T> {
    fn drop(&mut self) {
        for s in self.slots.iter() {
            let p = s.load(std::sync::atomic::Ordering::Acquire);
            if !p.is_null() {
                // SAFETY: every non-null slot holds a pointer from Box::into_raw, published once.
                drop(unsafe { Box::from_raw(p) });
            }
        }
    }
}

impl<T: serde::Serialize + serde::de::DeserializeOwned> LazyTable<T> {
    /// encode a section from (id, aux, entry) triples
    fn encode_entries<'x>(mut v: Vec<(u32, u32, &'x T)>) -> Result<Vec<u8>, String>
    where
        T: 'x,
    {
        v.sort_unstable_by_key(|x| x.0);
        let n = v.len();
        let max = v.last().map(|x| x.0 as usize).unwrap_or(0);
        let dense_len = if n == 0 || max >= 1 << 24 { 0 } else { max + 1 };
        let mut body = Vec::new();
        let mut starts = Vec::with_capacity(n + 1);
        let head = 4 * (2 + n + n + (n + 1) + dense_len);
        for (_, _, t) in &v {
            starts.push((head + body.len()) as u32);
            bincode::serialize_into(&mut body, t).map_err(|e| e.to_string())?;
        }
        starts.push((head + body.len()) as u32);
        let mut dense = vec![0u32; dense_len];
        if dense_len > 0 {
            for (p, x) in v.iter().enumerate() {
                dense[x.0 as usize] = p as u32 + 1;
            }
        }
        let mut out = Vec::with_capacity(head + body.len());
        let mut w = |x: u32| out.extend_from_slice(&x.to_le_bytes());
        w(n as u32);
        w(dense_len as u32);
        v.iter().for_each(|x| w(x.0));
        v.iter().for_each(|x| w(x.1));
        starts.iter().for_each(|&x| w(x));
        dense.iter().for_each(|&x| w(x));
        out.extend_from_slice(&body);
        Ok(out)
    }

    fn from_map(m: FxHashMap<u32, T>, aux: impl Fn(&T) -> u32) -> LazyTable<T> {
        let v: Vec<(u32, u32, &T)> = m.iter().map(|(k, t)| (*k, aux(t), t)).collect();
        let bytes = Self::encode_entries(v).expect("encode table");
        let end = bytes.len();
        let blob = std::sync::Arc::new(Blob::owned(&bytes));
        let t = Self::decode(&blob, 0, end).expect("table");
        // keep the already-built values instead of re-decoding them
        let mut m = m;
        for p in 0..t.n {
            let id = t.id_at(p);
            if let Some(x) = m.remove(&id) {
                t.slots[p].store(Box::into_raw(Box::new(x)), std::sync::atomic::Ordering::Release);
            }
        }
        t
    }

    /// section bytes for (id, entry) pairs (aux = 0)
    pub(crate) fn encode_pairs(v: &[(u32, T)]) -> Result<Vec<u8>, String> {
        Self::encode_entries(v.iter().map(|(k, t)| (*k, 0, t)).collect())
    }

    fn encode(&self) -> Result<Vec<u8>, String> {
        Self::encode_entries((0..self.n).map(|p| (self.id_at(p), self.aux_at(p), self.at(p))).collect())
    }

    fn decode(blob: &std::sync::Arc<Blob>, at: usize, end: usize) -> Option<LazyTable<T>> {
        let b: &[u8] = blob;
        if at % 4 != 0 || (b.as_ptr() as usize) % 4 != 0 || end > b.len() {
            return None;
        }
        let u = |o: usize| -> Option<usize> { Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize) };
        let n = u(at)?;
        let dense_len = u(at + 4)?;
        let words = 2usize.checked_add(n.checked_mul(3)?)?.checked_add(1)?.checked_add(dense_len)?;
        if at.checked_add(words.checked_mul(4)?)? > end {
            return None;
        }
        let base = b.as_ptr();
        // SAFETY: in bounds (checked above) and 4-aligned (blob base and `at` are 4-aligned)
        let ptr = |w: usize| unsafe { base.add(at + 4 * w) as *const u32 };
        let t = LazyTable {
            blob: blob.clone(),
            ids: ptr(2),
            aux: ptr(2 + n),
            starts: ptr(2 + 2 * n),
            dense: ptr(3 + 3 * n),
            n,
            dense_len,
            at,
            slots: zeroed_slots(n),
        };
        // per-entry structure (sorted ids, monotonic offsets) is not walked at start-up: `pos` with a dense index
        // re-checks the id, the binary-search fallback only needs sorted ids from our own writer, and entry bytes
        // are cut with bounds-checked slices (a corrupt file panics in `decode_slot`, it never reads out of bounds)
        let _ = end;
        Some(t)
    }

    #[inline]
    fn id_at(&self, p: usize) -> u32 {
        debug_assert!(p < self.n);
        // SAFETY: p < n, array validated at decode
        unsafe { *self.ids.add(p) }
    }
    #[inline]
    fn aux_at(&self, p: usize) -> u32 {
        unsafe { *self.aux.add(p) }
    }
    #[inline]
    fn start(&self, p: usize) -> usize {
        unsafe { *self.starts.add(p) as usize }
    }

    #[inline]
    fn pos(&self, id: u32) -> Option<usize> {
        if self.dense_len > 0 {
            if (id as usize) < self.dense_len {
                // SAFETY: index < dense_len
                let p = unsafe { *self.dense.add(id as usize) } as usize;
                if p != 0 && p <= self.n && self.id_at(p - 1) == id {
                    return Some(p - 1);
                }
            }
            None
        } else {
            let (mut lo, mut hi) = (0usize, self.n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                match self.id_at(mid).cmp(&id) {
                    std::cmp::Ordering::Less => lo = mid + 1,
                    std::cmp::Ordering::Greater => hi = mid,
                    std::cmp::Ordering::Equal => return Some(mid),
                }
            }
            None
        }
    }

    #[inline]
    fn at(&self, p: usize) -> &T {
        use std::sync::atomic::Ordering;
        let cur = self.slots[p].load(Ordering::Acquire);
        if !cur.is_null() {
            // SAFETY: published Box pointer, never freed while the table lives
            return unsafe { &*cur };
        }
        self.decode_slot(p)
    }

    #[cold]
    fn decode_slot(&self, p: usize) -> &T {
        use std::sync::atomic::Ordering;
        let (s, e) = (self.at + self.start(p), self.at + self.start(p + 1));
        let v: T = bincode::deserialize(&self.blob[s..e]).expect("corrupt dataset snapshot");
        let new = Box::into_raw(Box::new(v));
        match self.slots[p].compare_exchange(std::ptr::null_mut(), new, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => unsafe { &*new },
            Err(existing) => {
                drop(unsafe { Box::from_raw(new) });
                unsafe { &*existing }
            }
        }
    }

    #[inline]
    pub fn get(&self, id: &u32) -> Option<&T> {
        self.pos(*id).map(|p| self.at(p))
    }
    pub fn contains_key(&self, id: &u32) -> bool {
        self.pos(*id).is_some()
    }
    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    /// all entries in id order (decodes every entry)
    pub fn iter(&self) -> impl Iterator<Item = (u32, &T)> + '_ {
        (0..self.n).map(move |p| (self.id_at(p), self.at(p)))
    }
    pub fn values(&self) -> impl Iterator<Item = &T> + '_ {
        (0..self.n).map(move |p| self.at(p))
    }
}

impl LazyTable<TypeInfo> {
    /// type ids of one group, without decoding other types
    pub fn ids_in_group(&self, group: u32) -> impl Iterator<Item = u32> + '_ {
        (0..self.n).filter(move |&p| self.aux_at(p) == group).map(move |p| self.id_at(p))
    }
}

impl<T: serde::Serialize + serde::de::DeserializeOwned> std::ops::Index<&u32> for LazyTable<T> {
    type Output = T;
    fn index(&self, id: &u32) -> &T {
        self.get(id).expect("unknown id")
    }
}

/// name -> id lookup over an open-addressing table stored in the snapshot (no allocation at load).
/// Layout: u32 cap (power of two), cap x u32 (entry index + 1, 0 = empty), u32 n, n x (off, len, id), name bytes.
pub struct NameIndex {
    blob: std::sync::Arc<Blob>,
    at: usize,
    cap: usize,
    entries: usize,
    names: usize,
}

/// Stable (snapshot-persisted) string hash: 8 bytes per step, multiply-rotate (FxHash-style) + final mix.
#[inline]
fn fnv1a(b: &[u8]) -> u32 {
    const K: u64 = 0xf135_7aea_2e62_a9c5;
    let mut h: u64 = b.len() as u64;
    let mut c = b.chunks_exact(8);
    for w in &mut c {
        h = (h ^ u64::from_le_bytes(w.try_into().unwrap())).wrapping_mul(K).rotate_left(26);
    }
    let r = c.remainder();
    if !r.is_empty() {
        let mut t = [0u8; 8];
        t[..r.len()].copy_from_slice(r);
        h = (h ^ u64::from_le_bytes(t)).wrapping_mul(K).rotate_left(26);
    }
    h ^= h >> 29;
    h = h.wrapping_mul(K);
    (h ^ (h >> 32)) as u32
}

impl NameIndex {
    fn encode(m: &FxHashMap<String, u32>) -> Vec<u8> {
        let mut v: Vec<(&String, u32)> = m.iter().map(|(k, v)| (k, *v)).collect();
        v.sort();
        let cap = (v.len() * 2).next_power_of_two().max(8);
        let mut table = vec![0u32; cap];
        let mut bytes = Vec::new();
        let mut ents = Vec::with_capacity(v.len());
        for (i, (k, id)) in v.iter().enumerate() {
            let mut h = fnv1a(k.as_bytes()) as usize & (cap - 1);
            while table[h] != 0 {
                h = (h + 1) & (cap - 1);
            }
            table[h] = i as u32 + 1;
            ents.push((bytes.len() as u32, k.len() as u32, *id));
            bytes.extend_from_slice(k.as_bytes());
        }
        let mut out = Vec::with_capacity(8 + cap * 4 + ents.len() * 12 + bytes.len());
        out.extend_from_slice(&(cap as u32).to_le_bytes());
        for t in table {
            out.extend_from_slice(&t.to_le_bytes());
        }
        out.extend_from_slice(&(ents.len() as u32).to_le_bytes());
        for (o, l, id) in ents {
            for x in [o, l, id] {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        out.extend_from_slice(&bytes);
        out
    }

    fn decode(blob: &std::sync::Arc<Blob>, at: usize, end: usize) -> Option<NameIndex> {
        let b: &[u8] = blob;
        let u = |o: usize| -> Option<usize> { Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize) };
        let cap = u(at)?;
        if !cap.is_power_of_two() {
            return None;
        }
        let n_at = at + 4 + cap * 4;
        let n = u(n_at)?;
        let entries = n_at + 4;
        let names = entries + n * 12;
        if names > end {
            return None;
        }
        // entries are not validated up front (that touched every page of the index at start-up): `get` uses
        // bounds-checked slices, so a corrupt index can only fail a lookup loudly, never read out of bounds
        Some(NameIndex { blob: blob.clone(), at, cap, entries, names })
    }

    #[inline]
    fn rd(&self, o: usize) -> usize {
        u32::from_le_bytes(self.blob[o..o + 4].try_into().unwrap()) as usize
    }

    #[inline]
    pub fn get(&self, name: &str) -> Option<u32> {
        let key = name.as_bytes();
        let mut h = fnv1a(key) as usize & (self.cap - 1);
        loop {
            let e = self.rd(self.at + 4 + h * 4);
            if e == 0 {
                return None;
            }
            let eo = self.entries + (e - 1) * 12;
            let (o, l) = (self.rd(eo), self.rd(eo + 4));
            if l == key.len() && &self.blob[self.names + o..self.names + o + l] == key {
                return Some(self.rd(eo + 8) as u32);
            }
            h = (h + 1) & (self.cap - 1);
        }
    }

    fn from_map(m: &FxHashMap<String, u32>) -> NameIndex {
        let bytes = NameIndex::encode(m);
        let end = bytes.len();
        let blob = std::sync::Arc::new(Blob::owned(&bytes));
        NameIndex::decode(&blob, 0, end).expect("name index")
    }
}

#[derive(Default, serde::Serialize, Deserialize)]
struct Names {
    zh: FxHashMap<u32, String>,
    type_by_name: FxHashMap<String, u32>,
}

/// The eagerly decoded part of a snapshot (everything else is a lazily decoded section).
#[derive(serde::Serialize, Deserialize)]
struct Snapshot {
    build: u64,
    release_date: Option<String>,
    sha256: String,
    categories: FxHashMap<u32, String>,
    skills: Vec<u32>,
    skills_foldable: bool,
}

const SNAP_MAGIC: &[u8; 8] = b"EVEDVB04";
