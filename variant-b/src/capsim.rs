//! Event-driven capacitor simulator (behaviour-compatible with Pyfa eos/capSim.py, LGPL).
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Debug, Clone, Copy)]
pub struct Drain {
    /// cycle duration in ms
    pub duration: f64,
    /// cap used per cycle (negative = cap injected)
    pub cap_need: f64,
    /// shots before reload (0 = infinite)
    pub clip_size: u32,
    pub reload_ms: f64,
    pub is_injector: bool,
    pub disable_stagger: bool,
}

#[derive(Debug, Clone)]
pub struct CapResult {
    pub stable: bool,
    /// lowest cap fraction reached while stable (0..1)
    pub stable_low: f64,
    pub stable_high: f64,
    /// time (s) at which the cap ran out (unstable) or simulation end
    pub t_s: f64,
    pub depletes_in_s: Option<f64>,
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
}
impl PartialEq for Ev {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
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
        // min-heap with Python-list ordering like Pyfa's heapq of
        // [t, duration, capNeed, shot, clipSize, reloadTime, isInjector], then insertion order
        // lazily chained: almost every comparison is decided by `t`
        #[inline(always)]
        fn f(a: f64, b: f64) -> Ordering {
            b.partial_cmp(&a).unwrap_or(Ordering::Equal)
        }
        f(self.t, o.t)
            .then_with(|| f(self.duration, o.duration))
            .then_with(|| f(self.cap_need, o.cap_need))
            .then_with(|| o.shot.cmp(&self.shot))
            .then_with(|| o.clip.cmp(&self.clip))
            .then_with(|| f(self.reload, o.reload))
            .then_with(|| o.inj.cmp(&self.inj))
            .then_with(|| o.seq.cmp(&self.seq))
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

pub fn simulate(capacity: f64, recharge_ms: f64, drains: &[Drain], start_frac: f64, reload: bool, stagger: bool, t_max_ms: f64) -> CapResult {
    let finite = drains.iter().all(|d| !d.duration.is_nan() && !d.cap_need.is_nan() && !d.reload_ms.is_nan());
    if finite && std::env::var_os("VB_CAPSIM_REF").is_none() {
        return simulate_fast(capacity, recharge_ms, drains, start_frac, reload, stagger, t_max_ms);
    }
    simulate_ref(capacity, recharge_ms, drains, start_frac, reload, stagger, t_max_ms)
}

/// Reference implementation (direct port of Pyfa's capSim); `simulate_fast` must match it bit for bit.
pub fn simulate_ref(capacity: f64, recharge_ms: f64, drains: &[Drain], start_frac: f64, reload: bool, stagger: bool, t_max_ms: f64) -> CapResult {
    let tau = recharge_ms / 5.0;
    let mut heap = BinaryHeap::new();
    let mut seq = 0u64;
    let mut period: u64 = 1;
    let mut disable_period = false;
    // group identical modules
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
        if let Some(g) = groups.iter_mut().find(|(x, _)| {
            x.duration == d.duration && x.cap_need == d.cap_need && x.clip_size == d.clip_size && x.reload_ms == d.reload_ms
                && x.is_injector == d.is_injector && x.disable_stagger == d.disable_stagger
        }) {
            g.1 += 1;
        } else {
            groups.push((d, 1));
        }
    }
    for (d, n) in &groups {
        let mut d = *d;
        if d.clip_size > 0 {
            disable_period = true;
        }
        if d.is_injector {
            for _ in 0..*n {
                heap.push(Ev { t: 0.0, duration: d.duration, cap_need: d.cap_need, shot: 0, clip: d.clip_size, reload: d.reload_ms, inj: true, seq });
                seq += 1;
            }
            continue;
        }
        if stagger && !d.disable_stagger {
            if d.clip_size == 0 {
                d.duration = (d.duration / *n as f64).floor();
            } else {
                let st = (d.duration * d.clip_size as f64 + d.reload_ms) / (*n as f64 * d.clip_size as f64);
                for i in 1..*n {
                    heap.push(Ev { t: i as f64 * st, duration: d.duration, cap_need: d.cap_need, shot: 0, clip: d.clip_size, reload: d.reload_ms, inj: false, seq });
                    seq += 1;
                }
            }
        } else {
            d.cap_need *= *n as f64;
        }
        let dur = d.duration.round().max(1.0) as u64;
        period = period / gcd(period, dur) * dur;
        heap.push(Ev { t: 0.0, duration: d.duration, cap_need: d.cap_need, shot: 0, clip: d.clip_size, reload: d.reload_ms, inj: false, seq });
        seq += 1;
    }
    let period = if disable_period || period as f64 > t_max_ms { t_max_ms } else { period as f64 };

