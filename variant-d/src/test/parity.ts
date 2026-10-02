/**
 * Oracle parity: every case in pyfa_expected.json (Pyfa eos run as a black box, see eve-dogma-rs/oracle)
 * must match within max(1e-3, 1e-4·|want|). Usage: node dist/test/parity.js [--root fixtures] [--dataset P] [--verbose]
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { calc, parseEft } from '../index.js';
import { datasetPath, loadDatasetFile } from '../node.js';

const argv = process.argv.slice(2);
const opt = (f: string, d: string | null = null) => { const p = argv.indexOf(f); return p >= 0 ? argv[p + 1] : d; };
const root = opt('--root', 'fixtures')!;
const verbose = argv.includes('--verbose');
const only = opt('--only');
const ds = loadDatasetFile(datasetPath(opt('--dataset')));

/** JSON pointer with optional array selector segments `name[key=value]` (e.g. /offense/weapons[module_index=3]/tracking) */
function pointer(o: any, p: string): any {
  let cur = o;
  for (const seg of p.split('/').slice(1)) {
    if (cur == null) return undefined;
    const b = seg.indexOf('[');
    if (b >= 0 && seg.endsWith(']')) {
      const [k, want] = seg.slice(b + 1, -1).split('=');
      const arr = cur[seg.slice(0, b)];
      cur = Array.isArray(arr) ? arr.find((e) => e != null && JSON.stringify(e[k]) === want) : undefined;
    } else cur = cur[seg];
  }
  return cur;
}
function close(a: any, b: any): boolean {
  if (typeof a === 'number' && typeof b === 'number') return Math.abs(a - b) <= Math.max(1e-3, 1e-4 * Math.abs(b));
  if (typeof a === 'boolean' && typeof b === 'number') return (b !== 0) === a;
  if (typeof b === 'boolean' && typeof a === 'number') return (a !== 0) === b;
  return a === b;
}

const exp = JSON.parse(readFileSync(join(root, 'tests/oracle/pyfa_expected.json'), 'utf8'));
const failures: string[] = [];
let checked = 0, cases = 0;
const failingCases = new Set<string>();
const t0 = performance.now();
for (const [name, f] of Object.entries<any>(exp.fits)) {
  if (only && !name.includes(only)) continue;
  cases++;
  const text = readFileSync(join(root, f.eft), 'utf8');
  let req: any = parseEft(ds, text);
  req.character.skills.default_level = 5;
  if (f.request_patch) req = { ...req, ...f.request_patch };
  const st = calc(ds, req);
  for (const [ptr, want] of Object.entries<any>(f.values)) {
    checked++;
    const got = ptr.includes('+') ? ptr.split('+').reduce((s, p) => s + (Number(pointer(st, p)) || 0), 0) : pointer(st, ptr) ?? null;
    if (!close(got, want)) { failures.push(`${name} ${ptr}: got ${got} want ${want}`); failingCases.add(name); }
  }
  if (f.cap_state_percent !== undefined) {
    checked++;
    const got = pointer(st, '/capacitor/stable_percent') ?? null;
    if (!close(got, f.cap_state_percent)) { failures.push(`${name} cap: got ${got} want ${f.cap_state_percent}`); failingCases.add(name); }
  }
}
const ms = performance.now() - t0;
if (verbose || failures.length < 60) for (const l of failures) console.log(l);
console.log(JSON.stringify({ cases, values: checked, mismatches: failures.length, failing_cases: failingCases.size, ms: Math.round(ms) }));
process.exitCode = failures.length ? 1 : 0;
