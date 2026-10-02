/** EFT text import/export incl. mutation blocks (same behaviour as eve-dogma-rs eft.rs). */
import { Dataset } from '../core/dataset.js';
import { inferSlot } from '../core/fit.js';
import { FitRequest, ModuleReq, Mutation } from '../core/request.js';

const isHead = (l: string) => {
  const t = l.trim();
  if (!t.startsWith('[')) return false;
  const e = t.indexOf(']');
  return e > 1 && /^\d+$/.test(t.slice(1, e));
};

function parseMutations(ds: Dataset, lines: string[]): [Map<number, Mutation>, number] {
  const out = new Map<number, Mutation>();
  let first = lines.findIndex(isHead);
  if (first < 0) first = lines.length;
  let i = first;
  while (i < lines.length) {
    const t = lines[i].trim();
    if (!isHead(t)) { i++; continue; }
    const e = t.indexOf(']');
    const n = Number(t.slice(1, e));
    const baseName = t.slice(e + 1).trim();
    const base = ds.typeByName(baseName);
    if (base === undefined) throw new Error(`unknown mutated base '${baseName}'`);
    const m: Mutation = { base_type_id: base, mutaplasmid_type_id: null, attributes: {} };
    i++;
    while (i < lines.length && !isHead(lines[i])) {
      const l = lines[i].trim();
      i++;
      if (!l) continue;
      if (m.mutaplasmid_type_id == null) {
        const id = ds.typeByName(l);
        if (id === undefined) throw new Error(`unknown mutaplasmid '${l}'`);
        m.mutaplasmid_type_id = id;
        continue;
      }
      for (const kv0 of l.split(',')) {
        const kv = kv0.trim();
        const p = kv.lastIndexOf(' ');
        if (p < 0) continue;
        const aid = ds.attrId(kv.slice(0, p).trim());
        const vs = kv.slice(p + 1).trim();
        const v = Number(vs);
        if (aid !== 0 && vs !== '' && Number.isFinite(v)) m.attributes![String(aid)] = v;
      }
    }
    out.set(n, m);
  }
  return [out, first];
}

function mutatedType(ds: Dataset, m: Mutation): number {
  const mu = m.mutaplasmid_type_id != null ? ds.mutaplasmids.get(m.mutaplasmid_type_id) : undefined;
  return mu?.mapping.find((x) => x.inputs.includes(m.base_type_id))?.output ?? m.base_type_id;
}

/** strip a trailing " [N]" mutation reference */
function mutRef(line: string): [string, number | null] {
  const l = line.trimEnd();
  if (l.endsWith(']')) {
    const p = l.lastIndexOf(' [');
    if (p >= 0 && /^\d+$/.test(l.slice(p + 2, -1))) return [l.slice(0, p).trimEnd(), Number(l.slice(p + 2, -1))];
  }
  return [l, null];
}

