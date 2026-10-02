"""Event-driven capacitor simulation (same algorithm as Pyfa eos capSim / the reference engine)."""
import heapq
import math


def simulate(capacity, recharge_ms, drains, start_frac, reload, stagger, t_max_ms):
    """drains: list of dicts duration, cap_need, clip_size, reload_ms, is_injector, disable_stagger"""
    tau = recharge_ms / 5.0
    heap = []
    seq = 0
    period = 1
    disable_period = False
    groups = []  # [drain tuple, count]
    for d in drains:
        dur, need, clip, rl, inj, nost = d
        if not reload and not inj:
            clip, rl = 0, 0.0
        if dur <= 0.0:
            continue
        key = (dur, need, clip, rl, inj, nost)
        for g in groups:
            if g[0] == key:
                g[1] += 1
                break
        else:
            groups.append([key, 1])
    # heap entries mirror Pyfa's list ordering: [t, duration, capNeed, shot, clip, reload, isInjector, seq]
    for (dur, need, clip, rl, inj, nost), n in groups:
        if clip > 0:
            disable_period = True
        if inj:
            for _ in range(n):
                heapq.heappush(heap, (0.0, dur, need, 0, clip, rl, inj, seq))
                seq += 1
            continue
        if stagger and not nost:
            if clip == 0:
                dur = math.floor(dur / n)
            else:
                st = (dur * clip + rl) / (n * clip)
                for i in range(1, n):
                    heapq.heappush(heap, (i * st, dur, need, 0, clip, rl, False, seq))
                    seq += 1
        else:
            need *= n
        di = max(int(_round(dur)), 1)
        period = period // math.gcd(period, di) * di
        heapq.heappush(heap, (0.0, dur, need, 0, clip, rl, False, seq))
        seq += 1
    period = t_max_ms if (disable_period or period > t_max_ms) else float(period)

    cap_max = capacity
    cap = capacity * start_frac
    cap_wrap = cap
    cap_lowest = cap
    cap_lowest_pre = cap
    t_wrap = period
    t_last = 0.0
    iterations = 0
    awaiting = []
    awaiting_wrap = []
    ran_out = False
    last_ev = None
    exp = math.exp
    sqrt = math.sqrt
    while heap:
        ev = heapq.heappop(heap)
        t_now = ev[0]
        if t_now >= t_max_ms:
            last_ev = ev
            break
        if t_now > t_last and cap_max > 0.0 and tau > 0.0:
            x = sqrt(max(cap / cap_max, 0.0))
            cap = (1.0 + (x - 1.0) * exp((t_last - t_now) / tau)) ** 2 * cap_max
        if t_now != t_last:
            if cap < cap_lowest_pre:
                cap_lowest_pre = cap
            if t_now == t_wrap:
                k = sorted((e[1], e[2]) for e in awaiting)
                if cap >= cap_wrap and k == awaiting_wrap:
                    last_ev = ev
                    break
                cap_wrap = _round(cap * 10.0) / 10.0
                awaiting_wrap = k
                t_wrap += period
        t_last = t_now
        iterations += 1
        if iterations > 5_000_000:
            last_ev = ev
            break
        _, dur, need, shot, clip, rl, inj, _s = ev
        if inj and cap - need > cap_max:
            awaiting.append(ev)
            continue
        if need > cap and cap < cap_max:
            while awaiting and need > cap and cap_max > cap:
                want = min(need - cap, cap_max - cap)
                good = [i for i, a in enumerate(awaiting) if -a[2] >= want]
                if good:
                    pick = min(good, key=lambda i: -awaiting[i][2])
                else:
                    pick = _last_max(range(len(awaiting)), lambda i: -awaiting[i][2])
                seq, inj_need = _fire_injector(heap, awaiting.pop(pick), t_now, seq)
                cap = min(cap - inj_need, cap_max)
        cap = min(cap - need, cap_max)
        if cap < cap_lowest:
            if cap < 0.0:
                ran_out = True
                last_ev = ev
                break
            cap_lowest = cap
        while awaiting and cap < cap_max:
            want = cap_max - cap
            good = [i for i, a in enumerate(awaiting) if -a[2] <= want]
            if not good:
                break
            pick = _last_max(good, lambda i: -awaiting[i][2])
            seq, inj_need = _fire_injector(heap, awaiting.pop(pick), t_now, seq)
            cap = min(cap - inj_need, cap_max)
        t = t_now + dur
        shot += 1
        if clip > 0 and shot % clip == 0:
            shot = 0
            t += rl
        heapq.heappush(heap, (t, dur, need, shot, clip, rl, inj, seq))
        seq += 1
    allev = list(heap)
    if last_ev is not None:
        allev.append(last_ev)
    avg_drain = sum(e[2] / e[1] for e in allev)
    eve_stable = 0.0
    if cap_max > 0.0:
        inner = -(2.0 * avg_drain * tau - cap_max) / cap_max
        if inner >= 0.0:
            eve_stable = 0.25 * (1.0 + sqrt(inner)) ** 2
    stable = not ran_out
    return {"stable": stable,
            "stable_low": cap_lowest / cap_max if stable and cap_max > 0.0 else 0.0,
            "stable_high": cap_lowest_pre / cap_max if stable and cap_max > 0.0 else 0.0,
            "t_s": t_last / 1000.0, "eve_stable": eve_stable, "iterations": iterations}


def _last_max(idx, key):
    """like Rust Iterator::max_by: the last of equal maxima"""
    best, bk = None, None
    for i in idx:
        k = key(i)
        if best is None or k >= bk:
            best, bk = i, k
    return best


def _fire_injector(heap, inj, t_now, seq):
    _, dur, need, shot, clip, rl, isinj, _s = inj
    t = t_now + dur
    shot += 1
    if clip > 0 and shot % clip == 0:
        shot = 0
        t += rl
    heapq.heappush(heap, (t, dur, need, shot, clip, rl, isinj, seq))
    return seq + 1, need


def _round(x):
    """Rust f64::round (half away from zero)"""
    return math.floor(x + 0.5) if x >= 0 else -math.floor(-x + 0.5)
