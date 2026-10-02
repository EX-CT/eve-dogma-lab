//! Port of Pyfa eos/capSim.py (CapSimulator), including Python heapq ordering. GPL-3.0-or-later.

#[derive(Clone, Copy, Debug)]
pub struct Drain {
    pub duration: f64,
    pub cap_need: f64,
    pub clip_size: f64,
    pub disable_stagger: bool,
    pub reload_time: f64,
    pub is_injector: bool,
}

/// heap entry [t_now, duration, capNeed, shot, clipSize, reloadTime, isInjector]
#[derive(Clone, Copy, Debug)]
struct Act([f64; 7]);

#[inline(always)]
fn lt(a: &Act, b: &Act) -> bool {
    // first field (event time) decides almost every comparison
    if a.0[0] < b.0[0] {
        return true;
    }
    if a.0[0] > b.0[0] {
        return false;
    }
    for i in 1..7 {
        if a.0[i] < b.0[i] {
            return true;
        }
        if a.0[i] > b.0[i] {
            return false;
        }
    }
    false
}

// CPython heapq (_siftdown/_siftup) over a heap of indices into an entry arena: identical comparisons and
// layout to heapq on the tuples themselves, but sifting moves 4-byte indices instead of 56-byte entries.
struct Heap {
    a: Vec<Act>,
    h: Vec<u32>,
}

impl Heap {
    #[inline]
    fn lt(&self, x: u32, y: u32) -> bool {
        lt(&self.a[x as usize], &self.a[y as usize])
    }
    #[inline(always)]
    fn siftdown(&mut self, start: usize, mut pos: usize) {
        let new = self.h[pos];
        while pos > start {
            let parent = (pos - 1) >> 1;
            if self.lt(new, self.h[parent]) {
                self.h[pos] = self.h[parent];
                pos = parent;
                continue;
            }
            break;
        }
        self.h[pos] = new;
    }
    fn siftup(&mut self, mut pos: usize) {
        let end = self.h.len();
        let start = pos;
        let new = self.h[pos];
        let mut child = 2 * pos + 1;
        while child < end {
            let right = child + 1;
            if right < end && !self.lt(self.h[child], self.h[right]) {
                child = right;
            }
            self.h[pos] = self.h[child];
            pos = child;
            child = 2 * pos + 1;
        }
        self.h[pos] = new;
        self.siftdown(start, pos);
    }
    /// heappush of a new entry
    fn push(&mut self, act: Act) {
        self.a.push(act);
        let i = (self.a.len() - 1) as u32;
        self.push_idx(i);
    }
    /// heappush of an entry already in the arena (popped earlier)
    #[inline(always)]
    fn push_idx(&mut self, i: u32) {
        self.h.push(i);
        let n = self.h.len() - 1;
        self.siftdown(0, n);
    }
    /// heappop -> arena index
    fn pop(&mut self) -> Option<u32> {
        let last = self.h.pop()?;
        if !self.h.is_empty() {
            let ret = self.h[0];
            self.h[0] = last;
            self.siftup(0);
            Some(ret)
        } else {
            Some(last)
        }
    }
}

/// `a % b == 0.0` (C fmod), using integer arithmetic when both are small integers (fmod is exact, so identical)
#[inline]
fn mod_zero(a: f64, b: f64) -> bool {
    if a >= 0.0 && b >= 1.0 && a < 2147483648.0 && b < 2147483648.0 && a.fract() == 0.0 && b.fract() == 0.0 {
        (a as u64) % (b as u64) == 0
    } else {
        a % b == 0.0
    }
}

fn lcm(a: f64, b: f64) -> f64 {
    let n = a * b;
    let (mut a, mut b) = (a, b);
    while b != 0.0 {
        let t = a % b;
        a = b;
        b = t;
    }
    n / a
}

fn py_round1(v: f64) -> f64 {
    super::stats::py_round_digits(v, 1)
}

pub struct SimResult {
    pub t: f64,
    pub iterations: u64,
    pub cap_stable_low: f64,
    pub cap_stable_high: f64,
    pub cap_stable_eve: f64,
}

pub fn run(modules: &[Drain], capacity: f64, recharge: f64, starting: f64, t_max: f64, reload: bool, stagger: bool) -> SimResult {
    run_ex(modules, capacity, recharge, starting, t_max, reload, stagger, true, None)
}

