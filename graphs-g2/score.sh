#!/bin/sh
# score against the graphs corpus: ./score.sh [bench checkout] (default ../../g2-bench)
B=${1:-/workspace/exct-eve/g2-bench}
D=${EVE_DOGMA_DATASET:-/workspace/exct-eve/data/dataset-3569502.json.gz}
cd "$(dirname "$0")" && npx tsc -p . && (cd ../variant-c && go build -trimpath -o bin/eve-dogma-go ./cmd/eve-dogma-go) &&
python3 "$B/graphs/run_graphs.py" --name G2 --batch-cmd "./bin/graph-batch --dataset $D" --cwd "$(pwd)"
