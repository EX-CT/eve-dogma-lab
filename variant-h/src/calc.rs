//! Attribute calculation system: lazy, memoised evaluation of an attribute on an entity.
//! CCP operator order (PreAssign, PreMul, PreDiv, ModAdd, ModSub, PostMul, PostDiv, PostPercent, PostAssign),
//! stacking penalties for non-stackable attributes, min/max attribute caps.
use crate::components::{AttrSlot, Attrs, Src};
use crate::data::Dataset;
use hecs::{Entity, ViewBorrow, World};
use rustc_hash::FxHashMap;
use std::cell::RefCell;

/// Read-only view over all `Attrs` components + the memo table (None = being evaluated, cycle guard).
pub struct Calc<'w> {
    pub ds: &'w Dataset,
    view: ViewBorrow<'w, &'static Attrs>,
    memo: RefCell<FxHashMap<u64, Option<f64>>>,
    /// local fitted modules -> request index (fit order), for `Src::Before`
    order: FxHashMap<u32, u32>,
}

#[inline]
fn key(e: Entity, attr: u32) -> u64 {
    ((e.id() as u64) << 32) | attr as u64
}


impl<'w> Calc<'w> {
    pub fn new(ds: &'w Dataset, world: &'w World, order: FxHashMap<u32, u32>) -> Self {
        Calc { ds, view: world.view::<&Attrs>(), memo: RefCell::new(FxHashMap::default()), order }
    }

    fn src_entity(s: &Src) -> Option<Entity> {
        match *s {
            Src::Attr { e, .. } | Src::Before { e, .. } | Src::Projected { e, .. } => Some(e),
            Src::Prop { module, .. } => Some(module),
            Src::Const(_) => None,
        }
    }

    /// `attr` on `e` counting only modifiers whose source is not a local module listed after `idx` (not memoised)
    fn get_before(&self, e: Entity, attr: u32, idx: u32) -> f64 {
        let a = self.attrs(e);
        match a.slots.get(&attr) {
            Some(s) => {
                let later = |m: &crate::components::Mod| {
                    Self::src_entity(&m.src).and_then(|x| self.order.get(&x.id())).map(|&i| i > idx).unwrap_or(false)
                };
                if !s.mods.iter().any(|m| later(m)) {
                    return self.get(e, attr);
                }
                let kept = AttrSlot { base: s.base, mods: s.mods.iter().filter(|m| !later(m)).copied().collect() };
                self.eval(e, attr, &kept)
            }
            None => self.get(e, attr),
        }
    }

    #[inline]
    fn attrs(&self, e: Entity) -> &Attrs {
        self.view.get(e).expect("entity without Attrs")
    }

    #[inline]
    fn type_base(&self, a: &Attrs, attr: u32) -> Option<f64> {
        self.ds.types.get(&a.type_id).and_then(|t| t.attr(attr))
    }

    /// Final value of `attr` on `e` (dataset default if the item does not have it at all).
    pub fn get(&self, e: Entity, attr: u32) -> f64 {
        let k = key(e, attr);
        if let Some(v) = self.memo.borrow().get(&k) {
            // None: cycle - fall back to the base value
            return v.unwrap_or_else(|| self.base(e, attr));
        }
        let a = self.attrs(e);
        let v = if let Some(s) = a.slots.get(&attr) {
            self.memo.borrow_mut().insert(k, None);
            self.eval(e, attr, s)
        } else {
            match self.type_base(a, attr) {
                Some(v) => {
                    self.memo.borrow_mut().insert(k, None);
                    self.post(e, attr, v, false)
                }
                None => return self.ds.attr_default(attr),
            }
        };
        self.memo.borrow_mut().insert(k, Some(v));
        v
    }

    /// Does the item carry this attribute (type, own base or modified)?
    pub fn has(&self, e: Entity, attr: u32) -> bool {
        let a = self.attrs(e);
        a.slots.contains_key(&attr) || self.type_base(a, attr).is_some()
    }

    /// Unmodified base value.
    pub fn base(&self, e: Entity, attr: u32) -> f64 {
        let a = self.attrs(e);
        if let Some(s) = a.slots.get(&attr) {
            return s.base;
        }
        self.type_base(a, attr).unwrap_or_else(|| self.ds.attr_default(attr))
    }

    /// All attribute ids present on the item (sorted).
    pub fn attr_ids(&self, e: Entity) -> Vec<u32> {
        let a = self.attrs(e);
        let mut v: Vec<u32> = a.slots.keys().copied().collect();
        if let Some(t) = self.ds.types.get(&a.type_id) {
            v.extend(t.attrs.iter().map(|x| x.0));
        }
        v.sort();
        v.dedup();
        v
    }

