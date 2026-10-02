//! Salsa database + session: maps each request onto persistent inputs (diffed item by item), so a
//! sequence of related requests (batch, RPC, an editor) only recomputes what changed.
use crate::data::Dataset;
use crate::engine::{self, Consts, Ctx, Db, FitIn, ItemIn, F};
use crate::request::{FitRequest, Resists};
use crate::spec::{self, ItemKey, ItemSpec, Kind};
use rustc_hash::FxHashMap;
use salsa::Setter;
use serde_json::{json, Value};
use std::sync::Arc;

#[salsa::db]
#[derive(Clone)]
pub struct EngineDb {
    storage: salsa::Storage<Self>,
    ds: Arc<Dataset>,
    consts: Arc<Consts>,
}

#[salsa::db]
impl salsa::Database for EngineDb {}

#[salsa::db]
impl Db for EngineDb {
    fn ds(&self) -> &Dataset {
        &self.ds
    }
    fn consts(&self) -> &Consts {
        &self.consts
    }
}

pub struct Session {
    pub db: EngineDb,
    fit: Option<FitIn>,
    slots: Vec<ItemIn>,
    slot_of: FxHashMap<ItemKey, u32>,
    /// requests since the database was (re)created; used to bound memory
    pub calcs: u64,
    pub max_slots: usize,
}

/// Facade item consumed by stats (canonical order; indices are canonical positions).
#[derive(Debug, Clone)]
pub struct Item {
    pub type_id: u32,
    pub group: u32,
    pub category: u32,
    pub kind: Kind,
    pub state: crate::request::State,
    pub owned: bool,
    pub parent: Option<usize>,
    pub charge: Option<usize>,
    pub slot: Option<crate::request::Slot>,
    pub req_index: Option<usize>,
    pub quantity: u32,
    pub active_count: u32,
    pub effects: Vec<(u32, bool)>,
    pub fighter_abilities: Option<Vec<u32>>,
    pub spool: Option<crate::request::Spool>,
    pub skill_level: f64,
}

/// Read view of one evaluated fit (what stats.rs computes on).
pub struct Fit<'a> {
    pub ds: &'a Dataset,
    pub db: &'a EngineDb,
    pub fit: FitIn,
    pub items: Vec<Item>,
    pub slot_of: Vec<u32>,
    pub ship: usize,
    pub char: usize,
    pub warnings: Vec<String>,
    pub is_structure: bool,
    pub layer: u32,
}

impl<'a> Fit<'a> {
    #[inline]
    pub fn get(&self, item: usize, attr: u32) -> f64 {
        engine::value(self.db, self.fit, self.slot_of[item], attr, self.layer)
    }
    pub fn get_opt(&self, item: usize, attr: u32) -> Option<f64> {
        if self.has(item, attr) { Some(self.get(item, attr)) } else { None }
    }
    pub fn has(&self, item: usize, attr: u32) -> bool {
        let s = self.slot_of[item];
        if engine::has(self.db, self.fit, s, attr) {
            return true;
        }
        (1..=self.layer).any(|l| engine::layer_mods(self.db, self.fit, engine::LKey::new(self.db, l)).contains_key(&(s, attr)))
    }
    pub fn base(&self, item: usize, attr: u32) -> f64 {
        let it = self.fit.slots(self.db)[self.slot_of[item] as usize];
        it.spec(self.db).base(attr).unwrap_or_else(|| self.ds.attr_default(attr))
    }
    pub fn attr_keys(&self, item: usize) -> Vec<u32> {
        let s = self.slot_of[item];
        let it = self.fit.slots(self.db)[s as usize];
        let mut k: Vec<u32> = it.spec(self.db).attrs.iter().map(|x| x.0).collect();
        k.extend(engine::item_mods(self.db, self.fit, it).keys().copied());
        for l in 1..=self.layer {
            for (t, a) in engine::layer_mods(self.db, self.fit, engine::LKey::new(self.db, l)).keys() {
                if *t == s {
                    k.push(*a);
                }
            }
        }
        k.sort_unstable();
        k.dedup();
        k
    }
}

impl Session {
    pub fn new(ds: Arc<Dataset>) -> Session {
        let consts = Arc::new(Consts::new(&ds));
        Session { db: EngineDb { storage: salsa::Storage::default(), ds, consts }, fit: None, slots: Vec::new(), slot_of: FxHashMap::default(), calcs: 0, max_slots: 20_000 }
    }

    pub fn ds(&self) -> &Dataset {
        &self.db.ds
    }

    /// Drop all memoised state (keeps the dataset).
    pub fn reset(&mut self) {
        let ds = self.db.ds.clone();
        *self = Session::new(ds);
    }

