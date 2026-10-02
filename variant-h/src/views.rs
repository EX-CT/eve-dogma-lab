//! Random-access read views over the immutable per-item components (borrowed once per system, no per-access
//! borrow bookkeeping). Systems that only read the world use these instead of `World::get`.
use crate::components::*;
use crate::data::Dataset;
use crate::request::State;
use hecs::{Entity, ViewBorrow, World};

pub struct Views<'w> {
    ds: &'w Dataset,
    item: ViewBorrow<'w, &'static Item>,
    power: ViewBorrow<'w, &'static Power>,
    fitted: ViewBorrow<'w, &'static Fitted>,
    squad: ViewBorrow<'w, &'static Squad>,
    mutated: ViewBorrow<'w, &'static Mutated>,
    loaded: ViewBorrow<'w, &'static LoadedIn>,
}

impl<'w> Views<'w> {
    pub fn new(ds: &'w Dataset, w: &'w World) -> Self {
        Views {
            ds,
            item: w.view::<&Item>(),
            power: w.view::<&Power>(),
            fitted: w.view::<&Fitted>(),
            squad: w.view::<&Squad>(),
            mutated: w.view::<&Mutated>(),
            loaded: w.view::<&LoadedIn>(),
        }
    }
    #[inline]
    pub fn item(&self, e: Entity) -> Item {
        *self.item.get(e).unwrap()
    }
    #[inline]
    pub fn state(&self, e: Entity) -> State {
        self.power.get(e).unwrap().0
    }
    #[inline]
    pub fn fitted(&self, e: Entity) -> Fitted {
        *self.fitted.get(e).unwrap()
    }
    #[inline]
    pub fn fitted_opt(&self, e: Entity) -> Option<&Fitted> {
        self.fitted.get(e)
    }
    #[inline]
    pub fn charge(&self, e: Entity) -> Option<Entity> {
        self.fitted.get(e).and_then(|f| f.charge)
    }
    #[inline]
    pub fn parent(&self, e: Entity) -> Option<Entity> {
        self.loaded.get(e).map(|p| p.0)
    }
    #[inline]
    pub fn squad(&self, e: Entity) -> Squad {
        *self.squad.get(e).unwrap()
    }
    pub fn effects(&self, e: Entity) -> &[(u32, bool)] {
        if let Some(m) = self.mutated.get(e) {
            return &m.effects;
        }
        &self.ds.types[&self.item(e).type_id].effects
    }
    pub fn has_effect(&self, e: Entity, eid: u32) -> bool {
        eid != 0 && self.effects(e).iter().any(|x| x.0 == eid)
    }
    pub fn req_skills(&self, e: Entity) -> &[u32] {
        if let Some(m) = self.mutated.get(e) {
            return &m.req_skills;
        }
        &self.ds.types[&self.item(e).type_id].req_skills
    }
    pub fn type_name(&self, e: Entity) -> &'w str {
        &self.ds.types[&self.item(e).type_id].name
    }
}