export function parseEft(ds: Dataset, text: string): FitRequest {
  const all = text.split(/\r?\n/);
  const [muts, firstMut] = parseMutations(ds, all);
  const lines = all.slice(0, firstMut).map((l) => l.trim()).filter((l) => l.length > 0);
  const header = lines.shift();
  if (header === undefined) throw new Error('empty EFT');
  const h = header.replace(/^\[+/, '').replace(/\]+$/, '');
  const shipName = h.split(',')[0].trim();
  const ship = ds.typeByName(shipName);
  if (ship === undefined) throw new Error(`unknown ship '${shipName}'`);
  const modules: ModuleReq[] = [];
  const req = {
    schema_version: 1, ship: { type_id: ship, mode_type_id: null as number | null },
    character: { skills: { default_level: null as number | null, levels: {} }, security_status: null },
    modules, drones: [] as any[], fighters: [] as any[], implants: [] as number[], boosters: [] as any[], cargo: [] as any[],
    fleet: { buffs: [], booster_fits: [] }, projected: [], environment: { effect_type_ids: [], system_security: null },
    damage_pattern: null, target_profile: null, overrides: [], options: { validate: true },
  };
  for (let line of lines) {
    if (line.startsWith('[Empty')) continue;
    let offline = false;
    if (line.endsWith('/OFFLINE') || line.endsWith('/offline')) { line = line.slice(0, -8).trim(); offline = true; }
    const [l2, mref] = mutRef(line);
    line = l2;
    let mutation: Mutation | null = null;
    if (mref !== null) {
      const mu = muts.get(mref);
      if (!mu) throw new Error(`mutation [${mref}] not defined`);
      mutation = mu;
    }
    const xp = line.lastIndexOf(' x');
    if (xp >= 0 && /^\d+$/.test(line.slice(xp + 2).trim())) {
      const n = Number(line.slice(xp + 2).trim());
      const name = line.slice(0, xp).trim();
      let tid = ds.typeByName(name);
      if (tid === undefined) throw new Error(`unknown item '${name}'`);
      if (mutation) tid = mutatedType(ds, mutation);
      const t = ds.types.get(tid)!;
      if (t.category === 18) req.drones.push({ type_id: tid, quantity: n, active: n, mutation });
      else if (t.category === 87) req.fighters.push({ type_id: tid, quantity: n, active: true, abilities: null });
      else req.cargo.push({ type_id: tid, quantity: n });
      continue;
    }
    const comma = line.indexOf(',');
    const name = (comma >= 0 ? line.slice(0, comma) : line).trim();
    const charge = comma >= 0 ? line.slice(comma + 1).trim() : null;
    let tid = ds.typeByName(name);
    if (tid === undefined) throw new Error(`unknown item '${name}'`);
    if (mutation) tid = mutatedType(ds, mutation);
    const t = ds.types.get(tid)!;
    if (t.category === 20) {
      if ('1087' in t.rawAttrs) req.boosters.push({ type_id: tid, side_effects: [] });
      else req.implants.push(tid);
    } else if (t.category === 18) req.drones.push({ type_id: tid, quantity: 1, active: 1, mutation });
    else if (t.category === 8) req.cargo.push({ type_id: tid, quantity: 1 });
    else if (t.group === 1306) req.ship.mode_type_id = tid;
    else {
      const slot = inferSlot(t.effects);
      let chargeId: number | null = null;
      if (charge !== null) {
        const c = ds.typeByName(charge);
        if (c === undefined) throw new Error(`unknown charge '${charge}'`);
        chargeId = c;
      }
      const activeCapable = t.effects.some(([e]) => ds.effects.get(e)?.category === 1) || (t.rawAttrs['6'] ?? 0) !== 0;
      const state = offline ? 'offline' : activeCapable && slot !== 'rig' && slot !== 'subsystem' ? 'active' : 'online';
      modules.push({ type_id: tid, slot, state, charge_type_id: chargeId, mutation, spool: null });
    }
  }
  return req as FitRequest;
}

/** Python float repr (str(float)) for EFT mutation values */
function pyFloat(v: number): string {
  if (!Number.isFinite(v)) return v > 0 ? 'inf' : v < 0 ? '-inf' : 'nan';
  if (Number.isInteger(v) && Math.abs(v) < 1e16) return v.toFixed(1);
  let s = String(v);
  if (s.includes('e')) {
    // JS 1e-7 / 1.5e+21 -> Python 1e-07 / 1.5e+21
    s = s.replace(/e([+-])(\d)$/, 'e$10$2');
  } else if (Math.abs(v) < 1e-4) {
    s = v.toExponential().replace(/e([+-])(\d)$/, 'e$10$2');
  }
  return s;
}

/** Pyfa floatUnerr: round to keep 7 significant digits */
function floatUnerr(v: number): number {
  if (v === 0 || v === Infinity) return v;
  const f = Math.trunc(7 - Math.ceil(Math.log10(Math.abs(v))));
  if (f >= 0) return Number(v.toFixed(Math.min(f, 100)));
  const p = Math.pow(10, -f);
  return Math.round(v / p) * p;
}

