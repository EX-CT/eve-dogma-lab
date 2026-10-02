/** In-process timing over a directory of FitRequest JSON files. node dist/bench/bench.js --cases DIR [-n N] [--dataset P] */
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { calc } from '../index.js';
import { datasetPath, loadDatasetFile } from '../node.js';

const argv = process.argv.slice(2);
const opt = (f: string, d: string | null = null) => { const p = argv.indexOf(f); return p >= 0 ? argv[p + 1] : d; };
const dir = opt('--cases', '../../eve-dogma-bench/cases')!;
const n = Number(opt('-n', '20'));
const t0 = performance.now();
const ds = loadDatasetFile(datasetPath(opt('--dataset')));
const loadMs = performance.now() - t0;
const reqs = readdirSync(dir).filter((f) => f.endsWith('.json')).sort().map((f) => [f, JSON.parse(readFileSync(join(dir, f), 'utf8'))] as const);
const tw = performance.now();
for (const [, r] of reqs) calc(ds, r);
const warm = performance.now() - tw;
const per: [string, number][] = [];
const t1 = performance.now();
for (const [f, r] of reqs) {
  const s = performance.now();
  for (let i = 0; i < n; i++) calc(ds, r);
  per.push([f, (performance.now() - s) / n]);
}
const total = performance.now() - t1;
per.sort((a, b) => b[1] - a[1]);
const med = [...per].sort((a, b) => a[1] - b[1])[per.length >> 1][1];
console.log(JSON.stringify({ dataset_load_ms: loadMs, cases: reqs.length, first_pass_ms: warm, mean_ms: total / (n * reqs.length), median_ms: med, slowest: per.slice(0, 5) }, null, 1));
