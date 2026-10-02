#!/usr/bin/env node
/** eve-dogma-ts CLI — same stateless contract as eve-dogma-rs (docs/contract.md). */
import { readFileSync, realpathSync, statSync, writeSync } from 'node:fs';
import { calc, calcJson, meta, parseEft, rpc, search, typeInfo } from './index.js';
import { cachePath, datasetPath, loadDatasetFile, setPackageDir, writeCache } from './node.js';
import { attachCacheBytes, datasetFromCache, detachCacheBytes } from './core/cache.js';
import { dirname, join, resolve } from 'node:path';
import type { Dataset } from './core/dataset.js';

const USAGE = `eve-dogma-ts <command> [--dataset PATH] [args]

Commands:
  calc [FILE]            FitRequest JSON (file or stdin) -> FitStats JSON
  batch                  JSONL FitRequests on stdin -> JSONL FitStats on stdout
  serve-stdio            JSONL RPC: {"id":..,"method":"calc|eft_parse|eft_export|search|type|meta","params":..}
  eft [FILE]             EFT text -> FitRequest JSON (add --calc to compute, --skills N)
  search QUERY           search types by name
  type ID|NAME           show type with base attributes
  meta                   dataset info
  bench [FILE] [-n N]    time N calculations of a request
  cache                  build the fast cold-start cache for the dataset (.cache/)
  snapshot [-o BLOB]     build a V8 startup snapshot with the dataset preloaded (dist-cli/eve-dogma-ts.blob);
                         run it as: node --snapshot-blob dist-cli/eve-dogma-ts.blob <command> [--dataset PATH] ...

Dataset: --dataset PATH, or $EVE_DOGMA_DATASET, or ./dataset.json.gz`;

/** sample request (bench case exct_rifter) used to warm the code cache in `cache` */
const WARMUP_REQUEST = '{"character":{"skills":{"default_level":5}},"drones":[{"active":2,"quantity":2,"type_id":2488}],"modules":[{"slot":"low","state":"active","type_id":2048},{"slot":"low","state":"active","type_id":519},{"slot":"low","state":"active","type_id":20347},{"slot":"mid","state":"active","type_id":438},{"slot":"mid","state":"active","type_id":448},{"slot":"mid","state":"active","type_id":527},{"charge_type_id":21898,"slot":"high","state":"active","type_id":2889},{"charge_type_id":21898,"slot":"high","state":"active","type_id":2889},{"charge_type_id":21898,"slot":"high","state":"active","type_id":2889},{"charge_type_id":24473,"slot":"high","state":"active","type_id":10631},{"slot":"rig","state":"online","type_id":31674},{"slot":"rig","state":"online","type_id":31686},{"slot":"rig","state":"online","type_id":31015}],"ship":{"type_id":587}}';

let args: string[] = [];
/** script a batch worker thread runs (the launcher, also when this process started from the snapshot) */
let workerScript = '';
function takeFlag(f: string): string | null {
  const p = args.indexOf(f);
  if (p < 0) return null;
  const v = args[p + 1] ?? null;
  args.splice(p, 2);
  return v;
}
let datasetArg: string | null = null;

/**
 * Dataset already in the heap of a startup snapshot (`snapshot` command), without its cache body bytes (read from
 * the cache file at run time; a smaller heap deserialises faster). Used only when the requested dataset file and
 * its cache file are the very files it was built from (path, size, mtime, cache head bytes); else loads normally.
 */
let preloaded: { key: string; cache: string; cacheKey: string; sig: { size: number; head: number[] }; ds: Dataset } | null = null;
const fileKey = (p: string): string => {
  const abs = resolve(p);
  const st = statSync(abs);
  return `${abs}\0${st.size}\0${Math.trunc(st.mtimeMs)}`;
};
function load() {
  try {
    const p = datasetPath(datasetArg);
    if (preloaded !== null && !process.env.EVE_DOGMA_TS_NO_CACHE) {
      try {
        const pr = preloaded;
        preloaded = null;
        if (fileKey(p) === pr.key && fileKey(pr.cache) === pr.cacheKey && attachCacheBytes(pr.ds, readFileSync(pr.cache), pr.sig)) return pr.ds;
      } catch { /* fall through to the normal load */ }
    }
    return loadDatasetFile(p);
  } catch (e) {
    process.stderr.write(`error: ${(e as Error).message}\n`);
    process.exit(3);
  }
}
function readInput(f?: string): string {
  if (f && f !== '-') {
    try { return readFileSync(f, 'utf8'); } catch (e) { process.stderr.write(`error: ${f}: ${(e as Error).message}\n`); process.exit(2); }
  }
  return readFileSync(0, 'utf8');
}
/**
 * stdout through fs.writeSync on fd 1: no stream machinery is loaded (~6 ms of startup) and output stays ordered.
 * EAGAIN (a non-blocking pipe that is full) waits 1 ms and retries.
 */
