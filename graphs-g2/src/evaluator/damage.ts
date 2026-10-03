// damage + application_profile graphs. Application formulas are the public EVE ones (turret chance to hit,
// missile explosion formula); the behaviour around them (drone placement, projected webs/TPs re-applied to the
// target, time schedules) follows CONTRACT-GRAPHS.md. Written from the contract, not from Pyfa's code.
import { floatUnerr, foldExtended, rangeFactor, stackMultiply } from "./math.js";
import type { FitPrim, GraphRequest, ItemPrim, Primitives } from "./types.js";
import { GraphError } from "./types.js";
import { dealerSchedule, type TimeState } from "./timecache.js";

export type DealerKind = "turret" | "missile" | "smartbomb" | "vorton" | "bomb" | "doomsday" | "breacher" | "drone" | "fighter_attack" | "fighter_missiles" | "fighter_bomb" | "other";

export interface Dealer {
  kind: DealerKind;
  item: ItemPrim;
  dps: number[]; // em th ki ex (stats panel, spool 100%)
  volley: number[];
  count: number; // drones: active count already folded into dps/volley
  mobile: boolean;
  speed: number; // drones/fighters max velocity
  sig: number;
  optimal: number;
  falloff: number;
  tracking: number;
  optimalSig: number;
  eR: number;
  eV: number;
  drf: number;
  rangeLow: number;
  rangeHigh: number;
  rangeChance: number;
  fof: boolean;
  breacher?: { maxTick: number; pct: number; durationS: number };
}

/** fighter missiles: damage reduction exponent = ln(reductionFactor) / ln(reductionSensitivity) */
const drfExp = (rf?: number, rs?: number) => (rf && rs && rs !== 1 ? Math.log(rf) / Math.log(rs) : rf ?? 1);
const v4 = (o: any) => [o?.em ?? 0, o?.thermal ?? 0, o?.kinetic ?? 0, o?.explosive ?? 0];

/** Missile flight range data: lower / higher range and the chance to reach the higher one. */
function missileRanges(ship: FitPrim, charge: Record<string, number>, fofLimit: number | null) {
  const vel = charge.maxVelocity ?? 0;
  if (vel <= 0) return { lo: 0, hi: 0, chance: 0 };
  const radius = ship.ship.attrs.radius ?? 0;
  const ft = floatUnerr((charge.explosionDelay ?? 0) / 1000 + radius / vel);
  const accelCap = ((charge.mass ?? 0) * (charge.agility ?? 0)) / 1e6;
  const at = (t: number) => {
    const acc = Math.min(t, accelCap);
    return (vel / 2) * acc + vel * (t - acc);
  };
  const lt = Math.floor(ft);
  const ht = Math.ceil(ft);
  let lr = at(lt);
  let hr = at(ht);
  if (fofLimit !== null && fofLimit > 0) (lr = Math.min(lr, fofLimit)), (hr = Math.min(hr, fofLimit));
  lr = Math.max(lr - radius, 0);
  hr = Math.max(hr - radius, 0);
  return { lo: lr, hi: hr, chance: ft - lt };
}

