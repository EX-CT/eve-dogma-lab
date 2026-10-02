#!/bin/sh
# dev loop: score the local release build against the bench corpus
B=/workspace/exct-eve/lab-e/variant-e/target/release/eve-dogma-e
D=/workspace/exct-eve/data/dataset-3569502.json.gz
cd /workspace/exct-eve/eve-dogma-bench && python3 run.py --name e-dev --cmd "$B calc --dataset $D" --batch-cmd "$B batch --dataset $D" --batch-repeat 1 --latency-n 5 "$@" >/dev/null 2>&1
sed -n 3,7p results/e-dev/scorecard.md
python3 - <<'PY'
import json
f=json.load(open('results/e-dev/failures.json'))
items = f.items() if isinstance(f, dict) else [(x.get('case'), x) for x in f]
for k,v in items:
    print(k, json.dumps(v)[:400])
PY