    fn post(&self, e: Entity, attr: u32, mut val: f64, capped: bool) -> f64 {
        // Pyfa only caps values it calculated: an attribute with no modifiers is returned as is
        if let Some(info) = self.ds.attrs.get(&attr) {
            if let (true, Some(mn)) = (capped, info.min_attr) {
                val = val.max(self.get(e, mn));
            }
            if let (true, Some(mx)) = (capped, info.max_attr) {
                val = val.min(self.get(e, mx));
            }
            if info.round2 {
                val = py_round2(val);
            }
        }
        val
    }

    fn src_value(&self, s: &Src) -> f64 {
        match *s {
            Src::Attr { e, attr } => self.get(e, attr),
            Src::Before { e, attr, idx } => self.get_before(e, attr, idx),
            Src::Const(v) => v,
            Src::Prop { module, ship } => {
                let a = &self.ds.a;
                let m = self.get(ship, a.mass);
                if m == 0.0 { 1.0 } else { 1.0 + self.get(module, a.speed_factor) / 100.0 * self.get(module, a.speed_boost_factor) / m }
            }
            Src::Projected { e, attr, factor, target, resist, mul } => {
                let mut f = factor;
                if resist != 0 {
                    f *= self.get(target, resist);
                }
                let v = self.get(e, attr);
                if mul { (v - 1.0) * f + 1.0 } else { v * f }
            }
        }
    }

    fn eval(&self, e: Entity, attr: u32, s: &AttrSlot) -> f64 {
        // Pyfa's ModifiedAttributeDict order (the oracle, down to float rounding):
        //   preAssign > additions > one product of every unpenalised multiplier (in application order) >
        //   stacking-penalised chains (per operator) > postAssign
        let mut val = s.base;
        if !s.mods.is_empty() {
            let hig = self.ds.attrs.get(&attr).map(|i| i.high_is_good).unwrap_or(true);
            let vals: Vec<(i8, bool, f64)> = s.mods.iter().map(|m| (m.op, m.penalized, self.src_value(&m.src))).collect();
            let pick = |op: i8| {
                let mut r: Option<f64> = None;
                for &(o, _, v) in &vals {
                    if o == op {
                        r = Some(match r {
                            None => v,
                            Some(c) => {
                                if hig { c.max(v) } else { c.min(v) }
                            }
                        });
                    }
                }
                r
            };
            if let Some(v) = pick(-1) {
                val = v;
            }
            for &(o, _, v) in &vals {
                match o {
                    2 => val += v,
                    3 => val -= v,
                    _ => {}
                }
            }
            let mult = |op: i8, v: f64| match op {
                0 | 4 => v,
                1 | 5 => {
                    if v == 0.0 { 1.0 } else { 1.0 / v }
                }
                6 => 1.0 + v / 100.0,
                _ => 1.0,
            };
            let mut prod = 1.0;
            for &(o, pen, v) in &vals {
                if !pen && matches!(o, 0 | 1 | 4 | 5 | 6) {
                    prod *= mult(o, v);
                }
            }
            val *= prod;
            let mut pos: Vec<f64> = Vec::new();
            let mut neg: Vec<f64> = Vec::new();
            for op in [0i8, 1, 4, 5, 6] {
                pos.clear();
                neg.clear();
                for &(o, pen, v) in &vals {
                    if o == op && pen {
                        let m = mult(op, v);
                        if m > 1.0 {
                            pos.push(m)
                        } else if m < 1.0 {
                            neg.push(m)
                        }
                    }
                }
                for list in [&mut pos, &mut neg] {
                    list.sort_by(|x, y| (*y - 1.0).abs().partial_cmp(&(*x - 1.0).abs()).unwrap_or(std::cmp::Ordering::Equal));
                    for (i, m) in list.iter().enumerate() {
                        val *= 1.0 + (m - 1.0) * (-((i * i) as f64) / 7.1289).exp();
                    }
                }
            }
            if let Some(v) = pick(7) {
                val = v;
            }
        }
        self.post(e, attr, val, !s.mods.is_empty())
    }
}


/// Python `round(v, 2)`: correctly rounded on the exact binary value (ties to even), e.g. 28.125000000000004 ->
/// 28.13 but 28.124999999999996 -> 28.12. Away from a half-cent boundary, scaling by 100 cannot change the result;
/// near one, the correctly rounded decimal formatting decides.
#[inline]
pub fn py_round2(v: f64) -> f64 {
    let t = v * 100.0;
    let f = t - t.floor();
    if (f - 0.5).abs() > 1e-6 || !v.is_finite() {
        return t.round() / 100.0;
    }
    format!("{v:.2}").parse().unwrap_or(t.round() / 100.0)
}