export function dealers(p: FitPrim): Dealer[] {
  const out: Dealer[] = [];
  const st = p.stats.offense;
  const base = (it: ItemPrim, kind: DealerKind): Dealer => ({
    kind, item: it, dps: [0, 0, 0, 0], volley: [0, 0, 0, 0], count: 1, mobile: false, speed: 0, sig: 0, optimal: 0, falloff: 0, tracking: 0, optimalSig: 0,
    eR: 0, eV: 0, drf: 1, rangeLow: 0, rangeHigh: 0, rangeChance: 0, fof: false,
  });
  const modByIdx = new Map<number, ItemPrim>();
  for (const x of p.items) if (x.kind === "module" && x.index !== null) modByIdx.set(x.index, x);
  let d0Bomb: { flight: number; blast: number } | null = null;
  for (const w of st.weapons ?? []) {
    const it = modByIdx.get(w.module_index);
    if (!it) continue;
    const a = it.attrs;
    const eff = it.effects;
    let kind: DealerKind = (w.kind as DealerKind) ?? "other";
    const c = it.charge?.attrs ?? {};
    const ceff = it.charge?.effects ?? [];
    if (eff.includes("ChainLightning")) kind = "vorton";
    else if (a.doomsdayDamageDuration !== undefined || eff.some((e) => /^superWeapon|^lightningWeapon|^doomsday/i.test(e))) kind = "doomsday";
    if (ceff.some((e) => /bomb/i.test(e)) || /Bomb/.test(it.charge?.group ?? "")) kind = "bomb";
    if (ceff.some((e) => /dotMissile|breacher/i.test(e)) || /Breacher/i.test(it.charge?.group ?? "")) kind = "breacher";
    if (kind === "bomb") {
      d0Bomb = { flight: (c.maxVelocity ?? 0) * ((c.explosionDelay ?? 0) / 1000), blast: c.empFieldRange ?? c.explosionRange ?? 0 };
    }
    const d = base(it, kind);
    if (d0Bomb) (d.rangeLow = d0Bomb.flight - d0Bomb.blast), (d.rangeHigh = d0Bomb.flight + d0Bomb.blast), (d0Bomb = null);
    d.dps = v4(w.dps);
    d.volley = v4(w.volley);
    d.optimal = a.maxRange ?? 0;
    d.falloff = a.falloff ?? 0;
    d.tracking = a.trackingSpeed ?? 0;
    d.optimalSig = a.optimalSigRadius ?? 0;
    if (kind === "bomb") {
      d.eR = c.aoeCloudSize ?? 0;
      d.eV = c.aoeVelocity ?? 0;
      d.drf = c.aoeDamageReductionFactor ?? 1;
    } else if (kind === "missile" || kind === "breacher") {
      d.eR = c.aoeCloudSize ?? 0;
      d.eV = c.aoeVelocity ?? 0;
      d.drf = c.aoeDamageReductionFactor ?? 1;
      d.fof = ceff.includes("fofMissileLaunching");
      const r = missileRanges(p, c, d.fof ? c.maxFOFTargetRange ?? null : null);
      d.rangeLow = r.lo;
      d.rangeHigh = r.hi;
      d.rangeChance = r.chance;
    }
    if (kind === "vorton") {
      d.eR = a.aoeCloudSize ?? 0;
      d.eV = a.aoeVelocity ?? 0;
      d.drf = a.aoeDamageReductionFactor ?? 1;
      d.optimal = a.maxRange ?? 0;
      d.falloff = 0;
    }
    if (kind === "smartbomb") d.optimal = a.empFieldRange ?? 0;
    out.push(d);
  }
  // breacher pods: damage over time, not in the stats weapon list
  for (const it of p.items) {
    if (it.kind !== "module" || (it.state !== "active" && it.state !== "overheated") || !it.charge) continue;
    if (!it.charge.effects.includes("dotMissileLaunching")) continue;
    const c = it.charge.attrs;
    const d = base(it, "breacher");
    d.breacher = { maxTick: c.dotMaxDamagePerTick ?? 0, pct: c.dotMaxHPPercentagePerTick ?? 0, durationS: (c.dotDuration ?? 0) / 1000 };
    const r = missileRanges(p, c, null);
    d.rangeLow = r.lo;
    d.rangeHigh = r.hi;
    d.rangeChance = r.chance;
    out.push(d);
  }
  for (const dr of st.drones ?? []) {
    const it = p.items.find((x) => x.kind === "drone" && x.index === dr.drone_index);
    if (!it) continue;
    const d = base(it, "drone");
    d.dps = v4(dr.dps);
    d.volley = v4(dr.volley);
    d.count = dr.count;
    d.mobile = (it.attrs.maxVelocity ?? 0) >= 1; // sentries have a token 1.25e-5 m/s: they never follow
    d.speed = it.attrs.maxVelocity ?? 0;
    d.sig = it.attrs.signatureRadius ?? 0;
    d.optimal = dr.optimal_m;
    d.falloff = dr.falloff_m;
    d.tracking = dr.tracking;
    d.optimalSig = it.attrs.optimalSigRadius ?? 0;
    out.push(d);
  }
  for (const it of p.items) {
    if (it.kind !== "fighter" || !(it.active ?? 0)) continue;
    const a = it.attrs;
    const n = it.active ?? 0;
    for (const [eff, pre, kind] of [
      ["fighterAbilityAttackM", "fighterAbilityAttackMissile", "fighter_attack"],
      ["fighterAbilityMissiles", "fighterAbilityMissiles", "fighter_missiles"],
    ] as const) {
      if (!(it.abilities ?? []).includes(eff)) continue;
      const m = a[pre + "DamageMultiplier"] || 1;
      const vol = ["DamageEM", "DamageTherm", "DamageKin", "DamageExp"].map((k) => (a[pre + k] ?? 0) * m * n);
      const dur = (a[pre + "Duration"] ?? 0) / 1000;
      if (!vol.some((x) => x > 0)) continue;
      const d = base(it, kind);
      d.volley = vol;
      d.dps = dur > 0 ? vol.map((x) => x / dur) : [0, 0, 0, 0];
      d.mobile = true;
      d.speed = a.maxVelocity ?? 0;
      d.sig = a.signatureRadius ?? 0;
      if (kind === "fighter_attack") {
        d.eR = a.fighterAbilityAttackMissileExplosionRadius ?? 0;
        d.eV = a.fighterAbilityAttackMissileExplosionVelocity ?? 0;
        d.drf = drfExp(a.fighterAbilityAttackMissileReductionFactor, a.fighterAbilityAttackMissileReductionSensitivity);
        d.optimal = a.fighterAbilityAttackMissileRangeOptimal ?? 0;
        d.falloff = a.fighterAbilityAttackMissileRangeFalloff ?? 0;
      } else {
        d.eR = a.fighterAbilityMissilesExplosionRadius ?? 0;
        d.eV = a.fighterAbilityMissilesExplosionVelocity ?? 0;
        d.drf = drfExp(a.fighterAbilityMissilesDamageReductionFactor, a.fighterAbilityMissilesDamageReductionSensitivity);
        d.optimal = a.fighterAbilityMissilesRange ?? 0;
        d.falloff = 0;
      }
      out.push(d);
    }
  }
  return out;
}

