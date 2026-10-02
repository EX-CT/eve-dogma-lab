#!/bin/sh
# Deterministic corpus cost: total instructions (callgrind) for `batch --threads 1` over the bench corpus,
# with a warm snapshot cache. usage: tools/ir_corpus.sh  (builds to /tmp/vbprof)
set -e
D=${EVE_DOGMA_DATASET:-/workspace/exct-eve/data/dataset-3569502.json.gz}
CARGO_PROFILE_RELEASE_DEBUG=true cargo build -q --release --target-dir /tmp/vbprof
/tmp/vbprof/release/eve-dogma-vb --dataset $D batch --threads 1 < /tmp/corpus.jsonl > /dev/null
valgrind --tool=callgrind --callgrind-out-file=/tmp/cg.corpus /tmp/vbprof/release/eve-dogma-vb --dataset $D batch --threads 1 < /tmp/corpus.jsonl 2>&1 >/dev/null | grep refs
