#!/bin/bash
# usage: run_bench.sh NAME [extra run.py args]
D=/workspace/exct-eve/data/dataset-3569502.json.gz
B=/workspace/exct-eve/lab-h/variant-h/target/release/eve-dogma-h
N=${1:-H-dev}; shift
cd /workspace/exct-eve/eve-dogma-bench && python3 run.py --name $N --cmd "$B calc --dataset $D" --batch-cmd "$B batch --dataset $D" "$@" 2>&1 | sed -n '3,6p'
