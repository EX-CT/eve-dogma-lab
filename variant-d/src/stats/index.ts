/** Compose stats sections into a FitStats object. */
import type { Fit } from '../core/fit.js';
import { NormRequest } from '../core/request.js';
import { capacitor } from './capacitor.js';
import { StatsCtx } from './ctx.js';
import { defense } from './defense.js';
import { drones, navigation, targeting } from './navigation.js';
import { offense } from './offense.js';
import { resources } from './resources.js';
import { tidy } from './util.js';
import { validate } from './validation.js';

export const ENGINE = 'eve-dogma-ts 0.1.0 (variant-d)';

export function computeStats(fit: Fit, req: NormRequest): unknown {
  const c = new StatsCtx(fit, req);
  const ds = fit.ds;
  const res = resources(c);
  const cap = capacitor(c);
  const st = ds.types.get(fit.items[fit.ship].typeId)!;
  const out: Record<string, unknown> = {
    meta: { schema_version: 1, engine: ENGINE, sde_build: ds.build, dataset_sha256: ds.sha256 },
    ship: { type_id: st.id, name: st.name, group: ds.groups.get(st.group)?.name ?? null },
    resources: res.json,
    offense: offense(c),
    defense: defense(c, cap),
    capacitor: cap.json,
    navigation: navigation(c),
    targeting: targeting(c),
    drones: drones(c),
    modules: cap.modules,
  };
  if (req.options.validate) out.violations = validate(c, res.used);
  if (fit.warnings.length) out.warnings = fit.warnings;
  const inc = req.options.include_attributes;
  if (inc === 'ship') out.attributes = { ship: dumpAttrs(fit, fit.ship) };
  else if (inc === 'all') {
    out.attributes = {
      ship: dumpAttrs(fit, fit.ship),
      character: dumpAttrs(fit, fit.char),
      modules: c.modules.map((i) => ({
        module_index: fit.items[i].reqIndex, type_id: fit.items[i].typeId, attributes: dumpAttrs(fit, i),
        charge: fit.items[i].charge >= 0 ? dumpAttrs(fit, fit.items[i].charge) : null,
      })),
      drones: c.drones.map((i) => ({ drone_index: fit.items[i].reqIndex, attributes: dumpAttrs(fit, i) })),
    };
  }
  return tidy(out);
}

export function dumpAttrs(fit: Fit, i: number): Record<string, number> {
  const it = fit.items[i];
  const keys = new Set<number>(it.tattrs.keys());
  if (it.base) for (const k of it.base.keys()) keys.add(k);
  if (it.ovA >= 0) keys.add(it.ovA);
  if (it.cells) for (const k of it.cells.keys()) keys.add(k);
  const out: Record<string, number> = {};
  for (const k of [...keys].sort((a, b) => a - b)) out[fit.ds.attrs.get(k)?.name ?? String(k)] = fit.get(i, k);
  return out;
}
