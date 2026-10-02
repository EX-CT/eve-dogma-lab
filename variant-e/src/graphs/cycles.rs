//! Cycle sequences (Pyfa eos.utils.cycles CycleInfo / CycleSequence) for time-axis graphs.
use crate::eos::cx::{Fit, It};

/// (active ms, inactive ms, quantity, is inactivity reload)
pub type Part = (f64, f64, f64, bool);

#[derive(Clone, Debug)]
pub struct Cycles {
    pub seq: Vec<Part>,
    pub repeat: f64,
}

impl Cycles {
    pub fn single(active: f64, inactive: f64, qty: f64, reload: bool) -> Cycles {
        Cycles { seq: vec![(active, inactive, qty, reload)], repeat: 1.0 }
    }
    pub fn average(&self) -> f64 {
        let t: f64 = self.seq.iter().map(|p| (p.0 + p.1) * p.2).sum();
        let q: f64 = self.seq.iter().map(|p| p.2).sum();
        if self.seq.len() == 1 && self.repeat == 1.0 {
            return self.seq[0].0 + self.seq[0].1;
        }
        t / q
    }
    /// every cycle in order: (active ms, inactive ms, is inactivity reload); infinite when a quantity is infinite
    pub fn iter(&self) -> impl Iterator<Item = (f64, f64, bool)> + '_ {
        let mut rep = 0.0;
        let mut part = 0usize;
        let mut i = 0.0;
        std::iter::from_fn(move || {
            loop {
                if rep >= self.repeat {
                    return None;
                }
                if part >= self.seq.len() {
                    part = 0;
                    rep += 1.0;
                    continue;
                }
                let p = self.seq[part];
                if i < p.2 {
                    i += 1.0;
                    return Some((p.0, p.1, p.3));
                }
                i = 0.0;
                part += 1;
            }
        })
    }
}

/// Module.getCycleParameters(reloadOverride)
pub fn module_cycles(fit: &Fit, m: It, reload_override: Option<bool>) -> Option<Cycles> {
    let factor = reload_override.unwrap_or(fit.items[m].force_reload.unwrap_or(fit.factor_reload));
    let mut until = fit.num_shots(m);
    if until == 0.0 {
        until = f64::INFINITY;
    }
    let active = fit.raw_cycle_time(m);
    if active == 0.0 {
        return None;
    }
    let inactive = fit.g(m, "moduleReactivationDelay");
    let reload = fit.reload_time(m);
    if !factor || until == f64::INFINITY || inactive >= reload {
        return Some(Cycles::single(active, inactive, f64::INFINITY, factor && inactive >= reload));
    }
    let early = until - 1.0;
    if early == 0.0 {
        return Some(Cycles::single(active, reload, f64::INFINITY, true));
    }
    Some(Cycles { seq: vec![(active, inactive, early, false), (active, reload, 1.0, true)], repeat: f64::INFINITY })
}