// ---------------------------------------------------------------- application maths

export function turretMult(cth: number): number {
  const wreck = Math.min(cth, 0.01);
  const normal = Math.max(cth - 0.01, 0);
  const avg = (0.01 + cth) / 2 + 0.49;
  return floatUnerr(wreck * 3 + normal * avg);
}

export function turretCth(optimal: number, falloff: number, tracking: number, optimalSig: number, distCentre: number | null, distSurface: number | null, angular: number, sig: number): number {
  const rf = rangeFactor(optimal, falloff, distSurface, false);
  let tf = 1;
  if (angular > 0 && tracking > 0 && sig > 0) tf = Math.pow(0.5, Math.pow((angular * optimalSig) / (tracking * sig), 2));
  else if (angular > 0 && (tracking <= 0 || sig <= 0)) tf = 0;
  return rf * tf;
}

export function missileFactor(eR: number, eV: number, drf: number, sig: number, speed: number): number {
  if (sig === Infinity) return 1;
  if (eR <= 0) return 1;
  const a = sig / eR;
  const b = speed > 0 && eV > 0 ? Math.pow((eV * sig) / (eR * speed), drf) : Infinity;
  return floatUnerr(Math.min(1, a, b));
}

export interface Geometry {
  distance: number | null; // surface-to-surface
  atkSpeed: number;
  atkAngle: number;
  tgtSpeed: number;
  tgtAngle: number;
  tgtSig: number;
  atkRadius: number;
  tgtRadius: number;
}