let napBuf: Int32Array | null = null;
function writeAll(s: string): void {
  const b = Buffer.from(s, 'utf8');
  let o = 0;
  while (o < b.length) {
    try {
      o += writeSync(1, b, o, b.length - o);
    } catch (e) {
      if ((e as NodeJS.ErrnoException).code !== 'EAGAIN') throw e;
      Atomics.wait((napBuf ??= new Int32Array(new SharedArrayBuffer(4))), 0, 0, 1);
    }
  }
}
const out = (s: string) => writeAll(s + '\n');

/**
 * JSONL loop: answers every complete line of each stdin chunk, then writes the answers of that chunk in one
 * write (no readline/async-iterator overhead; an interactive client still gets each answer as soon as its line arrives).
 */
function lines(fn: (l: string) => string): Promise<void> {
  return new Promise((done) => {
    let rest = '';
    const flush = (text: string, final: boolean) => {
      const parts = text.split('\n');
      rest = final ? '' : parts.pop()!;
      let o = '';
      for (const raw of parts) {
        const l = raw.endsWith('\r') ? raw.slice(0, -1) : raw;
        if (l.trim()) o += fn(l) + '\n';
        if (o.length > 1 << 20) { writeAll(o); o = ''; }
      }
      if (o) writeAll(o);
    };
    process.stdin.setEncoding('utf8');
    process.stdin.on('data', (c: string) => flush(rest + c, false));
    process.stdin.on('end', () => { flush(rest, true); done(); });
  });
}

const builtin = (name: string): unknown => (process as any).getBuiltinModule(name);

/**
 * `batch`: answers JSONL in input order. Small inputs are computed on the main thread only (no thread start-up);
 * once a backlog builds up, worker threads (each its own dataset load from the cache) take units of lines as they
 * become idle while the main thread keeps computing too. Every answer is the same pure calcJson, so the output is
 * byte-identical to a serial run. Threads: --threads N / $EVE_DOGMA_TS_THREADS (0 = cores, max 8; default 1 = serial).
 */
