import { StatsCtx } from './ctx.js';
import { lockTime } from './util.js';

export function navigation(c: StatsCtx): object {
  const { fit } = c;
  const ship = fit.ship;
  const maxv = c.g(ship, 'maxVelocity');
  const limit = c.g(ship, 'speedLimit');
  const mass = fit.get(ship, 4);
  const agility = c.g(ship, 'agility');
  const bw = c.g(ship, 'baseWarpSpeed') || 1;
  const wm = c.g(ship, 'warpSpeedMultiplier') || 1;
  const warpNeed = c.g(ship, 'warpCapacitorNeed');
  const cap = c.g(ship, 'capacitorCapacity');
  return {
    max_velocity: limit > 0 && maxv > limit ? limit : maxv,
    align_time_s: (-Math.log(0.25) * agility * mass) / 1e6,
    mass, agility, signature_radius: c.g(ship, 'signatureRadius'),
    warp_speed_au_s: bw * wm,
    max_warp_distance_au: warpNeed > 0 && mass > 0 ? cap / (mass * warpNeed) : 0,
    warp_scramble_status: c.g(ship, 'warpScrambleStatus'),
  };
}

export function targeting(c: StatsCtx): object {
  const { fit, req } = c;
  const ship = fit.ship;
  let best: [string, number] = ['none', 0];
  for (const [n, a] of [['radar', 'scanRadarStrength'], ['ladar', 'scanLadarStrength'], ['magnetometric', 'scanMagnetometricStrength'], ['gravimetric', 'scanGravimetricStrength']]) {
    const v = c.g(ship, a);
    if (v > best[1]) best = [n, v];
  }
  const scanRes = c.g(ship, 'scanResolution');
  const sig = c.g(ship, 'signatureRadius');
  const lt = (s: number) => lockTime(scanRes, s);
  const tpSig = req.target_profile?.signature_radius;
  return {
    max_targets: Math.min(c.g(ship, 'maxLockedTargets'), Math.max(c.g(fit.char, 'maxLockedTargets'), 0)),
    max_range_m: c.g(ship, 'maxTargetRange'), scan_resolution: scanRes,
    sensor_strength: best[1], sensor_type: best[0], jam_chance_percent: jamChance(c),
    probe_size: best[1] > 0 ? Math.max(sig / best[1], 1.08) : null,
    lock_time_s: { sig_25m: lt(25), sig_40m: lt(40), sig_125m: lt(125), sig_400m: lt(400), sig_target_profile: tpSig != null ? lt(tpSig) : null },
  };
}

export function drones(c: StatsCtx): object {
  return {
    active: c.drones.reduce((s, i) => s + c.item(i).activeCount, 0),
    max_active: c.g(c.fit.char, 'maxActiveDrones'),
    control_range_m: c.g(c.fit.char, 'droneControlDistance'),
  };
}

/** ECM jam chance (Pyfa Fit.jamChance): strengths vs the strongest sensor type (ties -> multispectral -> 0) */
const SENSOR_TYPES = ['Magnetometric', 'Ladar', 'Radar', 'Gravimetric'] as const;
const SENSOR_ATTRS = ['scanMagnetometricStrength', 'scanLadarStrength', 'scanRadarStrength', 'scanGravimetricStrength'] as const;

function jamChance(c: StatsCtx): number {
  const { fit } = c;
  const ship = fit.ship;
  let maxS = -1;
  let ty: string | null = null;
  for (let k = 0; k < SENSOR_TYPES.length; k++) {
    const t = SENSOR_TYPES[k];
    const v = c.g(ship, SENSOR_ATTRS[k]);
    if (v > maxS) { maxS = v; ty = t; } else if (v === maxS) ty = null;
  }
  let retain = 1;
  let any = false;
  for (const ps of fit.projSpecial) {
    if (ps.kind !== 'ecm') continue;
    any = true;
    if (ty === null) continue;
    let st = c.g(ps.item, ps.fighter ? `fighterAbilityECMStrength${ty}` : `scan${ty}StrengthBonus`) * ps.factor;
    if (ps.resist !== 0) {
      const r = fit.get(ship, ps.resist);
      if (r !== 0) st *= r;
    }
    if (maxS > 0) retain *= 1 - Math.min(st / maxS, 1);
  }
  return any ? (1 - retain) * 100 : 0;
}
