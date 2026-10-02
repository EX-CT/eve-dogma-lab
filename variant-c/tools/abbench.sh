#!/bin/sh
# A/B micro benchmark: committed HEAD vs working tree, interleaved runs, min of N (the box is shared/noisy).
# usage: tools/abbench.sh [bench regex] [rounds]
set -e
B=${1:-AllCasesJSON}; N=${2:-5}
cd "$(dirname "$0")/.."
go test -c -o /tmp/ab_new.test ./dogma
git stash -q && trap 'git stash pop -q' EXIT
go test -c -o /tmp/ab_old.test ./dogma
git stash pop -q && trap - EXIT
cd dogma
for i in $(seq $N); do
  for v in old new; do
    GOMAXPROCS=1 /tmp/ab_$v.test -test.run x -test.bench "$B\$" -test.benchtime 20x | awk -v v=$v '/us\/fit/ {for(i=1;i<=NF;i++) if($(i+1)=="us/fit") print v, $i}'
  done
done | awk '{ if (!($1 in m) || $2 < m[$1]) m[$1]=$2 } END { printf "min us/fit  old %s  new %s  (%.1f%%)\n", m["old"], m["new"], (m["new"]/m["old"]-1)*100 }'
