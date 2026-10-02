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
    sensor_strength: best[1], sensor_type: best[0],
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
