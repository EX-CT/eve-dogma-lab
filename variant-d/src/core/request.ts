/** FitRequest v1 (see EX-CT/eve-dogma-rs docs/contract.md). Normalised with the same defaults as the Rust serde model. */

export type StateName = 'offline' | 'online' | 'active' | 'overheated';
export type SlotName = 'high' | 'mid' | 'low' | 'rig' | 'subsystem' | 'service';
export type SpoolKind = 'spool_scale' | 'cycle_scale' | 'time' | 'cycles';

export const enum State { Offline = 0, Online = 1, Active = 2, Overheated = 3 }
export const STATE_NAMES: StateName[] = ['offline', 'online', 'active', 'overheated'];
export const SLOT_NAMES: SlotName[] = ['high', 'mid', 'low', 'rig', 'subsystem', 'service'];

export interface Spool { type: SpoolKind; amount: number }
export interface Mutation { base_type_id: number; mutaplasmid_type_id?: number | null; attributes?: Record<string, number> }
export interface ModuleReq {
  type_id: number; slot?: SlotName | null; state?: StateName | null; charge_type_id?: number | null;
  mutation?: Mutation | null; spool?: Spool | null;
}
export interface DroneReq { type_id: number; quantity?: number; active?: number | null; mutation?: Mutation | null }
export interface FighterReq { type_id: number; quantity?: number | null; active?: boolean; abilities?: number[] | null }
export interface BoosterReq { type_id: number; side_effects?: number[] }
export interface CargoReq { type_id: number; quantity?: number }
export interface Skills { default_level?: number | null; levels?: Record<string, number> }
export interface Character { skills?: Skills; security_status?: number | null }
export interface Buff { buff_id: number; value: number }
export interface Fleet { buffs?: Buff[]; booster_fits?: FitRequest[] }
export interface Projected {
  kind: string; module?: ModuleReq | null; drone?: DroneReq | null; fighter?: FighterReq | null; fit?: FitRequest | null; amount?: number; distance_m?: number | null;
}
export interface Environment { effect_type_ids?: number[]; system_security?: string | null }
export interface Resists { em: number; thermal: number; kinetic: number; explosive: number }
export interface TargetProfile extends Partial<Resists> { signature_radius?: number | null; max_velocity?: number | null; radius?: number | null }
export interface Override { type_id: number; attribute_id: number; value: number }
export interface CapSimOpts { reload?: boolean; stagger?: boolean; max_time_s?: number | null }
export interface Options {
  nos_no_target_cap?: boolean; factor_reload?: boolean; default_spool?: Spool | null; rah?: string | null;
  include_attributes?: string | null; sources?: boolean; validate?: boolean; cap_sim?: CapSimOpts;
}
export interface FitRequest {
  schema_version?: number | null;
  ship: { type_id: number; mode_type_id?: number | null };
  character?: Character; modules?: ModuleReq[]; drones?: DroneReq[]; fighters?: FighterReq[]; implants?: number[];
  boosters?: BoosterReq[]; cargo?: CargoReq[]; fleet?: Fleet; projected?: Projected[]; environment?: Environment;
  damage_pattern?: Resists | null; target_profile?: TargetProfile | null; overrides?: Override[]; options?: Options;
}

/** Fully-defaulted request used internally. */
export interface NormRequest {
  ship: { type_id: number; mode_type_id: number | null };
  character: { skills: { default_level: number | null; levels: Record<string, number> }; security_status: number | null };
  modules: ModuleReq[]; drones: DroneReq[]; fighters: FighterReq[]; implants: number[]; boosters: BoosterReq[]; cargo: CargoReq[];
  fleet: { buffs: Buff[]; booster_fits: FitRequest[] }; projected: Projected[];
  environment: { effect_type_ids: number[]; system_security: string | null };
  damage_pattern: Resists | null; target_profile: TargetProfile | null; overrides: Override[];
  options: Required<Omit<Options, 'cap_sim'>> & { cap_sim: Required<CapSimOpts> };
}

export class RequestError extends Error {
  constructor(message: string, public path = '') { super(message); }
}

