#!/usr/bin/env python3
"""print compact failures of the last G2 scoring run: fails.py [prefix] [max per case]"""
import json, sys
pre = sys.argv[1] if len(sys.argv) > 1 else ""
n = int(sys.argv[2]) if len(sys.argv) > 2 else 4
d = json.load(open('/workspace/exct-eve/g2-bench/results/graphs-G2/failures.json'))
for k, v in d.items():
    if not k.startswith(pre): continue
    if v.get('error'): print(k, 'ERROR', v['error'], v['mismatches'][:1]); continue
    ms = v['mismatches']
    print(k, len(ms), '; '.join(f"{m.get('y')}@{m.get('x')}: got {m.get('got') if not isinstance(m.get('got'), float) else round(m['got'],4)} want {m.get('want') if not isinstance(m.get('want'), float) else round(m['want'],4)}" for m in ms[:n]))
