import { simulateCapHistory } from "./capsim.js";
import { regenLevel, regenRate } from "./simple.js";
import type { GraphRequest, Primitives } from "./types.js";
import { GraphError } from "./types.js";

export function capacitor(req: GraphRequest, p: Primitives): Record<string, (number | null)[]> {
  const a = p.source.ship.attrs;
  const C = a.capacitorCapacity ?? 0;
  const rechargeMs = a.rechargeRate ?? 0;
  const tau = rechargeMs / 1000;
  const prm = req.params ?? {};
  const startFrac = (prm.cap_start_pct ?? 100) / 100;
  const useSim = prm.use_capsim ?? true;
  const drains = p.source.cap_drains;
  let level: (x: number) => number | null;
  if (req.x.axis === "cap_pct") {
    level = (x) => (x < 0 || x > 100 ? null : (x / 100) * C);
  } else if (req.x.axis === "time_s") {
    if (useSim && drains.length) {
      const reload = !!req.fit?.options?.factor_reload;
      const sim = simulateCapHistory(C, rechargeMs, drains, startFrac, reload, true, 3600 * 1000);
      level = (t) => {
        if (t < 0 || t > 3600) return null;
        const pts = sim.points;
        let lo = 0;
        let hi = pts.length - 1;
        if (t < pts[0][0]) return null;
        while (lo < hi) {
          const mid = (lo + hi + 1) >> 1;
          if (pts[mid][0] <= t) lo = mid;
          else hi = mid - 1;
        }
        const [t0, c0] = pts[lo];
        if (sim.ranOut && lo === pts.length - 1 && t > t0) return null;
        return regenLevel(C, tau, Math.max(c0, 0), t - t0);
      };
    } else {
      level = (t) => (t < 0 || t > 3600 ? null : regenLevel(C, tau, startFrac * C, t));
    }
  } else throw new GraphError("BAD_AXIS", `capacitor has no x axis ${req.x.axis}`, "x.axis");
  const out: Record<string, (number | null)[]> = {};
  for (const y of req.y) {
    out[y] = req.x.values.map((x) => {
      const c = level(x);
      if (c === null) return null;
      if (y === "cap_gj") return c;
      if (y === "cap_regen_gj_s") return regenRate(C, tau, c);
      throw new GraphError("BAD_AXIS", `capacitor has no series ${y}`, "y");
    });
  }
  return out;
}
