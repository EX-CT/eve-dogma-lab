/** Node-only helpers (file + gzip + sha256 + VDC4 cache). The core never imports this. */
import { existsSync, mkdirSync, readFileSync, renameSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { homedir } from 'node:os';
import { buildCache, datasetFromCache, isCache } from './core/cache.js';
import { Dataset } from './core/dataset.js';

/** node:crypto is loaded only when a digest is really needed (~13 ms of startup; the cache path never needs it) */
const sha256Hex = (b: Uint8Array): string =>
  ((process as any).getBuiltinModule('node:crypto') as typeof import('node:crypto')).createHash('sha256').update(b).digest('hex');

/** 64-bit FNV-1a of a string as 16 hex digits (cache file key; not security relevant) */
function fnv64(str: string): string {
  let h1 = 0x84222325, h2 = 0xcbf29ce4; // offset basis 0xcbf29ce484222325 (low, high)
  for (let i = 0; i < str.length; i++) {
    h1 ^= str.charCodeAt(i);
    // multiply by the 64-bit FNV prime 0x100000001b3 = 2^40 + 0x1b3
    const lo = h1 * 0x1b3, hi = h2 * 0x1b3 + ((h1 << 8) >>> 0) + Math.floor(lo / 0x100000000);
    h1 = lo >>> 0;
    h2 = hi >>> 0;
  }
  return h2.toString(16).padStart(8, '0') + h1.toString(16).padStart(8, '0');
}

function readJsonBytes(path: string): Buffer {
  let bytes: Buffer = readFileSync(path);
  if (bytes.length > 2 && bytes[0] === 0x1f && bytes[1] === 0x8b) {
    // zlib is loaded only when needed (~20 ms of startup otherwise; the cache path never needs it)
    const zlib = (process as any).getBuiltinModule('node:zlib') as typeof import('node:zlib');
    bytes = zlib.gunzipSync(bytes);
  }
  return bytes;
}

/** package directory: set by the CLI entry points (dist/cli.js, dist-cli bundle); library default ~/.cache/eve-dogma-ts */
let pkgDir: string | null = null;
export function setPackageDir(dir: string): void { pkgDir = dir; }

/** cache location for a dataset file: $EVE_DOGMA_TS_CACHE_DIR, <package>/.cache (CLI) or ~/.cache/eve-dogma-ts, <fnv64(abs path)>-<size>-<mtime>.vdc4 */
export function cachePath(datasetFile: string): string {
  const abs = resolve(datasetFile);
  const st = statSync(abs);
  const key = fnv64(abs);
  const dir = process.env.EVE_DOGMA_TS_CACHE_DIR ?? (pkgDir !== null ? join(pkgDir, '.cache') : join(homedir(), '.cache', 'eve-dogma-ts'));
  return join(dir, `${key}-${st.size}-${Math.trunc(st.mtimeMs)}.vdc4`);
}

/** Load a dataset: a VDC4 cache file, a prebuilt cache for this dataset file if present, else the gz/JSON itself. */
export function loadDatasetFile(path: string, useCache = true): Dataset {
  if (useCache && !process.env.EVE_DOGMA_TS_NO_CACHE) {
    try {
      const cp = cachePath(path);
      if (existsSync(cp)) return datasetFromCache(readFileSync(cp));
    } catch { /* fall back to the full load */ }
  }
  const bytes = readJsonBytes(path);
  if (isCache(bytes)) return datasetFromCache(bytes);
  const sha = sha256Hex(bytes);
  return Dataset.fromJson(JSON.parse(bytes.toString('utf8')), sha);
}

/** Build (or rebuild) the VDC4 cache for a dataset file; returns its path. */
export function writeCache(path: string): string {
  const bytes = readJsonBytes(path);
  const sha = sha256Hex(bytes);
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