const isNum = (v: unknown) => typeof v === 'number' && Number.isFinite(v);
function uint(v: unknown, path: string): number {
  if (!isNum(v) || (v as number) < 0 || !Number.isInteger(v)) throw new RequestError(`${path}: expected unsigned integer`, path);
  return v as number;
}
function enumOf<T extends string>(v: unknown, allowed: readonly T[], path: string): T | null {
  if (v === undefined || v === null) return null;
  if (!allowed.includes(v as T)) throw new RequestError(`${path}: unknown variant \`${String(v)}\`, expected one of ${allowed.join(', ')}`, path);
  return v as T;
}
const arr = <T>(v: T[] | undefined | null): T[] => (Array.isArray(v) ? v : []);

/** Validate + default a raw request object (mirrors serde defaults of eve-dogma-rs request.rs). */
export function normalize(raw: any): NormRequest {
  if (!raw || typeof raw !== 'object') throw new RequestError('request must be an object');
  if (!raw.ship || typeof raw.ship !== 'object') throw new RequestError('missing field `ship`', '/ship');
  const ship = { type_id: uint(raw.ship.type_id, '/ship/type_id'), mode_type_id: raw.ship.mode_type_id ?? null };
  const ch = raw.character ?? {};
  const sk = ch.skills ?? {};
  const modules: ModuleReq[] = arr<any>(raw.modules).map((m, i) => {
    const p = `/modules/${i}`;
    return {
      type_id: uint(m.type_id, `${p}/type_id`), slot: enumOf(m.slot, SLOT_NAMES, `${p}/slot`), state: enumOf(m.state, STATE_NAMES, `${p}/state`),
      charge_type_id: m.charge_type_id ?? null, mutation: m.mutation ?? null, spool: m.spool ?? null,
    };
  });
  const o = raw.options ?? {};
  const cs = o.cap_sim ?? {};
  return {
    ship,
    character: { skills: { default_level: sk.default_level ?? null, levels: sk.levels ?? {} }, security_status: ch.security_status ?? null },
    modules,
    drones: arr<any>(raw.drones).map((d, i) => ({ type_id: uint(d.type_id, `/drones/${i}/type_id`), quantity: d.quantity ?? 1, active: d.active ?? null, mutation: d.mutation ?? null })),
    fighters: arr<any>(raw.fighters).map((f, i) => ({ type_id: uint(f.type_id, `/fighters/${i}/type_id`), quantity: f.quantity ?? null, active: f.active ?? true, abilities: f.abilities ?? null })),
    implants: arr<number>(raw.implants),
    boosters: arr<any>(raw.boosters).map((b) => ({ type_id: b.type_id, side_effects: b.side_effects ?? [] })),
    cargo: arr<any>(raw.cargo).map((c) => ({ type_id: c.type_id, quantity: c.quantity ?? 1 })),
    fleet: { buffs: arr(raw.fleet?.buffs), booster_fits: arr(raw.fleet?.booster_fits) },
    projected: arr<any>(raw.projected).map((p) => ({ ...p, amount: p.amount ?? 1, distance_m: p.distance_m ?? null })),
    environment: { effect_type_ids: arr(raw.environment?.effect_type_ids), system_security: raw.environment?.system_security ?? null },
    damage_pattern: raw.damage_pattern
      ? { em: raw.damage_pattern.em ?? 0, thermal: raw.damage_pattern.thermal ?? 0, kinetic: raw.damage_pattern.kinetic ?? 0, explosive: raw.damage_pattern.explosive ?? 0 }
      : null,
    target_profile: raw.target_profile ?? null,
    overrides: arr(raw.overrides),
    options: {
      nos_no_target_cap: !!o.nos_no_target_cap, factor_reload: !!o.factor_reload, default_spool: o.default_spool ?? null,
      rah: o.rah ?? null, include_attributes: o.include_attributes ?? null, sources: !!o.sources, validate: o.validate ?? true,
      cap_sim: { reload: !!cs.reload, stagger: !!cs.stagger, max_time_s: cs.max_time_s ?? null },
    },
  };
}
