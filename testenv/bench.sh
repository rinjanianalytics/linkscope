#!/bin/bash
# Benchmark one polling cycle: original MRTG (sequential and Forks: N) vs
# linkscope, same targets, same agent (snmpsim live counters on :20162).
# Reports wall time, CPU time (user+sys including child processes such as
# rateup) and peak RSS, as the median of RUNS timed cycles after a warm-up.
set -u
MRTG_PERL_BIN=${MRTG_PERL_BIN:-/tmp/mrtgperl/bin}
LINKSCOPE=${LINKSCOPE:?path to linkscope}
DEVICES=${DEVICES:-50}; IFACES=${IFACES:-24}; RUNS=${RUNS:-3}; FORKS=${FORKS:-4}
base=$(mktemp -d /tmp/mrtg-bench.XXXX); chmod 755 "$base"
gen() { # $1 workdir, $2 extra globals
  echo "WorkDir: $1"; printf '%b' "$2"
  for d in $(seq 0 $((DEVICES-1))); do for i in $(seq 1 $IFACES); do
    echo "Target[d${d}_i${i}]: ${i}:dev${d}@127.0.0.1:${PORT:-20162}::::2"
    echo "Title[d${d}_i${i}]: dev${d} Gi0/${i}"
    echo "MaxBytes[d${d}_i${i}]: 125000000"
  done; done
}
for v in perl perlfork rust; do mkdir -p "$base/$v"; done
gen "$base/perl" "" > "$base/perl.cfg"
gen "$base/perlfork" "Forks: $FORKS\n" > "$base/perlfork.cfg"
gen "$base/rust" "" > "$base/rust.cfg"
run() { # label cmd...
  local label=$1; shift
  local name; name=$(echo "$label" | tr -c 'A-Za-z0-9\n' '_')
  "$@" >/dev/null 2>&1  # warm-up (creates logs)
  for r in $(seq $RUNS); do
    /usr/bin/time -f "%e %U %S %M" -o "$base/$name.t$r" "$@" >/dev/null 2>&1
  done
  sort -n "$base"/$name.t* | awk -v n="$label" -v targets=$((DEVICES*IFACES)) '
    {w[NR]=$1; c[NR]=$2+$3; m[NR]=$4} END {i=int((NR+1)/2);
     printf "%-22s targets=%d  wall=%7.2fs  cpu=%7.2fs  peak_rss=%6.1fMB\n", n, targets, w[i], c[i], m[i]/1024}'
}
cd "$base"
run "MRTG (sequential)"   env LANG=C "$MRTG_PERL_BIN/mrtg" "$base/perl.cfg"
run "MRTG (Forks: $FORKS)" env LANG=C "$MRTG_PERL_BIN/mrtg" "$base/perlfork.cfg"
run "linkscope"             "$LINKSCOPE" "$base/rust.cfg"
echo "logs: perl=$(ls $base/perl/*.log | wc -l) rust=$(ls $base/rust/*.log | wc -l)  work: $base"
