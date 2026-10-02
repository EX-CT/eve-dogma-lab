#!/usr/bin/env node
/** eve-dogma-ts CLI — same stateless contract as eve-dogma-rs (docs/contract.md). */
import { readFileSync } from 'node:fs';
import { calc, calcJson, meta, parseEft, rpc, search, typeInfo } from './index.js';
import { datasetPath, loadDatasetFile, setPackageDir, writeCache } from './node.js';
import { dirname, resolve } from 'node:path';
import { realpathSync } from 'node:fs';

// dist/cli.js and dist-cli/eve-dogma-ts.cjs both live one level below the package directory
setPackageDir(resolve(dirname(realpathSync(process.argv[1])), '..'));

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

Dataset: --dataset PATH, or $EVE_DOGMA_DATASET, or ./dataset.json.gz`;

const args = process.argv.slice(2);
function takeFlag(f: string): string | null {
  const p = args.indexOf(f);
  if (p < 0) return null;
  const v = args[p + 1] ?? null;
  args.splice(p, 2);
  return v;
}
const datasetArg = takeFlag('--dataset');
function load() {
  try {
    return loadDatasetFile(datasetPath(datasetArg));
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
const out = (s: string) => process.stdout.write(s + '\n');

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
        if (o.length > 1 << 20) { process.stdout.write(o); o = ''; }
      }
      if (o) process.stdout.write(o);
    };
    process.stdin.setEncoding('utf8');
    process.stdin.on('data', (c: string) => flush(rest + c, false));
    process.stdin.on('end', () => { flush(rest, true); done(); });
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
      const ds = load();
      await lines((l) => calcJson(ds, l));
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
main();