    /// Full stats for one request. Output is a pure function of (dataset, request).
    pub fn calc(&mut self, req: &FitRequest) -> Value {
        if self.slots.len() > self.max_slots {
            self.reset();
        }
        self.calcs += 1;
        let built = match spec::build(&self.db.ds, req) {
            Ok(b) => b,
            Err(e) => return json!({"error": {"code": e.code, "message": e.message, "path": e.path}}),
        };
        let fit = self.load(req, &built);
        let ds: &Dataset = &self.db.ds;
        let db = &self.db;
        let n = built.items.len();
        let slot_of: Vec<u32> = built.keys.iter().map(|k| self.slot_of[k]).collect();
        let mut warnings = built.warnings.clone();
        for it in &built.items {
            if it.kind == Kind::Projected {
                warnings.extend(engine::projected_warnings(ds, it));
            }
        }
        warnings.extend(buff_warnings(ds, req));
        let items: Vec<Item> = built
            .items
            .iter()
            .map(|s| Item {
                type_id: s.type_id,
                group: s.group,
                category: s.category,
                kind: s.kind,
                state: s.state,
                owned: s.owned,
                parent: s.parent,
                charge: s.charge,
                slot: s.slot,
                req_index: s.req_index,
                quantity: s.quantity,
                active_count: s.active_count,
                effects: s.effects.clone(),
                fighter_abilities: s.fighter_abilities.clone(),
                spool: s.spool,
                skill_level: s.base(spec::ATTR_SKILL_LEVEL).unwrap_or(0.0),
            })
            .collect();
        let layer = engine::plan(db, fit).final_layer;
        let view = Fit { ds, db, fit, items, slot_of, ship: 0, char: 1.min(n - 1), warnings, is_structure: built.is_structure, layer };
        view.compute_stats(req)
    }

    fn load(&mut self, req: &FitRequest, b: &spec::Built) -> FitIn {
        // allocate slots for new keys
        for k in &b.keys {
            if !self.slot_of.contains_key(k) {
                let s = self.slots.len() as u32;
                self.slot_of.insert(*k, s);
                let placeholder = Arc::new(b.items[0].clone());
                self.slots.push(ItemIn::new(&self.db, s, placeholder));
            }
        }
        let slot_of: Vec<u32> = b.keys.iter().map(|k| self.slot_of[k]).collect();
        for (i, sp) in b.items.iter().enumerate() {
            let mut sp: ItemSpec = sp.clone();
            sp.parent = sp.parent.map(|p| slot_of[p] as usize);
            sp.charge = sp.charge.map(|c| slot_of[c] as usize);
            let h = self.slots[slot_of[i] as usize];
            if **h.spec(&self.db) != sp {
                h.set_spec(&mut self.db).to(Arc::new(sp));
            }
        }
        let ctx = Arc::new(make_ctx(&self.db.ds, req, slot_of[0], slot_of.get(1).copied().unwrap_or(slot_of[0]), b.is_structure));
        match self.fit {
            None => {
                let f = FitIn::new(&self.db, self.slots.clone(), slot_of, ctx);
                self.fit = Some(f);
                f
            }
            Some(f) => {
                if f.slots(&self.db).len() != self.slots.len() {
                    f.set_slots(&mut self.db).to(self.slots.clone());
                }
                if *f.order(&self.db) != slot_of {
                    f.set_order(&mut self.db).to(slot_of);
                }
                if **f.ctx(&self.db) != *ctx {
                    f.set_ctx(&mut self.db).to(ctx);
                }
                f
            }
        }
    }
}

fn make_ctx(ds: &Dataset, req: &FitRequest, ship: u32, ch: u32, is_structure: bool) -> Ctx {
    let mut agg: FxHashMap<u32, f64> = FxHashMap::default();
    for b in &req.fleet.buffs {
        let Some(info) = ds.dbuffs.get(&b.buff_id) else { continue };
        let e = agg.entry(b.buff_id).or_insert(b.value);
        *e = match info.aggregate.as_deref() {
            Some("Minimum") => e.min(b.value),
            _ => e.max(b.value),
        };
    }
    let mut buffs: Vec<(u32, F)> = agg.into_iter().map(|(k, v)| (k, F(v))).collect();
    buffs.sort_by_key(|x| x.0);
    let dp = req.damage_pattern.unwrap_or(Resists { em: 25.0, thermal: 25.0, kinetic: 25.0, explosive: 25.0 });
    Ctx {
        ship,
        char: ch,
        is_structure,
        buffs,
        explicit_ids: req.fleet.buffs.iter().map(|b| b.buff_id).collect(),
        rah_disable: req.options.rah.as_deref() == Some("disable"),
        pattern: [F(dp.em), F(dp.thermal), F(dp.kinetic), F(dp.explosive)],
    }
}

/// One-shot convenience (fresh database per call).
pub fn calc(ds: Arc<Dataset>, req: &FitRequest) -> Value {
    Session::new(ds).calc(req)
}

/// unknown warfare buff warnings (reference emits them during registration)
pub fn buff_warnings(ds: &Dataset, req: &FitRequest) -> Vec<String> {
    req.fleet.buffs.iter().filter(|b| !ds.dbuffs.contains_key(&b.buff_id)).map(|b| format!("unknown warfare buff {}", b.buff_id)).collect()
}

impl Session {
    pub fn db_ds(&self) -> Arc<Dataset> {
        self.db.ds.clone()
    }
}
