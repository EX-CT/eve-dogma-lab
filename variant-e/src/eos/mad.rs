//! Port of Pyfa `eos/modifiedAttributeDict.py` (ModifiedAttributeDict). GPL-3.0-or-later (derived from Pyfa).
//!
//! Push model: effect handlers register modifications (preAssign / increase / multiply / penalised
//! multiply / force) into per-attribute accumulators; the value is computed lazily on read and cached until
//! the next modification of *that* attribute (Pyfa's CalculationPlaceholder), exactly like Pyfa.
use crate::data::Dataset;
use rustc_hash::FxHashMap;
use std::cell::Cell;

#[derive(Debug, Default, Clone)]
pub struct Entry {
    pub pre_assign: Option<f64>,
    pub pre_inc: f64,
    pub mult: f64,
    /// penalised multipliers grouped by penalty group, insertion order preserved
    pub pen: Vec<(u8, Vec<f64>)>,
    pub post_inc: f64,
    pub forced: Option<f64>,
    pub inter: Option<f64>,
    /// true once any modification was registered (Pyfa: key present in __modified)
    pub placeholder: bool,
    pub cache: Cell<Option<f64>>,
}

#[derive(Debug, Clone)]
pub struct Mad<'a> {
    /// item.attributes (sorted by id)
    pub base: &'a [(u32, f64)],
    /// mutators / overrides / Ship.EXTRA_ATTRIBUTES (sorted by id), take precedence over `base`
    pub over: Vec<(u32, f64)>,
    pub entries: FxHashMap<u32, Entry>,
}

const ROUND2: [u32; 4] = [50, 30, 48, 11]; // cpu, power, cpuOutput, powerOutput

pub fn py_round2(v: f64) -> f64 {
    // Python round(x, 2): correctly rounded, ties to even on the exact binary value
    let s = format!("{:.2}", v);
    s.parse().unwrap_or(v)
}

impl<'a> Mad<'a> {
    pub fn new(base: &'a [(u32, f64)]) -> Mad<'a> {
        Mad { base, over: Vec::new(), entries: FxHashMap::default() }
    }

    pub fn set_over(&mut self, a: u32, v: f64) {
        match self.over.binary_search_by_key(&a, |x| x.0) {
            Ok(i) => self.over[i].1 = v,
            Err(i) => self.over.insert(i, (a, v)),
        }
    }

    /// getOriginal (None if the item does not have it and the attribute is unknown)
    #[inline]
    pub fn original(&self, a: u32, ds: &Dataset) -> Option<f64> {
        if !self.over.is_empty() {
            if let Ok(i) = self.over.binary_search_by_key(&a, |x| x.0) {
                return Some(self.over[i].1);
            }
        }
        if let Ok(i) = self.base.binary_search_by_key(&a, |x| x.0) {
            return Some(self.base[i].1);
        }
        ds.attrs.get(&a).map(|x| x.default)
    }

    pub fn in_original(&self, a: u32) -> bool {
        self.over.binary_search_by_key(&a, |x| x.0).is_ok() || self.base.binary_search_by_key(&a, |x| x.0).is_ok()
    }

    pub fn contains(&self, a: u32) -> bool {
        self.in_original(a) || self.entries.get(&a).map(|e| e.placeholder || e.inter.is_some()).unwrap_or(false)
    }

    pub fn entry(&mut self, a: u32) -> &mut Entry {
        self.entries.entry(a).or_insert_with(|| Entry { mult: 1.0, ..Default::default() })
    }

    /// __getitem__
    pub fn get(&self, a: u32, ds: &Dataset) -> Option<f64> {
        if let Some(e) = self.entries.get(&a) {
            if e.placeholder {
                if let Some(v) = e.cache.get() {
                    return Some(v);
                }
                let v = self.calculate(a, e, ds);
                e.cache.set(Some(v));
                return Some(v);
            }
            if let Some(v) = e.inter {
                return Some(v);
            }
        }
        self.original(a, ds)
    }

    fn calculate(&self, a: u32, e: &Entry, ds: &Dataset) -> f64 {
        let info = ds.attrs.get(&a);
        let min_v = info.and_then(|i| i.min_attr).and_then(|m| self.get(m, ds));
        let max_v = info.and_then(|i| i.max_attr).and_then(|m| self.get(m, ds));
        let round = ROUND2.contains(&a);
        if let Some(mut f) = e.forced {
            if let Some(m) = min_v {
                f = f.max(m);
            }
            if let Some(m) = max_v {
                f = f.min(m);
            }
            return if round { py_round2(f) } else { f };
        }
        let default = info.map(|i| i.default).unwrap_or(0.0);
        let mut val = match e.inter {
            Some(v) => v,
            None => match e.pre_assign {
                Some(v) => v,
                None => self.original(a, ds).unwrap_or(default),
            },
        };
        val += e.pre_inc;
        val *= e.mult;
        for (_, list) in &e.pen {
            let mut l1: Vec<f64> = list.iter().copied().filter(|v| *v > 1.0).collect();
            let mut l2: Vec<f64> = list.iter().copied().filter(|v| *v < 1.0).collect();
            // Python's sort is stable: key -abs(v-1)
            l1.sort_by(|x, y| (y - 1.0).abs().partial_cmp(&(x - 1.0).abs()).unwrap_or(std::cmp::Ordering::Equal));
            l2.sort_by(|x, y| (y - 1.0).abs().partial_cmp(&(x - 1.0).abs()).unwrap_or(std::cmp::Ordering::Equal));
            for l in [&l1, &l2] {
                for (i, b) in l.iter().enumerate() {
                    val *= 1.0 + (b - 1.0) * (-((i * i) as f64) / 7.1289).exp();
                }
            }
        }
        val += e.post_inc;
        if let Some(m) = min_v {
            val = val.max(m);
        }
        if let Some(m) = max_v {
            val = val.min(m);
        }
        if round { py_round2(val) } else { val }
    }
}