// Pyfa exporter orders (service/port/eft.py): drones by market group, fighters by group
const DRONE_ORDER_MG: number[][] = [
  [837, 1531], [3881], [838, 1532], [3882], [359, 839], [3883], [911, 1533], [843, 1586], [841, 1029], [842, 1030], [158, 358], [1643, 1646],
];
const FIGHTER_ORDER = ['Light Fighter', 'Structure Light Fighter', 'Heavy Fighter', 'Structure Heavy Fighter', 'Support Fighter', 'Structure Support Fighter'];

const stableSort = <T>(xs: T[], key: (x: T) => (number | string)[]): T[] =>
  xs.map((x, i) => [key(x), i, x] as const).sort((a, b) => {
    for (let k = 0; k < a[0].length; k++) if (a[0][k] !== b[0][k]) return a[0][k] < b[0][k] ? -1 : 1;
    return a[1] - b[1];
  }).map((t) => t[2]);

/**
 * EFT export, byte-identical to Pyfa's exportEft with all options on, after fit.fill() (contract 1.4.1 ruling 4):
 * racks low/med/high/rig/subsystem/service padded with "[Empty X slot]" up to the ship's (modified) slot counts,
 * then drones+fighters, implants+boosters, cargo and mutation blocks, sections separated by two blank lines.
 * `slotTotals` = modified ship slot counts (from a calc); omitted -> no padding.
 */