const rad = (deg: number) => (deg * Math.PI) / 180;

function transversal(vA: number, aA: number, vT: number, aT: number) {
  return Math.abs(vA * Math.sin(rad(aA)) - vT * Math.sin(rad(aT)));
}

export function applicationFactor(d: Dealer, g: Geometry, settings: any, dcr: number, lockRange: number): number {
  const dist = g.distance;
  if (!settings.ignore_lock_range && dist !== null && d.kind !== "smartbomb" && !d.fof && dist > lockRange) return 0;
  switch (d.kind) {
    case "turret": {
      const ctr = dist === null ? null : g.atkRadius + dist + g.tgtRadius;
      const ang = ctr === null ? 0 : transversal(g.atkSpeed, g.atkAngle, g.tgtSpeed, g.tgtAngle) / ctr;
      return turretMult(turretCth(d.optimal, d.falloff, d.tracking, d.optimalSig, ctr, dist, ang, g.tgtSig));
    }
    case "bomb": {
      if (dist !== null && (dist < d.rangeLow - 1e-9 || dist > d.rangeHigh + 1e-9)) return 0;
      return missileFactor(d.eR, d.eV, d.drf, g.tgtSig, g.tgtSpeed);
    }
    case "missile": {
      let df = 1;
      if (dist !== null) {
        if (dist <= d.rangeLow) df = 1;
        else if (dist <= d.rangeHigh) df = d.rangeChance;
        else df = 0;
      }
      return df * missileFactor(d.eR, d.eV, d.drf, g.tgtSig, g.tgtSpeed);
    }
    case "breacher": {
      let df = 1;
      if (dist !== null) df = dist <= d.rangeLow ? 1 : dist <= d.rangeHigh ? d.rangeChance : 0;
      return df;
    }
    case "smartbomb":
      return dist === null || dist <= d.optimal ? 1 : 0;
    case "vorton":
      return rangeFactor(d.optimal, 0, dist, false) * missileFactor(d.eR, d.eV, d.drf, g.tgtSig, g.tgtSpeed);
    case "doomsday":
      if (dist !== null && dist > (d.item.attrs.maxRange ?? Infinity)) return 0;
      return g.tgtSig === Infinity ? 1 : Math.min(1, g.tgtSig / (d.item.attrs.signatureRadius || g.tgtSig));
    case "drone":
    case "fighter_attack":
    case "fighter_missiles": {
      if (!settings.ignore_drone_control_range && dist !== null && dist > dcr && d.kind === "drone") return 0;
      const mode = settings.mobile_drone_mode ?? "auto";
      const atTarget = d.mobile && (mode === "follow_target" || (mode === "auto" && d.speed > g.tgtSpeed));
      if (d.kind === "drone") {
        if (atTarget) return turretMult(1);
        // at the attacker's centre, moving with it (sentries: standing still)
        const vD = d.mobile ? Math.min(g.atkSpeed, d.speed) : 0;
        const ctr = dist === null ? null : g.atkRadius + dist + g.tgtRadius;
        // the drone sits at the attacker's centre: range is measured from the drone's surface
        const surf = dist === null ? null : g.atkRadius + dist - (d.item.attrs.radius ?? 0);
        const ang = ctr === null ? 0 : transversal(vD, g.atkAngle, g.tgtSpeed, g.tgtAngle) / ctr;
        return turretMult(turretCth(d.optimal, d.falloff, d.tracking, d.optimalSig, ctr, surf, ang, g.tgtSig));
      }
      const mf = missileFactor(d.eR, d.eV, d.drf, g.tgtSig, g.tgtSpeed);
      if (atTarget) return mf;
      // fighters left at the attacker's centre: range is measured from the fighter's surface (like drones)
      const surf = dist === null ? null : g.atkRadius + dist - (d.item.attrs.radius ?? 0);
      const rf = d.kind === "fighter_attack" ? rangeFactor(d.optimal, d.falloff, surf, true) : surf === null || surf <= d.optimal ? 1 : 0;
      return rf * mf;
    }
  }
  return 1;
}

