// Graph primitives (schema eve-dogma-graph-primitives/1) as emitted by `eve-dogma-go graph-primitives`.
export type Attrs = Record<string, number>;

export interface StackInputs {
  base: number;
  value: number;
  mods: { op: number; value: number; penalized: boolean }[];
  high_is_good: boolean;
  max?: number;
  min?: number;
}

export interface ChargePrim {
  type_id: number;
  name: string;
  group?: string;
  group_id: number;
  category_id: number;
  attrs: Attrs;
  effects: string[];
}

export interface ItemPrim {
  kind: "module" | "drone" | "fighter";
  index: number | null;
  type_id: number;
  name: string;
  group?: string;
  group_id: number;
  category_id: number;
  state: "offline" | "online" | "active" | "overheated";
  effects: string[];
  attrs: Attrs;
  effect_ranges?: Record<string, { range?: number; falloff?: number; tracking?: number; category: number; offensive: boolean; assistance: boolean; resistance_attr?: string }>;
  // modules
  weapon_kind?: string;
  volley?: [number, number, number, number]; // em, thermal, kinetic, explosive (one cycle, unspooled)
  cycle?: { raw_ms: number; reactivation_ms: number; reload_ms: number; shots: number; charges: number; avg_ms: number; avg_reload_ms: number };
  charge?: ChargePrim;
  spool?: { type: string; amount: number };
  // drones / fighters
  quantity?: number;
  active?: number;
  abilities?: string[];
}

export interface CapDrain {
  duration_ms: number;
  cap_need: number;
  clip_size: number;
  reload_ms: number;
  is_injector: boolean;
  disable_stagger: boolean;
  projected?: boolean;
}

export interface FitPrim {
  ship: { type_id: number; name: string; group?: string; group_id: number; attrs: Attrs; effects: string[]; character: Attrs; stack: { maxVelocity: StackInputs; signatureRadius: StackInputs } };
  items: ItemPrim[];
  cap_drains: CapDrain[];
  stats: any;
}

export interface Primitives {
  schema: string;
  engine: string;
  source: FitPrim;
  target?: { normal: FitPrim; scrammed?: FitPrim };
  subwarp_speed?: number;
  charges?: any;
}

export interface GraphRequest {
  graph: string;
  fit: any;
  target?: { profile?: any; fit?: any; resist_mode?: string } | null;
  x: { axis: string; values: number[] };
  y: string[];
  params?: Record<string, any>;
  settings?: Record<string, any>;
}

export interface GraphResult {
  graph: string;
  x_axis: string;
  x: number[];
  series: Record<string, (number | null)[]>;
  meta?: Record<string, any>;
}

export class GraphError extends Error {
  constructor(public code: string, message: string, public path = "") {
    super(message);
  }
}