    let cap_max = capacity;
    let mut cap = capacity * start_frac;
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
    let mut exp_arg = f64::NAN;
    let mut exp_val = f64::NAN;
    while let Some(mut ev) = heap.pop() {
        let t_now = ev.t;
        if t_now >= t_max_ms {
            last_ev = Some(ev);
            break;
        }
        if t_now > t_last && cap_max > 0.0 && tau > 0.0 {
            let x = (cap / cap_max).max(0.0).sqrt();
            // the same few time steps recur: memoise exp() on the exact argument (bit-identical)
            let arg = (t_last - t_now) / tau;
            let e = if arg.to_bits() == exp_arg.to_bits() {
                exp_val
            } else {
                exp_arg = arg;
                exp_val = arg.exp();
                exp_val
            };
            cap = (1.0 + (x - 1.0) * e).powi(2) * cap_max;
        }
        if t_now != t_last {
            if cap < cap_lowest_pre {
                cap_lowest_pre = cap;
            }
            if t_now == t_wrap {
                let k = key(&awaiting);
                if cap >= cap_wrap && k == awaiting_wrap {
                    last_ev = Some(ev);
                    break;
                }
                cap_wrap = (cap * 10.0).round() / 10.0;
                awaiting_wrap = k;
                t_wrap += period;
            }
        }
        t_last = t_now;
        iterations += 1;
        if iterations > 5_000_000 {
            last_ev = Some(ev);
            break;
        }
        if ev.inj && cap - ev.cap_need > cap_max {
            awaiting.push(ev);
            continue;
        }
        if ev.cap_need > cap && cap < cap_max {
            while !awaiting.is_empty() && ev.cap_need > cap && cap_max > cap {
                let need = (ev.cap_need - cap).min(cap_max - cap);
                let good: Vec<usize> = (0..awaiting.len()).filter(|&i| -awaiting[i].cap_need >= need).collect();
                let pick = if !good.is_empty() {
                    *good.iter().min_by(|&&a, &&b| (-awaiting[a].cap_need).partial_cmp(&-awaiting[b].cap_need).unwrap()).unwrap()
                } else {
                    (0..awaiting.len()).max_by(|&a, &b| (-awaiting[a].cap_need).partial_cmp(&-awaiting[b].cap_need).unwrap()).unwrap()
                };
                let mut inj = awaiting.remove(pick);
                cap = (cap - inj.cap_need).min(cap_max);
                inj.t = t_now + inj.duration;
                inj.shot += 1;
                if inj.clip > 0 && inj.shot % inj.clip == 0 {
                    inj.shot = 0;
                    inj.t += inj.reload;
                }
                inj.seq = seq;
                seq += 1;
                heap.push(inj);
            }
        }
        cap = (cap - ev.cap_need).min(cap_max);
        if cap < cap_lowest {
            if cap < 0.0 {
                ran_out = true;
                last_ev = Some(ev);
                break;
            }
            cap_lowest = cap;
        }
        while !awaiting.is_empty() && cap < cap_max {
            let need = cap_max - cap;
            let good: Vec<usize> = (0..awaiting.len()).filter(|&i| -awaiting[i].cap_need <= need).collect();
            if good.is_empty() {
                break;
            }
            let pick = *good.iter().max_by(|&&a, &&b| (-awaiting[a].cap_need).partial_cmp(&-awaiting[b].cap_need).unwrap()).unwrap();
            let mut inj = awaiting.remove(pick);
            cap = (cap - inj.cap_need).min(cap_max);
            inj.t = t_now + inj.duration;
            inj.shot += 1;
            if inj.clip > 0 && inj.shot % inj.clip == 0 {
                inj.shot = 0;
                inj.t += inj.reload;
            }
            inj.seq = seq;
            seq += 1;
            heap.push(inj);
        }
        ev.t = t_now + ev.duration;
        ev.shot += 1;
        if ev.clip > 0 && ev.shot % ev.clip == 0 {
            ev.shot = 0;
            ev.t += ev.reload;
        }
        ev.seq = seq;
        seq += 1;
        heap.push(ev);
    }
    // EVE's own stability estimate
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
        depletes_in_s: if stable { None } else { Some(t_last / 1000.0) },
        eve_stable,
        iterations,
    }
}

