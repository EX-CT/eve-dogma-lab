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

export function exportEft(ds: Dataset, req: FitRequest, name: string): string {
  const n = (id: number) => ds.types.get(id)?.name ?? String(id);
  let out = `[${n(req.ship.type_id)}, ${name}]\n`;
  const muts: Mutation[] = [];
  const tag = (m: Mutation | null | undefined) => (m ? (muts.push(m), ` [${muts.length}]`) : '');
  const slotOf = (m: ModuleReq) => m.slot ?? (ds.types.has(m.type_id) ? inferSlot(ds.types.get(m.type_id)!.effects) : null);
  for (const slot of ['low', 'mid', 'high', 'rig', 'subsystem', 'service'] as const) {
    let any = false;
    for (const m of (req.modules ?? []).filter((m) => slotOf(m) === slot)) {
      any = true;
      out += m.mutation ? n(m.mutation.base_type_id) : n(m.type_id);
      if (m.charge_type_id != null) out += `, ${n(m.charge_type_id)}`;
      if (m.state === 'offline') out += ' /OFFLINE';
      out += tag(m.mutation) + '\n';
    }
    if (any) out += '\n';
  }
  for (const d of req.drones ?? []) out += `${n(d.mutation?.base_type_id ?? d.type_id)} x${d.quantity ?? 1}${tag(d.mutation)}\n`;
  for (const f of req.fighters ?? []) out += `${n(f.type_id)} x${f.quantity ?? 1}\n`;
  if ((req.implants ?? []).length || (req.boosters ?? []).length) {
    out += '\n';
    for (const i of req.implants ?? []) out += `${n(i)}\n`;
    for (const b of req.boosters ?? []) out += `${n(b.type_id)}\n`;
  }
  if ((req.cargo ?? []).length) {
    out += '\n';
    for (const c of req.cargo ?? []) out += `${n(c.type_id)} x${c.quantity ?? 1}\n`;
  }
  if (muts.length) {
    out += '\n';
    muts.forEach((m, k) => {
      out += `[${k + 1}] ${n(m.base_type_id)}\n`;
      if (m.mutaplasmid_type_id != null) out += `  ${n(m.mutaplasmid_type_id)}\n`;
      const kv = Object.keys(m.attributes ?? {}).sort().map((a) => `${ds.attrs.get(Number(a))?.name ?? a} ${m.attributes![a]}`);
      if (kv.length) out += `  ${kv.join(', ')}\n`;
    });
  }
  return out;
}