// ---------------------------------------------------------------- target

export interface TargetModel {
  maxSpeed: number;
  sig: number;
  radius: number;
  resists: number[]; // fraction 0..1 per damage type (em th ki ex)
  fit?: FitPrim;
  scrammed?: FitPrim;
  immune: boolean;
  hp: number;
}

function fitResists(t: FitPrim, mode: string): number[] {
  const a = t.ship.attrs;
  const L = (pre: string) =>
    pre === "" ? [a.emDamageResonance, a.thermalDamageResonance, a.kineticDamageResonance, a.explosiveDamageResonance] : [a[pre + "EmDamageResonance"], a[pre + "ThermalDamageResonance"], a[pre + "KineticDamageResonance"], a[pre + "ExplosiveDamageResonance"]];
  const layers = { shield: L("shield"), armor: L("armor"), hull: L("") };
  const hp = { shield: a.shieldCapacity ?? 0, armor: a.armorHP ?? 0, hull: a.hp ?? 0 };
  let res: number[];
  if (mode === "shield" || mode === "armor" || mode === "hull") res = layers[mode];
  else if (mode === "weighted_average") {
    // per damage type: total HP / total EHP (the resonance that turns raw damage into HP removed over all layers)
    const tot = hp.shield + hp.armor + hp.hull;
    res = [0, 1, 2, 3].map((i) => {
      const ehp = (["shield", "armor", "hull"] as const).reduce((s, l) => s + (layers[l][i] > 0 ? hp[l] / layers[l][i] : 0), 0);
      return ehp > 0 ? tot / ehp : 1;
    });
  } else res = autoLayer(t, layers, hp);
  return res.map((r) => 1 - r);
}

/** "auto": the layer that matters most for the target: score EHP share, resist and active tank. */
function autoLayer(t: FitPrim, layers: Record<string, number[]>, hp: Record<string, number>): number[] {
  const tank = t.stats.defense?.tank ?? {};
  const avg = (r: number[]) => r.reduce((s, x) => s + x, 0) / 4;
  let best = "hull";
  let bestScore = -Infinity;
  for (const l of ["shield", "armor", "hull"]) {
    const ehp = hp[l] / Math.max(avg(layers[l]), 1e-9);
    const rep = (tank?.[l]?.reinforced ?? 0) + (l === "shield" ? tank?.shield?.passive ?? 0 : 0);
    const score = ehp + rep * 60;
    if (score > bestScore) (bestScore = score), (best = l);
  }
  return layers[best];
}

export function targetModel(req: GraphRequest, p: Primitives): TargetModel {
  const t = req.target ?? {};
  if (t.fit && p.target) {
    const f = p.target.normal;
    const nav = f.stats.navigation;
    return {
      maxSpeed: nav.max_velocity, sig: f.ship.attrs.signatureRadius, radius: f.ship.attrs.radius ?? 0, resists: fitResists(f, t.resist_mode ?? "auto"),
      fit: f, scrammed: p.target.scrammed, immune: (f.ship.attrs.disallowOffensiveModifiers ?? 0) > 0,
      hp: (f.ship.attrs.shieldCapacity ?? 0) + (f.ship.attrs.armorHP ?? 0) + (f.ship.attrs.hp ?? 0),
    };
  }
  const pr = t.profile ?? {};
  return {
    maxSpeed: pr.max_velocity ?? 0,
    sig: pr.signature_radius === null || pr.signature_radius === undefined ? Infinity : pr.signature_radius,
    radius: pr.radius ?? 0,
    resists: [pr.em ?? 0, pr.thermal ?? 0, pr.kinetic ?? 0, pr.explosive ?? 0],
    immune: false,
    hp: pr.hp === null || pr.hp === undefined ? Infinity : pr.hp,
  };
}

