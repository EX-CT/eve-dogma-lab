/** Node-only helpers (file + gzip + sha256 + VDC3 cache). The core never imports this. */
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, renameSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildCache, datasetFromCache, isCache } from './core/cache.js';
import { Dataset } from './core/dataset.js';

function readJsonBytes(path: string): Buffer {
  let bytes: Buffer = readFileSync(path);
  if (bytes.length > 2 && bytes[0] === 0x1f && bytes[1] === 0x8b) {
    // zlib is loaded only when needed (~20 ms of startup otherwise; the cache path never needs it)
    const zlib = (process as any).getBuiltinModule('node:zlib') as typeof import('node:zlib');
    bytes = zlib.gunzipSync(bytes);
  }
  return bytes;
}

/** cache location for a dataset file: <package>/.cache/<sha1(abs path)>-<size>-<mtime>.vdc */
export function cachePath(datasetFile: string): string {
  const abs = resolve(datasetFile);
  const st = statSync(abs);
  const key = createHash('sha1').update(abs).digest('hex').slice(0, 16);
  const pkg = resolve(dirname(fileURLToPath(import.meta.url)), '..');
  return join(pkg, '.cache', `${key}-${st.size}-${Math.trunc(st.mtimeMs)}.vdc3`);
}

/** Load a dataset: a VDC3 cache file, a prebuilt cache for this dataset file if present, else the gz/JSON itself. */
export function loadDatasetFile(path: string, useCache = true): Dataset {
  if (useCache && !process.env.EVE_DOGMA_TS_NO_CACHE) {
    try {
      const cp = cachePath(path);
      if (existsSync(cp)) return datasetFromCache(readFileSync(cp));
    } catch { /* fall back to the full load */ }
  }
  const bytes = readJsonBytes(path);
  if (isCache(bytes)) return datasetFromCache(bytes);
  const sha = createHash('sha256').update(bytes).digest('hex');
  return Dataset.fromJson(JSON.parse(bytes.toString('utf8')), sha);
}

/** Build (or rebuild) the VDC3 cache for a dataset file; returns its path. */
export function writeCache(path: string): string {
  const bytes = readJsonBytes(path);
  const sha = createHash('sha256').update(bytes).digest('hex');
  const text = buildCache(JSON.parse(bytes.toString('utf8')), sha);
  const cp = cachePath(path);
  mkdirSync(dirname(cp), { recursive: true });
  const tmp = `${cp}.${process.pid}.tmp`;
  writeFileSync(tmp, text);
  renameSync(tmp, cp); // atomic replace
  return cp;
}

export function datasetPath(explicit?: string | null): string {
  return explicit ?? process.env.EVE_DOGMA_DATASET ?? 'dataset.json.gz';
}