/// Static per-event-stream data; heap entries only carry what changes (t, shot, seq).
#[derive(Clone, Copy)]
struct Stream {
    duration: f64,
    cap_need: f64,
    clip: u32,
    reload: f64,
    inj: bool,
}

/// Compact heap entry. `r1` ranks (duration, cap_need) and `r2` ranks (clip, reload, inj) across streams, so
/// comparing (t, r1, shot, r2, seq) gives exactly the order of `Ev::cmp` (no NaNs: checked by the caller).
/// Same order + same std BinaryHeap algorithm => identical pops and identical final heap layout.
#[derive(Clone, Copy)]
struct K {
    t: f64,
    r1: u32,
    shot: u32,
    r2: u32,
    stream: u32,
    seq: u64,
}
impl PartialEq for K {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for K {}
impl PartialOrd for K {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for K {
    #[inline(always)]
    fn cmp(&self, o: &Self) -> Ordering {
        match o.t.partial_cmp(&self.t) {
            Some(Ordering::Equal) | None => {}
            Some(x) => return x,
        }
        o.r1.cmp(&self.r1)
            .then_with(|| o.shot.cmp(&self.shot))
            .then_with(|| o.r2.cmp(&self.r2))
            .then_with(|| o.seq.cmp(&self.seq))
    }
}

#[allow(unused_assignments)] // `in_heap = false` in take!() right before a break
fn simulate_fast(capacity: f64, recharge_ms: f64, drains: &[Drain], start_frac: f64, reload: bool, stagger: bool, t_max_ms: f64) -> CapResult {
    let tau = recharge_ms / 5.0;
    let mut streams: Vec<Stream> = Vec::new();
    let mut init: Vec<(f64, u32, u64)> = Vec::new(); // (t, stream, seq)
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
        if let Some(g) = groups.iter_mut().find(|(x, _)| {
            x.duration == d.duration && x.cap_need == d.cap_need && x.clip_size == d.clip_size && x.reload_ms == d.reload_ms
                && x.is_injector == d.is_injector && x.disable_stagger == d.disable_stagger
        }) {
            g.1 += 1;
        } else {
            groups.push((d, 1));
        }
    }
    let push_stream = |streams: &mut Vec<Stream>, s: Stream| -> u32 {
        streams.push(s);
        (streams.len() - 1) as u32
    };
    for (d, n) in &groups {
        let mut d = *d;
        if d.clip_size > 0 {
            disable_period = true;
        }
        if d.is_injector {
            let st = push_stream(&mut streams, Stream { duration: d.duration, cap_need: d.cap_need, clip: d.clip_size, reload: d.reload_ms, inj: true });
            for _ in 0..*n {
                init.push((0.0, st, seq));
                seq += 1;
            }
            continue;
        }
        if stagger && !d.disable_stagger {
            if d.clip_size == 0 {
                d.duration = (d.duration / *n as f64).floor();
            } else {
                let stg = (d.duration * d.clip_size as f64 + d.reload_ms) / (*n as f64 * d.clip_size as f64);
                let st = push_stream(&mut streams, Stream { duration: d.duration, cap_need: d.cap_need, clip: d.clip_size, reload: d.reload_ms, inj: false });
                for i in 1..*n {
                    init.push((i as f64 * stg, st, seq));
                    seq += 1;
                }
            }
        } else {
            d.cap_need *= *n as f64;
        }
        let dur = d.duration.round().max(1.0) as u64;
        period = period / gcd(period, dur) * dur;
        let st = push_stream(&mut streams, Stream { duration: d.duration, cap_need: d.cap_need, clip: d.clip_size, reload: d.reload_ms, inj: false });
        init.push((0.0, st, seq));
        seq += 1;
    }
    if streams.iter().any(|s| s.duration.is_nan() || s.cap_need.is_nan()) {
        return simulate_ref(capacity, recharge_ms, drains, start_frac, reload, stagger, t_max_ms);
    }
    // ranks consistent with Ev::cmp's field order (ascending)
    let rank = |cmp: &dyn Fn(&Stream, &Stream) -> Ordering| -> Vec<u32> {
        (0..streams.len()).map(|i| (0..streams.len()).filter(|&j| cmp(&streams[j], &streams[i]) == Ordering::Less).count() as u32).collect()
    };
    let r1 = rank(&|a, b| a.duration.partial_cmp(&b.duration).unwrap().then(a.cap_need.partial_cmp(&b.cap_need).unwrap()));
    let r2 = rank(&|a, b| a.clip.cmp(&b.clip).then(a.reload.partial_cmp(&b.reload).unwrap()).then(a.inj.cmp(&b.inj)));
    if streams.len() > 63 {
        return simulate_ref(capacity, recharge_ms, drains, start_frac, reload, stagger, t_max_ms);
    }
    let mk = |t: f64, st: u32, shot: u32, seq: u64| K { t, r1: r1[st as usize], shot, r2: r2[st as usize], stream: st, seq };
    let mut heap = PackedHeap { h: BinaryHeap::with_capacity(init.len() + 1), overflow: false };
    for &(t, st, sq) in &init {
        heap.push(mk(t, st, 0, sq));
    }
    let period = if disable_period || period as f64 > t_max_ms { t_max_ms } else { period as f64 };