function batchParallel(ds: ReturnType<typeof load>, threadsWanted: number): Promise<void> {
  const UNIT = 8, START_BACKLOG = 48;
  const os = builtin('node:os') as typeof import('node:os');
  const threads = Math.max(1, Math.min(threadsWanted > 0 ? threadsWanted : Math.min(os.availableParallelism(), 8), 32));
  return new Promise((done) => {
    let rest = '', ended = false;
    const queue: string[] = [];
    let qHead = 0, nextSeq = 0, nextOut = 0, inFlight = 0, scheduled = false;
    const results = new Map<number, string>();
    type W = { w: import('node:worker_threads').Worker; idle: boolean; units: number; readyMs: number };
    let mainUnits = 0;
    const t0 = performance.now();
    let pool: W[] | null = null;
    const take = (): { seq: number; lines: string[] } | null => {
      if (qHead >= queue.length) return null;
      const lines = queue.slice(qHead, qHead + UNIT);
      qHead += lines.length;
      if (qHead > 4096) { queue.splice(0, qHead); qHead = 0; }
      return { seq: nextSeq++, lines };
    };
    const flushOut = () => {
      let o = '';
      for (let r = results.get(nextOut); r !== undefined; r = results.get(nextOut)) {
        results.delete(nextOut++);
        o += r;
        if (o.length > 1 << 20) { writeAll(o); o = ''; }
      }
      if (o) writeAll(o);
    };
    const finish = () => {
      if (!ended || inFlight > 0 || qHead < queue.length) return;
      flushOut();
      if (pool) for (const p of pool) p.w.terminate();
      if (process.env.VD_BATCH_DEBUG) process.stderr.write(`batch: main ${mainUnits} units, workers ${pool ? pool.map((p) => `${p.units}@${p.readyMs.toFixed(0)}ms`).join(' ') : '-'} t=${(performance.now() - t0).toFixed(0)}ms\n`);
      done();
    };
    const feed = (p: W) => {
      const u = take();
      if (!u) { p.idle = true; return; }
      p.idle = false;
      inFlight++;
      p.w.postMessage(u);
    };
    const startPool = () => {
      const { Worker } = builtin('node:worker_threads') as typeof import('node:worker_threads');
      const wargs = ['__batch-worker'];
      if (datasetArg !== null) wargs.push('--dataset', datasetArg);
      pool = [];
      for (let i = 1; i < threads; i++) {
        const p: W = { w: new Worker(workerScript, { argv: wargs, stdout: false, stderr: false }), idle: false, units: 0, readyMs: 0 };
        p.w.on('message', (m: { seq: number; out: string }) => {
          if (m.seq < 0) p.readyMs = performance.now() - t0;
          else p.units++;
          if (m.seq >= 0) { inFlight--; results.set(m.seq, m.out); flushOut(); }
          feed(p);
          pump();
          finish();
        });
        pool.push(p);
      }
    };
    // main thread: one unit per macrotask, so worker messages are handled in between
    const step = () => {
      scheduled = false;
      const u = take();
      if (u) {
        let o = '';
        for (const l of u.lines) o += calcJson(ds, l) + '\n';
        results.set(u.seq, o);
        mainUnits++;
        flushOut();
        if (pool) for (const p of pool) if (p.idle) feed(p);
      }
      pump();
      finish();
    };
    const pump = () => {
      if (!pool && threads > 1 && queue.length - qHead >= START_BACKLOG) startPool();
      if (!scheduled && qHead < queue.length) { scheduled = true; setImmediate(step); }
    };
    const add = (text: string, final: boolean) => {
      const parts = text.split('\n');
      rest = final ? '' : parts.pop()!;
      for (const raw of parts) {
        const l = raw.endsWith('\r') ? raw.slice(0, -1) : raw;
        if (l.trim()) queue.push(l);
      }
    };
    process.stdin.setEncoding('utf8');
    process.stdin.on('data', (c: string) => { add(rest + c, false); pump(); });
    process.stdin.on('end', () => { add(rest, true); ended = true; pump(); finish(); });
  });
}

