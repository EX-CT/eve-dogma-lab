// Checks the browser ES bundle gives byte-identical results to the Node build on every bench case.
import { readFileSync, readdirSync } from 'node:fs';
import { gunzipSync } from 'node:zlib';
import { createHash } from 'node:crypto';
const [dataset, casesDir] = process.argv.slice(2);
const B = await import(new URL('../dist-web/eve-dogma-ts.mjs', import.meta.url));
const N = await import(new URL('../dist/index.js', import.meta.url));
const NN = await import(new URL('../dist/node.js', import.meta.url));
const bytes = gunzipSync(readFileSync(dataset));
const sha = createHash('sha256').update(bytes).digest('hex');
const dsB = B.Dataset.fromJson(JSON.parse(bytes.toString('utf8')), sha);
const dsN = NN.loadDatasetFile(dataset);
let ok = 0, bad = 0;
for (const f of readdirSync(casesDir).filter((x) => x.endsWith('.json'))) {
  const q = readFileSync(`${casesDir}/${f}`, 'utf8');
  if (B.calcJson(dsB, q) === N.calcJson(dsN, q)) ok++; else { bad++; console.log('DIFF', f); }
}
console.log(JSON.stringify({ identical: ok, different: bad }));
process.exit(bad ? 1 : 0);
