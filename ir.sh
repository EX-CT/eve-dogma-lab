#!/bin/bash
valgrind --tool=callgrind --callgrind-out-file=/tmp/cg3.out ${1:-/workspace/exct-eve/lab-h/variant-h/target/release/eve-dogma-h} --dataset /workspace/exct-eve/data/dataset-3569502.json.gz batch < /tmp/c249.jsonl > /dev/null 2>/tmp/cg.err; grep -o "Collected : [0-9]*" /tmp/cg.err
