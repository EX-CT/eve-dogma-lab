/** Node-only helpers (file + gzip + sha256). The core never imports this. */
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { gunzipSync } from 'node:zlib';
import { Dataset } from './core/dataset.js';

export function loadDatasetFile(path: string): Dataset {
  let bytes: Buffer = readFileSync(path);
  if (bytes.length > 2 && bytes[0] === 0x1f && bytes[1] === 0x8b) bytes = gunzipSync(bytes);
  const sha = createHash('sha256').update(bytes).digest('hex');
  return Dataset.fromJson(JSON.parse(bytes.toString('utf8')), sha);
}

export function datasetPath(explicit?: string | null): string {
  return explicit ?? process.env.EVE_DOGMA_DATASET ?? 'dataset.json.gz';
}
