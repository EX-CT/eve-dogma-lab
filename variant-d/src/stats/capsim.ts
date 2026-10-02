/** Event-driven capacitor simulator (behaviour-compatible with Pyfa eos/capSim.py, LGPL). */

export interface Drain {
  /** cycle duration in ms */ duration: number;
  /** cap used per cycle (negative = injected) */ capNeed: number;
  /** shots before reload (0 = infinite) */ clipSize: number;
  reloadMs: number; isInjector: boolean; disableStagger: boolean;
}

export interface CapResult {
  stable: boolean; stableLow: number; stableHigh: number; tS: number; depletesInS: number | null; eveStable: number; iterations: number;
}

interface Ev { t: number; duration: number; capNeed: number; shot: number; clip: number; reload: number; inj: boolean; seq: number }

/** Python-list ordering [t, duration, capNeed, shot, clip, reload, inj] then insertion order (min first). */
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
  get size() { return this.a.length; }
  push(e: Ev): void {
    const a = this.a;
    let i = a.length;
    a.push(e);
    // sift up with a hole (no swaps)
    while (i > 0) {
      const p = (i - 1) >> 1;
      const pe = a[p];
      if (!less(e, pe)) break;
      a[i] = pe;
      i = p;
    }
    a[i] = e;
  }
  /** replace the minimum by e (= pop + push of e, one sift) */
  replaceTop(e: Ev): void {
    const a = this.a;
    const n = a.length;
    let i = 0;
    for (;;) {
      const l = 2 * i + 1;
      if (l >= n) break;
      const r = l + 1;
      const m = r < n && less(a[r], a[l]) ? r : l;
      if (!less(a[m], e)) break;
      a[i] = a[m];
      i = m;
    }
    a[i] = e;
  }
  pop(): Ev | undefined {
    const a = this.a;
    const n0 = a.length;
    if (n0 === 0) return undefined;
    const top = a[0];
    const last = a.pop()!;
    const n = n0 - 1;
    if (n > 0) {
      let i = 0;
      for (;;) {
        const l = 2 * i + 1;
        if (l >= n) break;
        const r = l + 1;
        const m = r < n && less(a[r], a[l]) ? r : l;
        if (!less(a[m], last)) break;
        a[i] = a[m];
        i = m;
      }
      a[i] = last;
    }
    return top;
  }
}

const gcd = (a: number, b: number): number => (b === 0 ? a : gcd(b, a % b));
const rnd = (x: number) => (x < 0 ? -Math.round(-x) : Math.round(x));

