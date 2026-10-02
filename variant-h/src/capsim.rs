//! Event-driven capacitor simulation (observable behaviour matches Pyfa's capSim: same event ordering,
//! module grouping, injector handling, wrap-around stability detection and EVE's own stability estimate).
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drain {
    pub duration: f64,
    pub cap_need: f64,
    pub clip_size: u32,
    pub reload_ms: f64,
    pub is_injector: bool,
    pub disable_stagger: bool,
}

pub struct CapResult {
    pub stable: bool,
    pub stable_low: f64,
    pub stable_high: f64,
    pub t_s: f64,
    pub eve_stable: f64,
    pub iterations: u64,
}

#[derive(Debug, Clone, Copy)]
struct Ev {
    t: f64,
    duration: f64,
    cap_need: f64,
    shot: u32,
    clip: u32,
    reload: f64,
    inj: bool,
    seq: u64,
    /// rank of (duration, cap_need) among this run's event templates
    r1: u16,
    /// rank of (clip, reload, inj) among this run's event templates
    r2: u16,
}

impl Ev {
    /// ascending key: Python list order [t, duration, capNeed, shot, clip, reload, isInjector] then insertion
    /// (template fields never change after creation, so they are compared through precomputed ranks)
    #[inline]
    fn key_cmp(&self, o: &Self) -> Ordering {
        self.t
            .total_cmp(&o.t)
            .then(self.r1.cmp(&o.r1))
            .then(self.shot.cmp(&o.shot))
            .then(self.r2.cmp(&o.r2))
            .then(self.seq.cmp(&o.seq))
    }
    fn advance(&mut self, now: f64, seq: &mut u64) {
        self.t = now + self.duration;
        self.shot += 1;
        if self.clip > 0 && self.shot % self.clip == 0 {
            self.shot = 0;
            self.t += self.reload;
        }
        self.seq = *seq;
        *seq += 1;
    }
}
impl PartialEq for Ev {
    fn eq(&self, o: &Self) -> bool {
        self.key_cmp(o) == Ordering::Equal
    }
}
impl Eq for Ev {}
impl PartialOrd for Ev {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Ev {
    fn cmp(&self, o: &Self) -> Ordering {
        o.key_cmp(self) // BinaryHeap is a max-heap
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

pub fn simulate(capacity: f64, recharge_ms: f64, drains: &[Drain], reload: bool, stagger: bool, t_max_ms: f64) -> CapResult {
    let tau = recharge_ms / 5.0;
    let mut heap: Vec<Ev> = Vec::new();
    let mut seq = 0u64;
    let mut period: u64 = 1;
    let mut disable_period = false;
    let mut groups: Vec<(Drain, u32)> = Vec::new();
    for d in drains {
        let mut d = *d;
        if !reload && !d.is_injector {
            d.clip_size = 0;
            d.reload_ms = 0.0;
        }
        if d.duration <= 0.0 {
            continue;
        }
        match groups.iter_mut().find(|(x, _)| *x == d) {
            Some(g) => g.1 += 1,
            None => groups.push((d, 1)),
        }
    }
    let ev = |t: f64, d: &Drain, inj: bool, seq: &mut u64| {
        let e = Ev { t, duration: d.duration, cap_need: d.cap_need, shot: 0, clip: d.clip_size, reload: d.reload_ms, inj, seq: *seq, r1: 0, r2: 0 };
        *seq += 1;
        e
    };
    for (d, n) in &groups {
        let mut d = *d;
        if d.clip_size > 0 {
            disable_period = true;
        }
        if d.is_injector {
            for _ in 0..*n {
                heap.push(ev(0.0, &d, true, &mut seq));
            }
            continue;
        }
        if stagger && !d.disable_stagger {
            if d.clip_size == 0 {
                d.duration = (d.duration / *n as f64).floor();
            } else {
                let st = (d.duration * d.clip_size as f64 + d.reload_ms) / (*n as f64 * d.clip_size as f64);
                for i in 1..*n {
                    heap.push(ev(i as f64 * st, &d, false, &mut seq));
                }
            }
        } else {
            d.cap_need *= *n as f64;
        }
        let dur = d.duration.round().max(1.0) as u64;
        period = period / gcd(period, dur) * dur;
        heap.push(ev(0.0, &d, false, &mut seq));
    }
    {
        let f = |a: f64, b: f64| a.partial_cmp(&b).unwrap_or(Ordering::Equal);
        let mut k1: Vec<(f64, f64)> = heap.iter().map(|e| (e.duration, e.cap_need)).collect();
        k1.sort_by(|a, b| f(a.0, b.0).then(f(a.1, b.1)));
        k1.dedup_by(|a, b| f(a.0, b.0).then(f(a.1, b.1)) == Ordering::Equal);
        let mut k2: Vec<(u32, f64, bool)> = heap.iter().map(|e| (e.clip, e.reload, e.inj)).collect();
        k2.sort_by(|a, b| a.0.cmp(&b.0).then(f(a.1, b.1)).then(a.2.cmp(&b.2)));
        k2.dedup_by(|a, b| a.0.cmp(&b.0).then(f(a.1, b.1)).then(a.2.cmp(&b.2)) == Ordering::Equal);
        for e in heap.iter_mut() {
            e.r1 = k1.partition_point(|x| f(x.0, e.duration).then(f(x.1, e.cap_need)) == Ordering::Less) as u16;
            e.r2 = k2.partition_point(|x| x.0.cmp(&e.clip).then(f(x.1, e.reload)).then(x.2.cmp(&e.inj)) == Ordering::Less) as u16;
        }
    }
    let mut heap = BinaryHeap::from(heap);
    let period = if disable_period || period as f64 > t_max_ms { t_max_ms } else { period as f64 };

    let cap_max = capacity;
    let mut cap = capacity;
    let mut cap_wrap = cap;
    let mut cap_lowest = cap;
    let mut cap_lowest_pre = cap;
    let mut t_wrap = period;
    let mut t_last = 0.0f64;
    let mut iterations = 0u64;
    let mut awaiting: Vec<Ev> = Vec::new();
    let mut awaiting_wrap: Vec<(u64, u64)> = Vec::new();
    let mut ran_out = false;
    let key = |v: &Vec<Ev>| {
        let mut k: Vec<(u64, u64)> = v.iter().map(|e| (e.duration.to_bits(), e.cap_need.to_bits())).collect();
        k.sort();
        k
    };
    let mut last_ev: Option<Ev> = None;
    // The current event stays at the heap top while it is processed (anything pushed meanwhile sorts after it:
    // later time or, at equal time, a newer sequence number), so it is re-armed in place with a single sift-down
    // instead of pop + push.
    while let Some(&top) = heap.peek() {
        let mut e = top;
        let now = e.t;
        if now >= t_max_ms {
            heap.pop();
            last_ev = Some(e);
            break;
        }
        if now > t_last && cap_max > 0.0 && tau > 0.0 {
            let x = (cap / cap_max).max(0.0).sqrt();
            cap = (1.0 + (x - 1.0) * ((t_last - now) / tau).exp()).powi(2) * cap_max;
        }
        if now != t_last {
            if cap < cap_lowest_pre {
                cap_lowest_pre = cap;
            }
            if now == t_wrap {
                let k = key(&awaiting);
                if cap >= cap_wrap && k == awaiting_wrap {
                    heap.pop();
                    last_ev = Some(e);
                    break;
                }
                cap_wrap = (cap * 10.0).round() / 10.0;
                awaiting_wrap = k;
                t_wrap += period;
            }
        }
        t_last = now;
        iterations += 1;
        if iterations > 5_000_000 {
            heap.pop();
            last_ev = Some(e);
            break;
        }
        if e.inj && cap - e.cap_need > cap_max {
            heap.pop();
            awaiting.push(e);
            continue;
        }
        if e.cap_need > cap && cap < cap_max {
            while !awaiting.is_empty() && e.cap_need > cap && cap_max > cap {
                let need = (e.cap_need - cap).min(cap_max - cap);
                let gain = |i: usize| -awaiting[i].cap_need;
                let good: Vec<usize> = (0..awaiting.len()).filter(|&i| gain(i) >= need).collect();
                let pick = if !good.is_empty() {
                    *good.iter().min_by(|&&x, &&y| gain(x).partial_cmp(&gain(y)).unwrap()).unwrap()
                } else {
                    (0..awaiting.len()).max_by(|&x, &y| gain(x).partial_cmp(&gain(y)).unwrap()).unwrap()
                };
                let mut inj = awaiting.remove(pick);
                cap = (cap - inj.cap_need).min(cap_max);
                inj.advance(now, &mut seq);
                heap.push(inj);
            }
        }
        cap = (cap - e.cap_need).min(cap_max);
        if cap < cap_lowest {
            if cap < 0.0 {
                heap.pop();
                ran_out = true;
                last_ev = Some(e);
                break;
            }
            cap_lowest = cap;
        }
        while !awaiting.is_empty() && cap < cap_max {
            let need = cap_max - cap;
            let gain = |i: usize| -awaiting[i].cap_need;
            let good: Vec<usize> = (0..awaiting.len()).filter(|&i| gain(i) <= need).collect();
            if good.is_empty() {
                break;
            }
            let pick = *good.iter().max_by(|&&x, &&y| gain(x).partial_cmp(&gain(y)).unwrap()).unwrap();
            let mut inj = awaiting.remove(pick);
            cap = (cap - inj.cap_need).min(cap_max);
            inj.advance(now, &mut seq);
            heap.push(inj);
        }
        e.advance(now, &mut seq);
        *heap.peek_mut().unwrap() = e;
    }
    let mut all: Vec<Ev> = heap.into_vec();
    if let Some(e) = last_ev {
        all.push(e);
    }
    let avg_drain: f64 = all.iter().map(|e| e.cap_need / e.duration).sum();
    let inner = -(2.0 * avg_drain * tau - cap_max) / cap_max;
    let eve_stable = if inner >= 0.0 && cap_max > 0.0 { 0.25 * (1.0 + inner.sqrt()).powi(2) } else { 0.0 };
    let stable = !ran_out;
    CapResult {
        stable,
        stable_low: if stable && cap_max > 0.0 { cap_lowest / cap_max } else { 0.0 },
        stable_high: if stable && cap_max > 0.0 { cap_lowest_pre / cap_max } else { 0.0 },
        t_s: t_last / 1000.0,
        eve_stable,
        iterations,
    }
}
