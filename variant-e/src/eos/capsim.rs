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

fn lt(a: &Act, b: &Act) -> bool {
    for i in 0..7 {
        if a.0[i] < b.0[i] {
            return true;
        }
        if a.0[i] > b.0[i] {
            return false;
        }
    }
    false
}

// CPython heapq (_siftdown/_siftup)
fn siftdown(h: &mut [Act], start: usize, mut pos: usize) {
    let new = h[pos];
    while pos > start {
        let parent = (pos - 1) >> 1;
        if lt(&new, &h[parent]) {
            h[pos] = h[parent];
            pos = parent;
            continue;
        }
        break;
    }
    h[pos] = new;
}
fn siftup(h: &mut [Act], mut pos: usize) {
    let end = h.len();
    let start = pos;
    let new = h[pos];
    let mut child = 2 * pos + 1;
    while child < end {
        let right = child + 1;
        if right < end && !lt(&h[child], &h[right]) {
            child = right;
        }
        h[pos] = h[child];
        pos = child;
        child = 2 * pos + 1;
    }
    h[pos] = new;
    siftdown(h, start, pos);
}
fn push(h: &mut Vec<Act>, a: Act) {
    h.push(a);
    let n = h.len() - 1;
    siftdown(h, 0, n);
}
fn pop(h: &mut Vec<Act>) -> Option<Act> {
    let last = h.pop()?;
    if !h.is_empty() {
        let ret = h[0];
        h[0] = last;
        siftup(h, 0);
        Some(ret)
    } else {
        Some(last)
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
    // reset()
    let mut state: Vec<Act> = Vec::new();
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
                push(&mut state, Act([0.0, duration, cap_need, 0.0, clip, rt, inj]));
            }
            continue;
        }
        if stagger && ds == 0.0 {
            if clip == 0.0 {
                duration = (duration / amount as f64).trunc();
            } else {
                let sa = (duration * clip + rt) / (amount as f64 * clip);
                for i in 1..amount {
                    push(&mut state, Act([i as f64 * sa, duration, cap_need, 0.0, clip, rt, inj]));
                }
            }
        } else {
            cap_need *= amount as f64;
        }
        period = lcm(period, duration);
        push(&mut state, Act([0.0, duration, cap_need, 0.0, clip, rt, inj]));
    }
    let period = if disable_period { t_max } else { period };

    // run()
    let mut awaiting: Vec<[f64; 6]> = Vec::new();
    let mut awaiting_wrap: Vec<[f64; 6]> = Vec::new();
    let mut activation: Option<Act> = None;
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
    let inject = |cap: &mut f64, state: &mut Vec<Act>, t_now: f64, inj: [f64; 6]| {
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
        push(state, Act([t, d, need, shot, clip, rt, isinj]));
    };
    loop {
        let Some(mut act) = pop(&mut state) else { break };
        activation = Some(act);
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
                if cap >= cap_wrap && counter_eq(&awaiting, &awaiting_wrap) {
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
                }
            }
            cap -= cap_need;
            if cap > cap_cap {
                cap = cap_cap;
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
            }
            t_now += duration;
            shot += 1.0;
            if clip != 0.0 && mod_zero(shot, clip) {
                shot = 0.0;
                t_now += rt;
            }
            act.0[0] = t_now;
            act.0[3] = shot;
            activation = Some(act);
            push(&mut state, act);
        }
    }
    if let Some(a) = activation {
        push(&mut state, a);
    }
    let avg: f64 = state.iter().map(|x| x.0[2] / x.0[1]).sum();
    let inner = -(2.0 * avg * tau - cap_cap) / cap_cap;
    let eve = if inner < 0.0 { 0.0 } else { 0.25 * (1.0 + inner.sqrt()).powi(2) };
    let (lo, hi) = if cap > 0.0 { (cap_lowest, cap_lowest_pre) } else { (0.0, 0.0) };
    SimResult { t: t_last, iterations, cap_stable_low: lo, cap_stable_high: hi, cap_stable_eve: eve }
}
