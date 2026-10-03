/**
 * Target resolution (modifier func x domain -> items) through indexes built once per fit.
 * Lists are in ascending item order (same as a linear scan).
 */
import { Domain, Func } from './dataset.js';
import { AttrGraph, Kind, Loc } from './graph.js';

export class TargetIndex {
  shipLoc: number[] = [];
  charLoc: number[] = [];
  shipByGroup = new Map<number, number[]>();
  charByGroup = new Map<number, number[]>();
  /** skill -> items located on the ship that require it */
  skillShip = new Map<number, number[]>();
  /** skill -> owned items that require it */
  skillOwned = new Map<number, number[]>();
  /** skill -> (owned or char-located) non-skill items that require it */
  skillChar = new Map<number, number[]>();

  /** append a char-located item created after the index (skills); keeps ascending order */
  addCharItem(it: { idx: number; group: number }): void {
    this.charLoc.push(it.idx);
    const l = this.charByGroup.get(it.group);
    if (l) l.push(it.idx);
    else this.charByGroup.set(it.group, [it.idx]);
  }

  constructor(g: AttrGraph) {
    const push = (m: Map<number, number[]>, k: number, i: number) => {
      const l = m.get(k);
      if (l) l.push(i);
      else m.set(k, [i]);
    };
    const items = g.items;
    for (let n = 0; n < items.length; n++) {
      const it = items[n];
      const i = it.idx;
      if (it.loc === Loc.Ship) {
        this.shipLoc.push(i);
        push(this.shipByGroup, it.group, i);
      } else if (it.loc === Loc.Char) {
        this.charLoc.push(i);
        push(this.charByGroup, it.group, i);
      }
      const rs = it.reqSkills;
      for (let k = 0; k < rs.length; k++) {
        const s = rs[k];
        if (it.loc === Loc.Ship) push(this.skillShip, s, i);
        if (it.owned) push(this.skillOwned, s, i);
        if ((it.owned || it.loc === Loc.Char) && it.kind !== Kind.Skill) push(this.skillChar, s, i);
      }
    }
  }
}

const NONE: readonly number[] = [];
/** scratch one-element result (callers consume a result before resolving again) */
const ONE: number[] = [0];
const one = (i: number): readonly number[] => { ONE[0] = i; return ONE; };

export function resolveTargets(
  g: AttrGraph, idx: TargetIndex, src: number, func: Func, domain: Domain, extra: number,
  ship: number, char: number, isStructure: boolean,
): readonly number[] {
  const s = g.items[src];
  switch (domain) {
    case Domain.Item:
      return func === Func.Item ? one(src) : NONE;
    case Domain.Other:
      if (s.charge >= 0) return one(s.charge);
      if (s.parent >= 0) return one(s.parent);
      return NONE;
    case Domain.Structure:
      if (!isStructure) return NONE;
    // falls through
    case Domain.Ship:
      switch (func) {
        case Func.Item: return one(ship);
        case Func.Location: return idx.shipLoc;
        case Func.LocationGroup: return idx.shipByGroup.get(extra) ?? NONE;
        case Func.LocationRequiredSkill: return idx.skillShip.get(extra) ?? NONE;
        case Func.OwnerRequiredSkill: return idx.skillOwned.get(extra) ?? NONE;
        default: return NONE;
      }
    case Domain.Char:
      switch (func) {
        case Func.Item: return one(char);
        case Func.Location: return idx.charLoc;
        case Func.LocationGroup: return idx.charByGroup.get(extra) ?? NONE;
        case Func.LocationRequiredSkill:
        case Func.OwnerRequiredSkill: return idx.skillChar.get(extra) ?? NONE;
        default: return NONE;
      }
    default:
      return NONE;
  }
}