/// Full simulator. `optimize_repeats` = Pyfa `optimize_repeats`; `saved` (when given) receives Pyfa's
/// `saved_changes`: (t seconds, max(0, cap)) at every time the capacitor changed by an activation, sorted by t.
#[allow(clippy::too_many_arguments)]
pub fn run_ex(modules: &[Drain], capacity: f64, recharge: f64, starting: f64, t_max: f64, reload: bool, stagger: bool,
              optimize_repeats: bool, mut saved: Option<&mut Vec<(f64, f64)>>) -> SimResult {
    // reset()
    let mut state = Heap { a: Vec::with_capacity(16), h: Vec::with_capacity(16) };
    let mut mods: Vec<([f64; 6], u32)> = Vec::new();
    let mut period = 1.0f64;
    let mut disable_period = false;
    for m in modules {
        let (mut clip, mut rt) = (m.clip_size, m.reload_time);
        if !reload && !m.is_injector {
            clip = 0.0;
            rt = 0.0;
        }
        let key = [m.duration, m.cap_need, clip, m.disable_stagger as u8 as f64, rt, m.is_injector as u8 as f64];
        match mods.iter_mut().find(|x| x.0 == key) {
            Some(x) => x.1 += 1,
            None => mods.push((key, 1)),
        }
    }
    for (k, amount) in &mods {
        let [mut duration, mut cap_need, clip, ds, rt, inj] = *k;
        let amount = *amount;
        if clip != 0.0 {
            disable_period = true;
        }
        if inj != 0.0 {
            for _ in 0..amount {
                state.push(Act([0.0, duration, cap_need, 0.0, clip, rt, inj]));
            }
            continue;
        }
        if stagger && ds == 0.0 {
            if clip == 0.0 {
                duration = (duration / amount as f64).trunc();
            } else {
                let sa = (duration * clip + rt) / (amount as f64 * clip);
                for i in 1..amount {
                    state.push(Act([i as f64 * sa, duration, cap_need, 0.0, clip, rt, inj]));
                }
            }
        } else {
            cap_need *= amount as f64;
        }
        period = lcm(period, duration);
        state.push(Act([0.0, duration, cap_need, 0.0, clip, rt, inj]));
    }
    let period = if disable_period { t_max } else { period };

    // run()
    let mut awaiting: Vec<[f64; 6]> = Vec::new();
    let mut awaiting_wrap: Vec<[f64; 6]> = Vec::new();
    let mut activation: Option<u32> = None;
    let mut iterations = 0u64;
    let cap_cap = capacity;
    let tau = recharge / 5.0;
    let mut cap_wrap = starting;
    let mut cap_lowest = starting;
    let mut cap_lowest_pre = starting;
    let mut cap = starting;
    let mut t_wrap = period;
    let mut t_last = 0.0f64;
    let counter_eq = |a: &Vec<[f64; 6]>, b: &Vec<[f64; 6]>| {
        if a.len() != b.len() {
            return false;
        }
        let mut x = a.clone();
        let mut y = b.clone();
        let cmp = |p: &[f64; 6], q: &[f64; 6]| p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal);
        x.sort_by(cmp);
        y.sort_by(cmp);
        x == y
    };
    let inject = |cap: &mut f64, state: &mut Heap, t_now: f64, inj: [f64; 6]| {
        let [d, need, mut shot, clip, rt, isinj] = inj;
        *cap -= need;
        if *cap > cap_cap {
            *cap = cap_cap;
        }
        let mut t = t_now + d;
        shot += 1.0;
        if clip != 0.0 && mod_zero(shot, clip) {
            shot = 0.0;
            t += rt;
        }
        state.push(Act([t, d, need, shot, clip, rt, isinj]));
    };
    loop {
        let Some(ai) = state.pop() else { break };
        let mut act = state.a[ai as usize];
        activation = Some(ai);
        let [t_now0, duration, cap_need, mut shot, clip, rt, isinj] = act.0;
        let mut t_now = t_now0;
        if t_now >= t_max {
            break;
        }
        if t_now > t_last {
            cap = (1.0 + ((cap / cap_cap).sqrt() - 1.0) * ((t_last - t_now) / tau).exp()).powi(2) * cap_cap;
        }
        if t_now != t_last {
            if cap < cap_lowest_pre {
                cap_lowest_pre = cap;
            }
            if t_now == t_wrap {
                if optimize_repeats && cap >= cap_wrap && counter_eq(&awaiting, &awaiting_wrap) {
                    break;
                }
                cap_wrap = py_round1(cap);
                awaiting_wrap = awaiting.clone();
                t_wrap += period;
            }
        }
        t_last = t_now;
        iterations += 1;
        if isinj != 0.0 && cap - cap_need > cap_cap {
            awaiting.push([duration, cap_need, shot, clip, rt, isinj]);
        } else {
            if cap_need > cap && cap < cap_cap {
                while !awaiting.is_empty() && cap_need > cap && cap_cap > cap {
                    let needed = (cap_need - cap).min(cap_cap - cap);
                    let good: Vec<usize> = (0..awaiting.len()).filter(|&i| -awaiting[i][1] >= needed).collect();
                    let best = if !good.is_empty() {
                        // min by -capNeed (first minimal)
                        let mut b = good[0];
                        for &i in &good {
                            if -awaiting[i][1] < -awaiting[b][1] {
                                b = i;
                            }
                        }
                        b
                    } else {
                        // Pyfa would raise on max([]); take the largest injector instead
                        let mut b = 0;
                        for i in 0..awaiting.len() {
                            if -awaiting[i][1] > -awaiting[b][1] {
                                b = i;
                            }
                        }
                        b
                    };
                    let inj = awaiting.remove(best);
                    inject(&mut cap, &mut state, t_now, inj);
                    if let Some(sv) = saved.as_deref_mut() {
                        record(sv, t_now, cap);
                    }
                }
            }
            cap -= cap_need;
            if cap > cap_cap {
                cap = cap_cap;
            }
            if let Some(sv) = saved.as_deref_mut() {
                record(sv, t_now, cap);
            }
            if cap < cap_lowest {
                if cap < 0.0 {
                    break;
                }
                cap_lowest = cap;
            }
            while !awaiting.is_empty() && cap < cap_cap {
                let needed = cap_cap - cap;
                let good: Vec<usize> = (0..awaiting.len()).filter(|&i| -awaiting[i][1] <= needed).collect();
                if good.is_empty() {
                    break;
                }
                let mut b = good[0];
                for &i in &good {
                    if -awaiting[i][1] > -awaiting[b][1] {
                        b = i;
                    }
                }
                let inj = awaiting.remove(b);
                inject(&mut cap, &mut state, t_now, inj);
                if let Some(sv) = saved.as_deref_mut() {
                    record(sv, t_now, cap);
                }
            }
            t_now += duration;
            shot += 1.0;
            if clip != 0.0 && mod_zero(shot, clip) {
                shot = 0.0;
                t_now += rt;
            }
            act.0[0] = t_now;
            act.0[3] = shot;
            state.a[ai as usize] = act;
            state.push_idx(ai);
        }
    }
    if let Some(a) = activation {
        // Pyfa re-pushes the last popped activation (the list object, possibly already re-pushed and mutated)
        state.push_idx(a);
    }
    let avg: f64 = state.h.iter().map(|&i| state.a[i as usize].0[2] / state.a[i as usize].0[1]).sum();
    let inner = -(2.0 * avg * tau - cap_cap) / cap_cap;
    let eve = if inner < 0.0 { 0.0 } else { 0.25 * (1.0 + inner.sqrt()).powi(2) };
    let (lo, hi) = if cap > 0.0 { (cap_lowest, cap_lowest_pre) } else { (0.0, 0.0) };
    if let Some(sv) = saved {
        // dict keyed by time (ms), last write wins; sorted; values clamped at 0, times in s
        sv.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        for x in sv.iter_mut() {
            *x = (x.0 / 1000.0, x.1.max(0.0));
        }
    }
    SimResult { t: t_last, iterations, cap_stable_low: lo, cap_stable_high: hi, cap_stable_eve: eve }
}

fn record(sv: &mut Vec<(f64, f64)>, t: f64, cap: f64) {
    // activations pop in time order, so an equal key can only be the last one
    if let Some(l) = sv.last_mut().filter(|l| l.0 == t) {
        l.1 = cap;
    } else {
        sv.push((t, cap));
    }
}
