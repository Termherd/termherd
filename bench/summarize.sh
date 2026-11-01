#!/usr/bin/env bash
# Medians per tool, scenario and metric.
cd "${BENCH_RESULTS:-$(dirname "$0")/results}" || exit 1
for d in *-rep[0-9]*; do
  [ -f "$d/metrics.txt" ] || continue
  t=${d%-rep*}
  awk -v t="$t" '/^S[0-9] /{for(i=2;i<=NF;i++){split($i,a,"="); print t, $1, a[1], a[2]}}' "$d/metrics.txt"
done | sort -k1,1 -k2,2 -k3,3 -k4,4g \
  | awk '{k=$1" "$2" "$3; v[k]=v[k]" "$4; n[k]++}
         END{for(k in v){split(substr(v[k],2),a," "); m=a[int((n[k]+1)/2)];
             printf "%-34s median=%-9s runs=%s\n", k, m, substr(v[k],2)}}' | sort