    let cap_max = capacity;
    let mut cap = capacity * start_frac;
    let mut cap_wrap = cap;
    let mut cap_lowest = cap;
    let mut cap_lowest_pre = cap;
    let mut t_wrap = period;
    let mut t_last = 0.0f64;
    let mut iterations = 0u64;
    let mut awaiting: Vec<K> = Vec::new();
    let mut awaiting_wrap: Vec<(u64, u64)> = Vec::new();
    let mut ran_out = false;
    let key = |v: &Vec<K>| {
        let mut k: Vec<(u64, u64)> =
            v.iter().map(|e| (streams[e.stream as usize].duration.to_bits(), streams[e.stream as usize].cap_need.to_bits())).collect();
        k.sort();
        k
    };
    let need_of = |e: &K| streams[e.stream as usize].cap_need;
    // advance an event after it fired / was injected
    let next = |e: &mut K, t_now: f64, seq: &mut u64| {
        let s = &streams[e.stream as usize];
        e.t = t_now + s.duration;
        e.shot += 1;
        if s.clip > 0 && e.shot % s.clip == 0 {
            e.shot = 0;
            e.t += s.reload;
        }
        e.seq = *seq;
        *seq += 1;
    };
    // exp() memo on the exact argument (the same few time steps recur)
    let mut exp_memo: [(u64, f64); 64] = [(f64::NAN.to_bits(), 0.0); 64];
    let mut last_ev: Option<K> = None;
    // As in eve-dogma-rs: the current event stays in the heap and is replaced in place (one sift-down) unless
    // something else must be pushed first or it leaves the simulation; pops/pushes then match A's heap exactly.
    while let Some(mut ev) = heap.peek() {
        let mut in_heap = true;
        macro_rules! take {
            () => {
                if in_heap {
                    heap.h.pop();
                    in_heap = false;
                }
            };
        }
        let t_now = ev.t;
        if t_now >= t_max_ms {
            take!();
            last_ev = Some(ev);
            break;
        }
        if t_now > t_last && cap_max > 0.0 && tau > 0.0 {
            let x = (cap / cap_max).max(0.0).sqrt();
            let arg = (t_last - t_now) / tau;
            let ab = arg.to_bits();
            let slot = (ab.wrapping_mul(0x9E3779B97F4A7C15) >> 58) as usize;
            let e = if exp_memo[slot].0 == ab {
                exp_memo[slot].1
            } else {
                let v = arg.exp();
                exp_memo[slot] = (ab, v);
                v
            };
            cap = (1.0 + (x - 1.0) * e).powi(2) * cap_max;
        }
        if t_now != t_last {
            if cap < cap_lowest_pre {
                cap_lowest_pre = cap;
            }
            if t_now == t_wrap {
                let k = key(&awaiting);
                if cap >= cap_wrap && k == awaiting_wrap {
                    take!();
                    last_ev = Some(ev);
                    break;
                }
                cap_wrap = (cap * 10.0).round() / 10.0;
                awaiting_wrap = k;
                t_wrap += period;
            }
        }
        t_last = t_now;
        iterations += 1;
        if iterations > 5_000_000 {
            take!();
            last_ev = Some(ev);
            break;
        }
        let ev_need = need_of(&ev);
        if streams[ev.stream as usize].inj && cap - ev_need > cap_max {
            take!();
            awaiting.push(ev);
            continue;
        }
        if ev_need > cap && cap < cap_max {
            while !awaiting.is_empty() && ev_need > cap && cap_max > cap {
                let need = (ev_need - cap).min(cap_max - cap);
                let good: Vec<usize> = (0..awaiting.len()).filter(|&i| -need_of(&awaiting[i]) >= need).collect();
                let pick = if !good.is_empty() {
                    *good.iter().min_by(|&&a, &&b| (-need_of(&awaiting[a])).partial_cmp(&-need_of(&awaiting[b])).unwrap()).unwrap()
                } else {
                    (0..awaiting.len()).max_by(|&a, &b| (-need_of(&awaiting[a])).partial_cmp(&-need_of(&awaiting[b])).unwrap()).unwrap()
                };
                take!();
                let mut inj = awaiting.remove(pick);
                cap = (cap - need_of(&inj)).min(cap_max);
                next(&mut inj, t_now, &mut seq);
                heap.push(inj);
            }
        }
        cap = (cap - ev_need).min(cap_max);
        if cap < cap_lowest {
            if cap < 0.0 {
                take!();
                ran_out = true;
                last_ev = Some(ev);
                break;
            }
            cap_lowest = cap;
        }
        while !awaiting.is_empty() && cap < cap_max {
            let need = cap_max - cap;
            let good: Vec<usize> = (0..awaiting.len()).filter(|&i| -need_of(&awaiting[i]) <= need).collect();
            if good.is_empty() {
                break;
            }
            let pick = *good.iter().max_by(|&&a, &&b| (-need_of(&awaiting[a])).partial_cmp(&-need_of(&awaiting[b])).unwrap()).unwrap();
            take!();
            let mut inj = awaiting.remove(pick);
            cap = (cap - need_of(&inj)).min(cap_max);
            next(&mut inj, t_now, &mut seq);
            heap.push(inj);
        }
        next(&mut ev, t_now, &mut seq);
        if in_heap {
            heap.replace_top(ev);
        } else {
            heap.push(ev);
        }
    }
    if heap.overflow {
        return simulate_ref(capacity, recharge_ms, drains, start_frac, reload, stagger, t_max_ms);
    }
    let mut all: Vec<K> = heap.h.into_vec().into_iter().map(|p| unpack(p.0)).collect();
    if let Some(e) = last_ev {
        all.push(e);
    }
    let avg_drain: f64 = all.iter().map(|e| streams[e.stream as usize].cap_need / streams[e.stream as usize].duration).sum();
    let inner = -(2.0 * avg_drain * tau - cap_max) / cap_max;
    let eve_stable = if inner >= 0.0 && cap_max > 0.0 { 0.25 * (1.0 + inner.sqrt()).powi(2) } else { 0.0 };
    let stable = !ran_out;
    CapResult {
        stable,
        stable_low: if stable && cap_max > 0.0 { cap_lowest / cap_max } else { 0.0 },
        stable_high: if stable && cap_max > 0.0 { cap_lowest_pre / cap_max } else { 0.0 },
        t_s: t_last / 1000.0,
        depletes_in_s: if stable { None } else { Some(t_last / 1000.0) },
        eve_stable,
        iterations,
    }
}

/// K packed into one u128 whose integer order is K's order reversed: t (non-negative f64 bits) | r1:6 | shot:22 |
/// r2:6 | seq:24 | stream:6 (stream never decides: seq is unique). A std BinaryHeap of `Reverse<u128>` therefore
/// makes exactly the comparisons, pops and final layout of a BinaryHeap<K>, at a fraction of the cost.
struct PackedHeap {
    h: BinaryHeap<std::cmp::Reverse<u128>>,
    overflow: bool,
}
impl PackedHeap {
    #[inline]
    fn pack(&mut self, k: K) -> u128 {
        let tb = k.t.to_bits();
        if tb >> 63 != 0 || k.t.is_nan() || k.shot >= 1 << 22 || k.seq >= 1 << 24 || k.r1 >= 64 || k.r2 >= 64 || k.stream >= 64 {
            self.overflow = true;
        }
        let lo = (k.r1 as u64 & 63) << 58 | (k.shot as u64 & ((1 << 22) - 1)) << 36 | (k.r2 as u64 & 63) << 30 | (k.seq & ((1 << 24) - 1)) << 6 | (k.stream as u64 & 63);
        (tb as u128) << 64 | lo as u128
    }
    #[inline]
    fn push(&mut self, k: K) {
        let p = self.pack(k);
        self.h.push(std::cmp::Reverse(p));
    }
    /// top event without removing it (None after an overflow: the caller falls back to the reference simulation)
    #[inline]
    fn peek(&self) -> Option<K> {
        if self.overflow {
            return None;
        }
        self.h.peek().map(|p| unpack(p.0))
    }
    /// replace the top entry in place (BinaryHeap::peek_mut: one sift-down)
    #[inline]
    fn replace_top(&mut self, k: K) {
        let p = self.pack(k);
        *self.h.peek_mut().expect("non-empty") = std::cmp::Reverse(p);
    }
}

#[inline]
fn unpack(p: u128) -> K {
    let lo = p as u64;
    K {
        t: f64::from_bits((p >> 64) as u64),
        r1: (lo >> 58) as u32,
        shot: ((lo >> 36) & ((1 << 22) - 1)) as u32,
        r2: ((lo >> 30) & 63) as u32,
        seq: (lo >> 6) & ((1 << 24) - 1),
        stream: (lo & 63) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(a: &CapResult, b: &CapResult) -> bool {
        a.stable == b.stable
            && a.stable_low.to_bits() == b.stable_low.to_bits()
            && a.stable_high.to_bits() == b.stable_high.to_bits()
            && a.t_s.to_bits() == b.t_s.to_bits()
            && a.depletes_in_s.map(f64::to_bits) == b.depletes_in_s.map(f64::to_bits)
            // the fast path updates the top event in place like eve-dogma-rs (one sift-down), the reference pops and
            // pushes: same pop order, different final heap layout, so EVE's sum over the heap may differ in the last bit
            && ((a.eve_stable - b.eve_stable).abs() <= 1e-12 * a.eve_stable.abs().max(1.0))
            && a.iterations == b.iterations
    }

    #[test]
    fn fast_matches_reference_randomised() {
        let mut x: u64 = 0x9E3779B97F4A7C15;
        let mut rnd = |n: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % n
        };
        for case in 0..3000 {
            let nd = 1 + rnd(9) as usize;
            let mut drains = Vec::new();
            for _ in 0..nd {
                let dup = !drains.is_empty() && rnd(3) == 0;
                if dup {
                    let d: Drain = drains[rnd(drains.len() as u64) as usize];
                    drains.push(d);
                    continue;
                }
                let inj = rnd(6) == 0;
                let durs = [1000.0, 2000.0, 2500.0, 3000.0, 4500.0, 5000.0, 6000.0, 10000.0, 12000.0, 3333.0];
                let duration = durs[rnd(durs.len() as u64) as usize] + if rnd(4) == 0 { rnd(997) as f64 } else { 0.0 };
                let cap_need = if inj { -((50 + rnd(800)) as f64) } else { (rnd(400) as f64) * 0.5 + if rnd(5) == 0 { -10.0 } else { 0.0 } };
                let clip = if inj || rnd(4) == 0 { 1 + rnd(9) as u32 } else { 0 };
                drains.push(Drain { duration, cap_need, clip_size: clip, reload_ms: (rnd(3) * 5000) as f64, is_injector: inj, disable_stagger: rnd(5) == 0 });
            }
            let cap = (200 + rnd(6000)) as f64;
            let rr = (60_000 + rnd(900_000)) as f64;
            let reload = rnd(2) == 0;
            let stagger = rnd(3) != 0;
            let tmax = [6.0 * 3600.0 * 1000.0, 600_000.0][rnd(2) as usize];
            let a = simulate_ref(cap, rr, &drains, 1.0, reload, stagger, tmax);
            let b = simulate_fast(cap, rr, &drains, 1.0, reload, stagger, tmax);
            assert!(same(&a, &b), "case {case}: {drains:?} cap {cap} rr {rr} reload {reload} stagger {stagger}\n{a:?}\n{b:?}");
        }
    }
}
