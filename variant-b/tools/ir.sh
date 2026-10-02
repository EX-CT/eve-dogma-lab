#!/bin/sh
# Deterministic cost metric: instructions (callgrind) spent in Fit::build + compute_stats for N calcs of one case.
# usage: tools/ir.sh CASE.json [N]
set -e
D=${EVE_DOGMA_DATASET:-/workspace/exct-eve/data/dataset-3569502.json.gz}
N=${2:-200}
CARGO_PROFILE_RELEASE_DEBUG=true CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 cargo build -q --release --target-dir /tmp/vbprof
valgrind --tool=callgrind --callgrind-out-file=/tmp/cg.out --toggle-collect='*Fit*build*' --toggle-collect='*compute_stats*' \
  /tmp/vbprof/release/eve-dogma-vb --dataset $D bench "$1" -n $N 2>&1 | grep "refs" | awk -v n=$N '{gsub(",","",$4); printf "%s Ir/calc=%d\n", "'"$(basename $1)"'", $4/(n+1)}'
