#!/bin/sh
# score against the graphs corpus: [G2_MODE=batch|rpc|single] ./score.sh [bench checkout] (default ../../g2-bench)
B=${1:-/workspace/exct-eve/g2-bench}
D=${EVE_DOGMA_DATASET:-/workspace/exct-eve/data/dataset-3569502.json.gz}
cd "$(dirname "$0")" && npx tsc -p . && (cd ../variant-c && go build -trimpath -o bin/eve-dogma-go ./cmd/eve-dogma-go) || exit 1
case "${G2_MODE:-batch}" in
  rpc) exec python3 "$B/graphs/run_graphs.py" --name G2-rpc --rpc-cmd "./bin/serve-stdio --dataset $D" --cwd "$(pwd)" ;;
  single) exec python3 "$B/graphs/run_graphs.py" --name G2-single --cmd "./bin/graph --dataset $D" --cwd "$(pwd)" ;;
  *) exec python3 "$B/graphs/run_graphs.py" --name G2 --batch-cmd "./bin/graph-batch --dataset $D" --cwd "$(pwd)" ;;
esac