export function exportEft(ds: Dataset, req: FitRequest, name: string, slotTotals?: Record<string, number>): string {
  const n = (id: number) => ds.types.get(id)?.name ?? String(id);
  const attrOf = (tid: number, attr: string) => {
    const a = ds.attrId(attr);
    const v = ds.types.get(tid)?.rawAttrs[String(a)];
    return v ?? ds.attrDefault(a);
  };
  const mutants: { base: number; muta: number | null; typeId: number; attrs: Record<string, number> }[] = [];
  const ref = (typeId: number, m: Mutation | null | undefined) => {
    if (!m || m.mutaplasmid_type_id == null) return '';
    mutants.push({ base: m.base_type_id, muta: m.mutaplasmid_type_id, typeId, attrs: m.attributes ?? {} });
    return ` [${mutants.length}]`;
  };
  const isMut = (m: Mutation | null | undefined) => !!m && m.mutaplasmid_type_id != null;
  const sections: string[] = [];
  // 1: modules
  const slotOf = (m: ModuleReq) => m.slot ?? (ds.types.has(m.type_id) ? inferSlot(ds.types.get(m.type_id)!.effects) : null);
  const racks: string[] = [];
  const SLOTS = [['low', 'Low'], ['mid', 'Med'], ['high', 'High'], ['rig', 'Rig'], ['subsystem', 'Subsystem'], ['service', 'Service']] as const;
  for (const [slot, label] of SLOTS) {
    const lines: string[] = [];
    for (const m of (req.modules ?? []).filter((x) => slotOf(x) === slot)) {
      const modName = isMut(m.mutation) ? n(m.mutation!.base_type_id) : n(m.type_id);
      const suffix = ref(m.type_id, m.mutation);
      const off = m.state === 'offline' ? ' /offline' : '';
      lines.push(m.charge_type_id != null ? `${modName}, ${n(m.charge_type_id)}${off}${suffix}` : `${modName}${off}${suffix}`);
    }
    const total = slotTotals ? Math.trunc(slotTotals[slot] ?? 0) : 0;
    for (let k = lines.length; k < total; k++) lines.push(`[Empty ${label} slot]`);
    if (lines.length) racks.push(lines.join('\n'));
  }
  if (racks.length) sections.push(racks.join('\n\n'));
  // 2: drones, fighters
  const minions: string[] = [];
  const mgOrder = (tid: number) => {
    const t = ds.types.get(tid);
    const mg = t?.marketGroup ?? (t?.variationParent != null ? ds.types.get(t.variationParent)?.marketGroup : null) ?? null;
    const k = mg === null ? -1 : DRONE_ORDER_MG.findIndex((l) => l.includes(mg));
    return k < 0 ? DRONE_ORDER_MG.length : k;
  };
  const drones = stableSort(req.drones ?? [], (d) => {
    const item = isMut(d.mutation) ? d.mutation!.base_type_id : d.type_id;
    return [mgOrder(item), isMut(d.mutation) ? 1 : 0, n(d.type_id)];
  });
  const droneLines = drones.map((d) => `${n(isMut(d.mutation) ? d.mutation!.base_type_id : d.type_id)} x${d.quantity ?? 1}${ref(d.type_id, d.mutation)}`);
  if (droneLines.length) minions.push(droneLines.join('\n'));
  const fighters = stableSort(req.fighters ?? [], (f) => {
    const g = ds.groups.get(ds.types.get(f.type_id)?.group ?? -1)?.name ?? '';
    const k = FIGHTER_ORDER.indexOf(g);
    return [k < 0 ? FIGHTER_ORDER.length : k, n(f.type_id)];
  });
  const fighterLines = fighters.map((f) => {
    const maxsq = Math.max(Math.trunc(attrOf(f.type_id, 'fighterSquadronMaxSize')), 1);
    return `${n(f.type_id)} x${Math.min(Math.max(f.quantity ?? maxsq, 1), maxsq)}`;
  });
  if (fighterLines.length) minions.push(fighterLines.join('\n'));
  if (minions.length) sections.push(minions.join('\n\n'));
  // 3: implants, boosters
  const charSec: string[] = [];
  const imps = stableSort(req.implants ?? [], (i) => [attrOf(i, 'implantness')]).map(n);
  if (imps.length) charSec.push(imps.join('\n'));
  const boos = stableSort(req.boosters ?? [], (b) => [attrOf(b.type_id, 'boosterness')]).map((b) => n(b.type_id));
  if (boos.length) charSec.push(boos.join('\n'));
  if (charSec.length) sections.push(charSec.join('\n\n'));
  // 4: cargo
  const cargo = stableSort(req.cargo ?? [], (c) => {
    const t = ds.types.get(c.type_id);
    const g = ds.groups.get(t?.group ?? -1);
    return [ds.categories.get(g?.category ?? -1) ?? '', g?.name ?? '', n(c.type_id)];
  }).map((c) => `${n(c.type_id)} x${c.quantity ?? 1}`);
  if (cargo.length) sections.push(cargo.join('\n'));
  // 5: mutations
  if (mutants.length) {
    sections.push(mutants.map((m, k) => {
      const mu = ds.mutaplasmids.get(m.muta!);
      const base = ds.types.get(m.base)?.rawAttrs ?? {};
      const own = ds.types.get(m.typeId)?.rawAttrs ?? {};
      const kv: [string, number][] = [];
      for (const a of Object.keys(mu?.attrs ?? {})) {
        const bv = base[a] ?? own[a];
        if (bv === undefined) continue;
        let v = m.attrs[a] ?? bv;
        const r = mu!.attrs[a];
        if (r && bv !== 0) {
          const lo = bv * r[0], hi = bv * r[1];
          v = Math.min(Math.max(v, Math.min(lo, hi)), Math.max(lo, hi));
        }
        kv.push([ds.attrs.get(Number(a))?.name ?? a, v]);
      }
      kv.sort((x, y) => (x[0] < y[0] ? -1 : x[0] > y[0] ? 1 : 0));
      return [`[${k + 1}] ${n(m.base)}`, `  ${n(m.muta!)}`, `  ${kv.map(([a, v]) => `${a} ${pyFloat(floatUnerr(v))}`).join(', ')}`].join('\n');
    }).join('\n'));
  }
  return `[${n(req.ship.type_id)}, ${name}]\n\n${sections.join('\n\n\n')}`;
}
