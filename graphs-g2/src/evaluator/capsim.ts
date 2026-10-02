// Capacitor simulator with history (behaviour of eos capSim, LGPL; re-implemented here and in the Go engine).
// Events: each drain fires every `duration` ms, optionally with a clip and reload; injectors wait in a queue
// until their charge would not overflow; staggering spreads identical modules over the cycle.
import type { CapDrain } from "./types.js";

interface Ev {
  t: number;
  duration: number;
  capNeed: number;
  reload: number;
  shot: number;
  clip: number;
  seq: number;
  inj: boolean;
}

function less(a: Ev, b: Ev): boolean {
  if (a.t !== b.t) return a.t < b.t;
  if (a.duration !== b.duration) return a.duration < b.duration;
  if (a.capNeed !== b.capNeed) return a.capNeed < b.capNeed;
  if (a.shot !== b.shot) return a.shot < b.shot;
  if (a.clip !== b.clip) return a.clip < b.clip;
  if (a.reload !== b.reload) return a.reload < b.reload;
  if (a.inj !== b.inj) return !a.inj;
  return a.seq < b.seq;
}

class Heap {
  a: Ev[] = [];
  get size() {
    return this.a.length;
  }
  push(e: Ev) {
    const a = this.a;
    a.push(e);
    let i = a.length - 1;
    while (i > 0) {
      const p = (i - 1) >> 1;
      if (!less(a[i], a[p])) break;
      [a[i], a[p]] = [a[p], a[i]];
      i = p;
    }
  }
  pop(): Ev {
    const a = this.a;
    const top = a[0];
    const last = a.pop()!;
    if (a.length) {
      a[0] = last;
      let i = 0;
      for (;;) {
        const l = 2 * i + 1;
        const r = l + 1;
        let m = i;
        if (l < a.length && less(a[l], a[m])) m = l;
        if (r < a.length && less(a[r], a[m])) m = r;
        if (m === i) break;
        [a[i], a[m]] = [a[m], a[i]];
        i = m;
      }
    }
    return top;
  }
}

export interface CapHistory {
  points: [number, number][]; // (t seconds, cap GJ) after the events at t
  ranOut: boolean;
  endS: number;
}

export function simulateCapHistory(capacity: number, rechargeMs: number, drains: CapDrain[], startFrac: number, reload: boolean, stagger: boolean, tMaxMs: number): CapHistory {
  const tau = rechargeMs / 5;
  const h = new Heap();
  let seq = 0;
  type G = { d: CapDrain; n: number };
  const groups: G[] = [];
  const same = (a: CapDrain, b: CapDrain) =>
    a.duration_ms === b.duration_ms && a.cap_need === b.cap_need && a.clip_size === b.clip_size && a.reload_ms === b.reload_ms && a.is_injector === b.is_injector && a.disable_stagger === b.disable_stagger;
  for (const d0 of drains) {
    const d = { ...d0 };
    if (!reload && !d.is_injector) {
      d.clip_size = 0;
      d.reload_ms = 0;
    }
    if (d.duration_ms <= 0) continue;
    const g = groups.find((x) => same(x.d, d));
    if (g) g.n++;
    else groups.push({ d, n: 1 });
  }
  for (const { d: d0, n } of groups) {
    const d = { ...d0 };
    if (d.is_injector) {
      for (let k = 0; k < n; k++) h.push({ t: 0, duration: d.duration_ms, capNeed: d.cap_need, reload: d.reload_ms, shot: 0, clip: d.clip_size, seq: seq++, inj: true });
      continue;
    }
    if (stagger && !d.disable_stagger) {
      if (d.clip_size === 0) d.duration_ms = Math.floor(d.duration_ms / n);
      else {
        const st = (d.duration_ms * d.clip_size + d.reload_ms) / (n * d.clip_size);
        for (let k = 1; k < n; k++) h.push({ t: k * st, duration: d.duration_ms, capNeed: d.cap_need, reload: d.reload_ms, shot: 0, clip: d.clip_size, seq: seq++, inj: false });
      }
    } else d.cap_need *= n;
    h.push({ t: 0, duration: d.duration_ms, capNeed: d.cap_need, reload: d.reload_ms, shot: 0, clip: d.clip_size, seq: seq++, inj: false });
  }
  const capMax = capacity;
  let cap = capacity * startFrac;
  let capLowest = cap;
  let tLast = 0;
  const hist = new Map<number, number>();
  hist.set(0, cap);
  const awaiting: Ev[] = [];
  let ranOut = false;
  let iterations = 0;
  const next = (e: Ev, tNow: number) => {
    e.t = tNow + e.duration;
    e.shot++;
    if (e.clip > 0 && e.shot % e.clip === 0) {
      e.shot = 0;
      e.t += e.reload;
    }
    e.seq = seq++;
  };
  while (h.size > 0) {
    const ev = h.pop();
    const tNow = ev.t;
    if (tNow >= tMaxMs) break;
    if (tNow > tLast && capMax > 0 && tau > 0) {
      const x = Math.sqrt(Math.max(cap / capMax, 0));
      const y = 1 + (x - 1) * Math.exp((tLast - tNow) / tau);
      cap = y * y * capMax;
    }
    tLast = tNow;
    if (++iterations > 5_000_000) break;
    if (ev.inj && cap - ev.capNeed > capMax) {
      awaiting.push(ev);
      hist.set(tNow, cap);
      continue;
    }
    if (ev.capNeed > cap && cap < capMax) {
      while (awaiting.length && ev.capNeed > cap && capMax > cap) {
        const need = Math.min(ev.capNeed - cap, capMax - cap);
        let pick = -1;
        awaiting.forEach((a, i) => {
          if (-a.capNeed >= need && (pick < 0 || -a.capNeed < -awaiting[pick].capNeed)) pick = i;
        });
        if (pick < 0) awaiting.forEach((a, i) => {
          if (pick < 0 || -a.capNeed >= -awaiting[pick].capNeed) pick = i;
        });
        const inj = awaiting.splice(pick, 1)[0];
        cap = Math.min(cap - inj.capNeed, capMax);
        next(inj, tNow);
        h.push(inj);
      }
    }
    cap = Math.min(cap - ev.capNeed, capMax);
    if (cap < capLowest) {
      if (cap < 0) {
        ranOut = true;
        break;
      }
      capLowest = cap;
    }
    while (awaiting.length && cap < capMax) {
      const need = capMax - cap;
      let pick = -1;
      awaiting.forEach((a, i) => {
        if (-a.capNeed <= need && (pick < 0 || -a.capNeed >= -awaiting[pick].capNeed)) pick = i;
      });
      if (pick < 0) break;
      const inj = awaiting.splice(pick, 1)[0];
      cap = Math.min(cap - inj.capNeed, capMax);
      next(inj, tNow);
      h.push(inj);
    }
    hist.set(tNow, cap);
    next(ev, tNow);
    h.push(ev);
  }
  const points = [...hist.entries()].sort((a, b) => a[0] - b[0]).map(([t, c]) => [t / 1000, c] as [number, number]);
  return { points, ranOut, endS: tLast / 1000 };
}