async function main() {
  const cmd = args[0] ?? '';
  switch (cmd) {
    case 'calc': {
      const ds = load();
      const r = calcJson(ds, readInput(args[1]));
      out(r);
      if (r.startsWith('{"error"')) process.exitCode = 2;
      break;
    }
    case 'batch': {
      const threads = Number(takeFlag('--threads') ?? process.env.EVE_DOGMA_TS_THREADS ?? 1);
      const ds = load();
      await batchParallel(ds, threads);
      break;
    }
    case '__batch-worker': {
      // worker thread of `batch`: {seq, lines[]} in -> {seq, out} back (same calcJson as the main thread)
      const { parentPort } = builtin('node:worker_threads') as typeof import('node:worker_threads');
      const ds = load();
      parentPort!.on('message', (m: { seq: number; lines: string[] }) => {
        let o = '';
        for (const l of m.lines) o += calcJson(ds, l) + '\n';
        parentPort!.postMessage({ seq: m.seq, out: o });
      });
      parentPort!.postMessage({ seq: -1, out: '' });
      break;
    }
    case 'serve-stdio': {
      const ds = load();
      process.stderr.write(`eve-dogma-ts serve-stdio ready (sde ${ds.build})\n`);
      await lines((l) => JSON.stringify(rpc(ds, l)));
      break;
    }
    case 'eft': {
      const skills = takeFlag('--skills');
      const doCalc = args.includes('--calc');
      const rest = args.filter((a) => a !== '--calc');
      const ds = load();
      try {
        const r = parseEft(ds, readInput(rest[1]));
        if (skills !== null) r.character!.skills!.default_level = Number(skills);
        out(JSON.stringify(doCalc ? calc(ds, r) : r, null, 2));
      } catch (e) {
        process.stderr.write(`error: ${(e as Error).message}\n`);
        process.exit(2);
      }
      break;
    }
    case 'cache': {
      // precompute the fast-start cache for the dataset (pure re-layout of the same data)
      out(writeCache(datasetPath(datasetArg)));
      // launcher bundle: run one sample calc on the cached dataset, then save V8's code cache for the engine
      const cc = (globalThis as any).__eveDogmaCodeCache as (() => string | null) | undefined;
      if (cc) {
        const ds = load();
        for (let k = 0; k < 3; k++) calcJson(ds, WARMUP_REQUEST);
        const f = cc();
        if (f) out(f);
      }
      break;
    }
    case 'snapshot': {
      // V8 startup snapshot of the CLI with this dataset preloaded: `node --snapshot-blob dist-cli/eve-dogma-ts.blob <command> ...`
      // starts without module compilation or dataset load (the blob is specific to this node binary; rebuild after upgrades)
      const { execFileSync } = builtin('node:child_process') as typeof import('node:child_process');
      const blob = resolve(takeFlag('-o') ?? join(dirname(workerScript), 'eve-dogma-ts.blob'));
      const entry = join(dirname(workerScript), 'eve-dogma-ts.snapshot.cjs');
      const tmp = `${blob}.${process.pid}.tmp`;
      execFileSync(process.execPath, ['--snapshot-blob', tmp, '--build-snapshot', entry, resolve(datasetPath(datasetArg))], { stdio: 'inherit' });
      (builtin('node:fs') as typeof import('node:fs')).renameSync(tmp, blob);
      out(blob);
      break;
    }
    case 'search': out(JSON.stringify(search(load(), args.slice(1).join(' '), 25), null, 2)); break;
    case 'type': out(JSON.stringify(typeInfo(load(), args.slice(1).join(' ')), null, 2)); break;
    case 'meta': {
      const t0 = performance.now();
      const ds = load();
      out(JSON.stringify({ ...meta(ds), load_ms: performance.now() - t0 }, null, 2));
      break;
    }
    case 'bench': {
      const n = Number(takeFlag('-n') ?? 1000);
      const t0 = performance.now();
      const ds = load();
      const loadMs = performance.now() - t0;
      const req = JSON.parse(readInput(args[1]));
      calc(ds, req);
      const t1 = performance.now();
      for (let i = 0; i < n; i++) calc(ds, req);
      const el = (performance.now() - t1) / 1000;
      out(JSON.stringify({ dataset_load_ms: loadMs, iterations: n, total_s: el, per_calc_us: (el / n) * 1e6 }));
      break;
    }
    default:
      process.stderr.write(USAGE + '\n');
      process.exit(cmd === '' || cmd === 'help' || cmd === '--help' ? 0 : 2);
  }
}
/** run the CLI: argv without node and script, package directory */
function run(argv: string[], pkgDir: string): Promise<void> {
  args = argv.slice();
  if (!workerScript) workerScript = join(pkgDir, 'dist-cli', 'eve-dogma-ts.cjs');
  setPackageDir(pkgDir);
  datasetArg = takeFlag('--dataset');
  return main();
}

const G = globalThis as any;
if (G.__eveDogmaSnapshot) {
  // startup-snapshot builder (dist-cli/eve-dogma-ts.snapshot.cjs): preload + warm, then run per process from the snapshot
  G.__eveDogmaCli = {
    preload(datasetFile: string, pkgDir: string) {
      setPackageDir(pkgDir);
      const cache = cachePath(datasetFile);
      const ds = datasetFromCache(readFileSync(cache));
      for (let k = 0; k < 3; k++) calcJson(ds, WARMUP_REQUEST);
      const sig = detachCacheBytes(ds)!;
      preloaded = { key: fileKey(datasetFile), cache, cacheKey: fileKey(cache), sig, ds };
    },
    run,
  };
} else {
  // dist/cli.js and dist-cli/eve-dogma-ts.cjs both live one level below the package directory
  const script = realpathSync(process.argv[1]);
  workerScript = script;
  run(process.argv.slice(2), resolve(dirname(script), '..'));
}
