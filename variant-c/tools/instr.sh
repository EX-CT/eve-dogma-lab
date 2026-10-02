#!/usr/bin/env bash
# Deterministic cost per calc (instructions, via callgrind) — robust on a loaded box.
# usage: tools/instr.sh [request.json ...]   (default: rifter, tengu, vexor)
set -e
cd "$(dirname "$0")/.."
B=${B:-bin/eve-dogma-go}; D=${EVE_DOGMA_DATASET:-/workspace/exct-eve/data/dataset-3569502.json.gz}
reqs=("$@"); [ ${#reqs[@]} -eq 0 ] && reqs=(testdata/requests/exct_rifter.json testdata/requests/exct_tengu.json testdata/requests/esf_vexor.json)
# (Go under valgrind occasionally aborts; retry)
ir() { for _ in 1 2 3 4 5; do v=$(GOMAXPROCS=1 valgrind --tool=callgrind --callgrind-out-file=/tmp/instr.$$.cg "$B" --dataset "$D" bench "$1" -n "$2" 2>&1 | sed -n 's/.*Collected : //p'); [ -n "$v" ] && { echo "$v"; return; }; done; echo 0; }
for r in "${reqs[@]}"; do
  a=$(ir "$r" 20); b=$(ir "$r" 120)
  echo "$(basename "$r" .json): $(( (b - a) / 100 )) instr/calc"
done
