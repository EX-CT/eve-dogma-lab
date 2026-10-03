// Cycle-by-cycle damage schedule of one dealer from t = 0 (reloads always included, spool by cycles).
import { floatUnerr } from "./math.js";
import type { FitPrim } from "./types.js";
import type { Dealer } from "./damage.js";

export interface TimeState {
  at(t: number): { dps: number[]; volley: number[]; total: number[] };
}

/** tMax: the latest time that will be queried (events after it are not built). */
export function dealerSchedule(d: Dealer, _p: FitPrim, tMax = 2500): TimeState {
  const end = Math.min(2500, tMax) + 1e-6;
  const it = d.item;
  const cyc = it.cycle;
  // events: [time s, volley vector, dps vector (for the cycle that starts here)]
  const ev: { t: number; volley: number[]; dps: number[] }[] = [];
  const zero = [0, 0, 0, 0];
  if (it.kind === "module" && cyc && d.kind === "doomsday" && it.attrs.doomsdayDamageDuration) {
    // doomsday / lance: ticks every doomsdayDamageCycleTime after the warning delay; dps/volley as the stats panel
    const period = cyc.avg_ms / 1000;
    const warn = (it.attrs.doomsdayWarningDuration ?? 0) / 1000;
    const sub = (it.attrs.doomsdayDamageCycleTime ?? 0) / 1000;
    const n = sub > 0 ? Math.max(Math.floor(floatUnerr((it.attrs.doomsdayDamageDuration ?? 0) / 1000 / sub)), 0) : 1;
    const ticks: number[] = [];
    for (let c = 0; c <= end && period > 0; c += period) for (let k = 0; k < n; k++) ticks.push(c + warn + k * sub);
    return {
      at(t: number) {
        const tu = floatUnerr(t);
        const k = ticks.filter((x) => floatUnerr(x) <= tu).length;
        return { dps: d.dps, volley: d.volley, total: d.volley.map((v) => v * k) };
      },
    };
  }
  if (it.kind === "module" && cyc) {
    const active = cyc.raw_ms / 1000;
    const inactive = cyc.reactivation_ms / 1000;
    const reload = cyc.reload_ms / 1000;
    const shots = cyc.shots;
    const full = active + inactive;
    const unspooled = d.volley.map((v) => v / (it.attrs.damageMultiplierBonusMax ? 1 + it.attrs.damageMultiplierBonusMax : 1));
    const step = it.attrs.damageMultiplierBonusPerCycle ?? 0;
    const maxB = it.attrs.damageMultiplierBonusMax ?? 0;
    let t = 0;
    let n = 0;
    let shot = 0;
    let spoolN = 0;
    while (t <= end && n < 200000) {
      const mult = 1 + Math.min(step * spoolN, maxB);
      const vol = unspooled.map((v) => v * mult);
      let len = full;
      shot++;
      let reloadNow = false;
      if (shots > 0 && shot >= shots && reload > 0) (reloadNow = true), (shot = 0);
      if (reloadNow) len = active + Math.max(reload, inactive);
      ev.push({ t, volley: vol, dps: vol.map((v) => v / active) });
      // reactivation delay / reloading: no damage until the next cycle starts
      if (reloadNow || inactive > 0) ev.push({ t: t + active, volley: zero, dps: zero });
      t += len;
      n++;
      spoolN = reloadNow ? 0 : spoolN + 1;
    }
  } else {
    // drones / fighters: fixed cycle
    const cycS = d.volley.some((v) => v > 0) && d.dps.some((v) => v > 0) ? d.volley[d.dps.findIndex((v) => v > 0)] / d.dps[d.dps.findIndex((v) => v > 0)] : 0;
    let t = 0;
    let n = 0;
    while (cycS > 0 && t <= end && n < 200000) {
      ev.push({ t, volley: d.volley, dps: d.dps });
      t += cycS;
      n++;
    }
  }
  const cum: number[][] = [];
  let acc = [0, 0, 0, 0];
  for (const e of ev) {
    acc = acc.map((x, i) => x + e.volley[i]);
    cum.push(acc);
  }
  return {
    at(t: number) {
      const tu = floatUnerr(t);
      let lo = -1;
      let a = 0;
      let b = ev.length - 1;
      while (a <= b) {
        const m = (a + b) >> 1;
        if (floatUnerr(ev[m].t) <= tu) (lo = m), (a = m + 1);
        else b = m - 1;
      }
      if (lo < 0) return { dps: zero, volley: zero, total: zero };
      return { dps: ev[lo].dps, volley: ev[lo].volley, total: cum[lo] };
    },
  };
}
