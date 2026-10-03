#!/bin/sh
# Output-identity gate used for the perf work: runs a stress corpus through a base commit and the working tree
# and requires byte-identical graph-batch output (cached and --no-cache).
# usage: bench/identity_check.sh DATASET CASES_DIR [BASE_COMMIT=f7aa4cb]
set -e
D=$1; C=$2; BASE=${3:-f7aa4cb}
HERE=$(cd "$(dirname "$0")/.." && pwd); W=$(mktemp -d)
git -C "$HERE" worktree add -q --detach "$W/base" "$BASE"
python3 "$HERE/bench/make_stress.py" "$C" "$W"
for s in stress stress2 stress3; do
  "$W/base/graphs-g3/bin/eve-dogma-g3" graph-batch --dataset "$D" < "$W/$s.jsonl" > "$W/$s.ref"
  "$HERE/bin/eve-dogma-g3" graph-batch --dataset "$D" < "$W/$s.jsonl" | cmp - "$W/$s.ref"
  EVE_DOGMA_G3_NO_CACHE=1 "$HERE/bin/eve-dogma-g3" graph-batch --dataset "$D" < "$W/$s.jsonl" | cmp - "$W/$s.ref"
  echo "$s: $(wc -l < "$W/$s.jsonl") requests identical to $BASE (cache + no-cache)"
done
git -C "$HERE" worktree remove --force "$W/base"; rm -rf "$W"