// ---------------------------------------------------------------- projected webs / TPs / scram

interface Projector {
  kind: "web" | "tp" | "scram";
  value: number; // speedFactor / signatureRadiusBonus (%)
  range: number;
  falloff: number;
  mobile: boolean;
  count: number;
  resistAttr?: string;
}

export function projectors(p: FitPrim): Projector[] {
  const out: Projector[] = [];
  for (const it of p.items) {
    const active = it.kind === "module" ? it.state === "active" || it.state === "overheated" : (it.active ?? 0) > 0;
    if (!active) continue;
    for (const [name, er] of Object.entries(it.effect_ranges ?? {})) {
      let kind: Projector["kind"] | null = null;
      if (/Webifier|AOEWeb/i.test(name) || name === "fighterAbilityStasisWebifier") kind = "web";
      else if (/TargetPaint|AOEPaint/i.test(name)) kind = "tp";
      else if (/warpScramble|WarpScram/i.test(name)) kind = "scram";
      if (!kind) continue;
      const value = kind === "web" ? it.attrs.speedFactor ?? it.attrs.fighterAbilityStasisWebifierSpeedPenalty ?? 0 : kind === "tp" ? it.attrs.signatureRadiusBonus ?? 0 : 0;
      out.push({ kind, value, range: er.range ?? 0, falloff: er.falloff ?? 0, mobile: it.kind !== "module", count: it.kind === "module" ? 1 : it.active ?? 1, resistAttr: er.resistance_attr });
      break;
    }
  }
  return out;
}

/**
 * Identical weapons (same application inputs and cycle) are evaluated once: their dps / volley vectors add up and
 * every application factor and schedule is linear in them.
 */
function mergeDealers(ds: Dealer[]): Dealer[] {
  const out: Dealer[] = [];
  const byKey = new Map<string, Dealer>();
  for (const d of ds) {
    if (d.kind !== "turret" && d.kind !== "missile" && d.kind !== "drone") {
      out.push(d);
      continue;
    }
    const a = d.item.attrs;
    const c = d.item.cycle;
    const key = [d.kind, d.item.kind, d.mobile, d.speed, d.sig, d.optimal, d.falloff, d.tracking, d.optimalSig, d.eR, d.eV, d.drf,
      d.rangeLow, d.rangeHigh, d.rangeChance, d.fof, c?.raw_ms, c?.reactivation_ms, c?.reload_ms, c?.shots, c?.avg_ms,
      a.damageMultiplierBonusPerCycle, a.damageMultiplierBonusMax, a.radius].join("|");
    const m = byKey.get(key);
    if (!m) {
      const c = { ...d, dps: d.dps.slice(), volley: d.volley.slice() };
      byKey.set(key, c);
      out.push(c);
    } else {
      for (let i = 0; i < 4; i++) (m.dps[i] += d.dps[i]), (m.volley[i] += d.volley[i]);
    }
  }
  return out;
}

