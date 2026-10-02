#!/usr/bin/env python3
"""Score variant-d against the eve-dogma-bench expected values via batch mode (fast inner loop).
usage: python3 score_bench.py [BENCH_DIR] [DATASET] [-v]"""
import os, json, pathlib, subprocess, sys, collections
here = pathlib.Path(__file__).resolve().parent
bench = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 and not sys.argv[1].startswith('-') else here / '../../eve-dogma-bench').resolve()
dataset = sys.argv[2] if len(sys.argv) > 2 and not sys.argv[2].startswith('-') else str(here / '../../data/dataset-3569502.json.gz')
sys.path.insert(0, str(bench / 'tools'))
from metrics import METRICS, extract, close  # noqa
cases = []
for p in sorted((bench / 'cases').glob('*.json')):
    e = bench / 'expected' / p.name
    if e.exists():
        cases.append((p.stem, json.dumps(json.loads(p.read_text())), json.loads(e.read_text())))
r = subprocess.run(['node', str(here / os.environ.get('VD_CLI', 'dist/cli.js')), 'batch', '--dataset', dataset], input='\n'.join(c[1] for c in cases) + '\n',
                   capture_output=True, text=True)
outs = r.stdout.splitlines()
assert len(outs) == len(cases), (len(outs), len(cases), r.stderr[-500:])
tot = ok = cok = 0
bad = collections.Counter()
for (name, _, exp), line in zip(cases, outs):
    resp = json.loads(line)
    allok = True
    for k, want in exp['values'].items():
        got = extract(resp, METRICS[k][0])
        tot += 1
        if close(got, want):
            ok += 1
        else:
            allok = False
            bad[k.split('.')[0] if k[0] in 'wdf' and k[1:2].isdigit() else k] += 1
            if '-v' in sys.argv:
                print(f'{name}: {k} got={got} want={want}')
    cok += allok
print(json.dumps({'cases': len(cases), 'cases_ok': cok, 'values': tot, 'values_ok': ok}))
for k, n in bad.most_common(40):
    print(f'  {n:4d} {k}')