export function simulate(capacity: number, rechargeMs: number, drains: Drain[], startFrac: number, reload: boolean, stagger: boolean, tMaxMs: number): CapResult {
  const tau = rechargeMs / 5;
  const heap = new Heap();
  let seq = 0;
  let period = 1;
  let disablePeriod = false;
  const groups: [Drain, number][] = [];
  for (const d0 of drains) {
    const d = { ...d0 };
    if (!reload && !d.isInjector) { d.clipSize = 0; d.reloadMs = 0; }
    if (d.duration <= 0) continue;
    const g = groups.find(([x]) => x.duration === d.duration && x.capNeed === d.capNeed && x.clipSize === d.clipSize && x.reloadMs === d.reloadMs && x.isInjector === d.isInjector && x.disableStagger === d.disableStagger);
    if (g) g[1]++;
    else groups.push([d, 1]);
  }
  for (const [d0, n] of groups) {
    const d = { ...d0 };
    if (d.clipSize > 0) disablePeriod = true;
    if (d.isInjector) {
      for (let k = 0; k < n; k++) heap.push({ t: 0, duration: d.duration, capNeed: d.capNeed, shot: 0, clip: d.clipSize, reload: d.reloadMs, inj: true, seq: seq++ });
      continue;
    }
    if (stagger && !d.disableStagger) {
      if (d.clipSize === 0) d.duration = Math.floor(d.duration / n);
      else {
        const st = (d.duration * d.clipSize + d.reloadMs) / (n * d.clipSize);
        for (let k = 1; k < n; k++) heap.push({ t: k * st, duration: d.duration, capNeed: d.capNeed, shot: 0, clip: d.clipSize, reload: d.reloadMs, inj: false, seq: seq++ });
      }
    } else d.capNeed *= n;
    const dur = Math.max(rnd(d.duration), 1);
    period = (period / gcd(period, dur)) * dur;
    heap.push({ t: 0, duration: d.duration, capNeed: d.capNeed, shot: 0, clip: d.clipSize, reload: d.reloadMs, inj: false, seq: seq++ });
  }
  const per = disablePeriod || period > tMaxMs ? tMaxMs : period;

  const capMax = capacity;
  let cap = capacity * startFrac;
  let capWrap = cap, capLowest = cap, capLowestPre = cap;
  let tWrap = per, tLast = 0, iterations = 0;
  const awaiting: Ev[] = [];
  let awaitingWrap: string = '';
  let ranOut = false;
  const key = (v: Ev[]) => v.map((e) => [e.duration, e.capNeed] as [number, number]).sort((x, y) => x[0] - y[0] || x[1] - y[1]).map((x) => `${x[0]}:${x[1]}`).join(',');
  let lastEv: Ev | null = null;
  let dtA = NaN, exA = 0, dtB = NaN, exB = 0;
  const fire = (inj: Ev, tNow: number) => {
    cap = Math.min(cap - inj.capNeed, capMax);
    inj.t = tNow + inj.duration;
    inj.shot++;
    if (inj.clip > 0 && inj.shot % inj.clip === 0) { inj.shot = 0; inj.t += inj.reload; }
    inj.seq = seq++;
    heap.push(inj);
  };
  // The event stays at the heap top while processed (re-armed with one sift, replaceTop) unless it leaves the heap
  // (break / awaiting) or other events are pushed meanwhile (fire). The order is a strict total order (seq), so the
  // processing sequence is identical to pop-then-push.
  for (;;) {
    if (heap.a.length === 0) break;
    const ev = heap.a[0];
    let atTop = true;
    const tNow = ev.t;
    if (tNow >= tMaxMs) { heap.pop(); lastEv = ev; break; }
    if (tNow > tLast && capMax > 0 && tau > 0) {
      const x = Math.sqrt(Math.max(cap / capMax, 0));
      // event gaps repeat (periodic modules): memoise exp() per gap (same value, the function is deterministic)
      const dt = tLast - tNow;
      let ex: number;
      if (dt === dtA) ex = exA;
      else if (dt === dtB) ex = exB;
      else {
        ex = Math.exp(dt / tau);
        dtB = dtA; exB = exA; dtA = dt; exA = ex;
      }
      const y = 1 + (x - 1) * ex;
      cap = y * y * capMax;
    }
    if (tNow !== tLast) {
      if (cap < capLowestPre) capLowestPre = cap;
      if (tNow === tWrap) {
        const k = key(awaiting);
        if (cap >= capWrap && k === awaitingWrap) { heap.pop(); lastEv = ev; break; }
        capWrap = rnd(cap * 10) / 10;
        awaitingWrap = k;
        tWrap += per;
      }
    }
    tLast = tNow;
    iterations++;
    if (iterations > 5_000_000) { heap.pop(); lastEv = ev; break; }
    if (ev.inj && cap - ev.capNeed > capMax) { heap.pop(); awaiting.push(ev); continue; }
    if (ev.capNeed > cap && cap < capMax) {
      if (awaiting.length > 0) { heap.pop(); atTop = false; }
      while (awaiting.length > 0 && ev.capNeed > cap && capMax > cap) {
        const need = Math.min(ev.capNeed - cap, capMax - cap);
        let pick = -1;
        // smallest injector that covers the need, else the largest
        for (let j = 0; j < awaiting.length; j++) if (-awaiting[j].capNeed >= need && (pick < 0 || -awaiting[j].capNeed < -awaiting[pick].capNeed)) pick = j;
        if (pick < 0) for (let j = 0; j < awaiting.length; j++) if (pick < 0 || -awaiting[j].capNeed >= -awaiting[pick].capNeed) pick = j;
        fire(awaiting.splice(pick, 1)[0], tNow);
      }
    }
    cap = Math.min(cap - ev.capNeed, capMax);
    if (cap < capLowest) {
      if (cap < 0) { if (atTop) heap.pop(); ranOut = true; lastEv = ev; break; }
      capLowest = cap;
    }
    if (atTop && awaiting.length > 0 && cap < capMax) { heap.pop(); atTop = false; }
    while (awaiting.length > 0 && cap < capMax) {
      const need = capMax - cap;
      let pick = -1;
      for (let j = 0; j < awaiting.length; j++) if (-awaiting[j].capNeed <= need && (pick < 0 || -awaiting[j].capNeed >= -awaiting[pick].capNeed)) pick = j;
      if (pick < 0) break;
      fire(awaiting.splice(pick, 1)[0], tNow);
    }
    ev.t = tNow + ev.duration;
    ev.shot++;
    if (ev.clip > 0 && ev.shot % ev.clip === 0) { ev.shot = 0; ev.t += ev.reload; }
    ev.seq = seq++;
    if (atTop) heap.replaceTop(ev);
    else heap.push(ev);
  }
  const all = heap.a.slice();
  if (lastEv) all.push(lastEv);
  const avgDrain = all.reduce((s, e) => s + e.capNeed / e.duration, 0);
  const inner = -(2 * avgDrain * tau - capMax) / capMax;
  const eveStable = inner >= 0 && capMax > 0 ? 0.25 * Math.pow(1 + Math.sqrt(inner), 2) : 0;
  const stable = !ranOut;
  return {
    stable,
    stableLow: stable && capMax > 0 ? capLowest / capMax : 0,
    stableHigh: stable && capMax > 0 ? capLowestPre / capMax : 0,
    tS: tLast / 1000,
    depletesInS: stable ? null : tLast / 1000,
    eveStable,
    iterations,
  };
}