export function damageGraph(req: GraphRequest, p: Primitives, only: Set<number> | null = null): Record<string, (number | null)[]> {
  const settings = { ignore_resists: true, apply_projected: true, ignore_lock_range: true, ignore_drone_control_range: false, mobile_drone_mode: "auto", ...(req.settings ?? {}) };
  const prm = req.params ?? {};
  const src = p.source;
  let ds = only ? dealers(src).filter((d) => d.item.kind === "module" && only.has(d.item.index as number)) : dealers(src);
  if ((settings as any)._app) {
    // application profile: turrets and missiles only, without spool-up
    ds = ds.filter((d) => d.kind === "turret" || d.kind === "missile").map((d) => {
      const mb = d.item.attrs.damageMultiplierBonusMax ?? 0;
      return mb > 0 ? { ...d, dps: d.dps.map((v) => v / (1 + mb)), volley: d.volley.map((v) => v / (1 + mb)) } : d;
    });
  }
  ds = mergeDealers(ds);
  // application_profile: target model and projectors are the same for every charge variant (shared context)
  const ctx: any = (settings as any)._ctx;
  const tgt: TargetModel = ctx ? (ctx.tgt ??= targetModel(req, p)) : targetModel(req, p);
  const projs = !settings.apply_projected ? [] : ctx ? (ctx.projs ??= projectors(src)) : projectors(src);
  const dcr = src.stats.drones?.control_range_m ?? Infinity;
  const lockRange = src.stats.targeting?.max_range_m ?? Infinity;
  const atkMax = src.stats.navigation.max_velocity;
  const atkSpeed = prm.atk_speed_mps ?? ((prm.atk_speed_pct ?? 0) / 100) * atkMax;
  const atkAngle = prm.atk_angle_deg ?? 90;
  const tgtAngle = prm.tgt_angle_deg ?? 90;
  const atkRadius = src.ship.attrs.radius ?? 0;

  const point = (x: number) => {
    let distance: number | null = prm.distance_m ?? null;
    let time: number | null = prm.time_s ?? null;
    let tgtSpeedAbs: number | null = prm.tgt_speed_mps ?? null;
    let tgtSigBase = tgt.sig;
    switch (req.x.axis) {
      case "distance_m":
        if (x < 0) return null;
        distance = x;
        break;
      case "time_s":
        if (x < 0 || x > 2500) return null;
        time = x;
        break;
      case "tgt_speed_mps":
        if (x < 0) return null;
        tgtSpeedAbs = x;
        break;
      case "tgt_sig_m":
        if (!(x > 0)) return null;
        tgtSigBase = x;
        break;
      default:
        throw new GraphError("BAD_AXIS", `damage has no x axis ${req.x.axis}`, "x.axis");
    }
    // target speed / signature after the source's projected effects
    let maxSpeed = tgt.maxSpeed;
    let fit = tgt.fit;
    const webM: number[] = [];
    const tpM: number[] = [];
    if (!tgt.immune) {
      for (const pj of projs) {
        const rf = pj.mobile ? (distance === null || settings.ignore_drone_control_range || distance <= dcr ? 1 : 0) : rangeFactor(pj.range, pj.falloff, distance, true);
        if (rf <= 0) continue;
        let res = 1;
        if (fit && pj.resistAttr && fit.ship.attrs[pj.resistAttr] !== undefined) res = fit.ship.attrs[pj.resistAttr];
        for (let k = 0; k < pj.count; k++) {
          if (pj.kind === "web") webM.push(1 + (pj.value * rf * res) / 100);
          else if (pj.kind === "tp") tpM.push(1 + (pj.value * rf * res) / 100);
          else if (pj.kind === "scram" && tgt.scrammed) fit = tgt.scrammed;
        }
      }
    }
    let sig: number;
    if (fit) {
      const mv0 = tgt.fit!.stats.navigation.max_velocity;
      maxSpeed = foldExtended(fit.ship.stack.maxVelocity, webM);
      const sl = fit.stats.navigation; // speed limit not modelled for targets
      void sl;
      const speedPct = tgtSpeedAbs !== null ? null : (prm.tgt_speed_pct ?? 100) / 100;
      const tgtSpeed = speedPct !== null ? maxSpeed * speedPct : Math.min(tgtSpeedAbs!, mv0) * (mv0 > 0 ? maxSpeed / mv0 : 1);
      sig = req.x.axis === "tgt_sig_m" ? tgtSigBase * stackMultiply(tpM) : foldExtended(fit.ship.stack.signatureRadius, tpM);
      return { distance, time, tgtSpeed, sig };
    }
    const baseSpeed = tgtSpeedAbs !== null ? tgtSpeedAbs : ((prm.tgt_speed_pct ?? 100) / 100) * maxSpeed;
    const tgtSpeed = baseSpeed * stackMultiply(webM);
    sig = tgtSigBase * stackMultiply(tpM);
    return { distance, time, tgtSpeed, sig };
  };

  const resMul = settings.ignore_resists ? [1, 1, 1, 1] : tgt.resists.map((r) => 1 - r);
  const dmgOf = (v: number[]) => v.reduce((s, x, i) => s + x * resMul[i], 0);
  const tMax = req.x.axis === "time_s" ? Math.max(0, ...req.x.values.filter((v) => v <= 2500)) : (prm.time_s ?? 0);
  const schedules = new Map<Dealer, TimeState>();
  const sched = (d: Dealer) => {
    let s = schedules.get(d);
    if (!s) schedules.set(d, (s = dealerSchedule(d, src, tMax)));
    return s;
  };

  // target geometry per x does not depend on the source's charges: application_profile shares it across variants
  const ptCache: Map<number, ReturnType<typeof point>> | undefined = (settings as any)._ptCache;
  const pointC = (x: number) => {
    if (!ptCache) return point(x);
    if (!ptCache.has(x)) ptCache.set(x, point(x));
    return ptCache.get(x)!;
  };
  const out: Record<string, (number | null)[]> = {};
  for (const y of req.y) out[y] = [];
  for (const x of req.x.values) {
    let pt = pointC(x);
    // application_profile: the target's speed / signature after projected effects is sampled on a distance grid
    // and linearly interpolated between grid nodes (observed Pyfa behaviour, see DESIGN.md)
    const grid: number = (settings as any)._targetGrid ?? 0;
    if (pt !== null && grid > 0 && req.x.axis === "distance_m" && x % grid !== 0) {
      const lo = Math.floor(x / grid) * grid;
      const a = pointC(lo);
      const b = pointC(lo + grid);
      if (a && b) {
        const t = (x - lo) / grid;
        pt = { ...pt, tgtSpeed: a.tgtSpeed + (b.tgtSpeed - a.tgtSpeed) * t, sig: a.sig + (b.sig - a.sig) * t };
      }
    }
    if (pt === null) {
      for (const y of req.y) out[y].push(null);
      continue;
    }
    const g: Geometry = { distance: pt.distance, atkSpeed, atkAngle, tgtSpeed: pt.tgtSpeed, tgtAngle, tgtSig: pt.sig, atkRadius, tgtRadius: tgt.radius };
    let dps = 0;
    let volley = 0;
    let damage = 0;
    let breach = 0;
    for (const d of ds) {
      if (d.kind === "breacher") {
        const f = applicationFactor(d, g, settings, dcr, lockRange);
        if (f <= 0) continue;
        const hp = tgt.hp;
        const tick = Math.min(d.breacher!.maxTick, hp === Infinity ? Infinity : (d.breacher!.pct / 100) * hp);
        // range chance (flight time between whole seconds) scales the tick like any missile
        breach = Math.max(breach, (tick === Infinity ? d.breacher!.maxTick : tick) * f);
        continue;
      }
      const f = applicationFactor(d, g, settings, dcr, lockRange);
      if (pt.time === null) {
        dps += dmgOf(d.dps) * f;
        volley += dmgOf(d.volley) * f;
      } else {
        const s = sched(d).at(pt.time);
        dps += dmgOf(s.dps) * f;
        volley += dmgOf(s.volley) * f;
        damage += dmgOf(s.total) * f;
      }
    }
    if (breach > 0) {
      // breacher DoT: one pod at a time (the strongest), ticking every second from 1 s after launch
      if (pt.time === null) (dps += breach), (volley += breach);
      else if (pt.time >= 1 - 1e-9) {
        dps += breach;
        volley += breach;
        damage += breach * Math.max(0, Math.floor(floatUnerr(pt.time)) - 1);
      }
    }
    for (const y of req.y) {
      if (y === "dps") out[y].push(dps);
      else if (y === "volley") out[y].push(volley);
      else if (y === "damage") out[y].push(pt.time === null ? null : damage);
      else throw new GraphError("BAD_AXIS", `damage has no series ${y}`, "y");
    }
  }
  return out;
}
