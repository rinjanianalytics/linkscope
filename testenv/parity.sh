#!/bin/bash
# End-to-end parity: run the original MRTG (Perl + C rateup) and linkscope on
# the same config, against the same agent state, at the same (faked) time,
# for several cycles, and require byte-identical .log files.
#
# Needs: faketime, snmpsim, the original MRTG staged in $MRTG_PERL_BIN
# (mrtg script + compiled rateup) and linkscope built at $LINKSCOPE.
set -u
trap '[ -n "${simpid:-}" ] && kill "$simpid" 2>/dev/null' EXIT
here=$(cd "$(dirname "$0")" && pwd)
MRTG_PERL_BIN=${MRTG_PERL_BIN:-/tmp/mrtgperl/bin}
LINKSCOPE=${LINKSCOPE:?path to linkscope binary}
STEPS=${STEPS:-6}
PORT=20163
base=$(mktemp -d /tmp/mrtg-parity.XXXX)
chmod 755 "$base"
mkdir -p "$base/perl" "$base/rust" "$base/data" "$base/cache"
chown nobody:nogroup "$base/cache"
sed "s#@WORK@#$base/perl#" "$here/parity.cfg.in" > "$base/perl.cfg"
sed "s#@WORK@#$base/rust#" "$here/parity.cfg.in" > "$base/rust.cfg"
T0=$(( $(date +%s) / 300 * 300 - 86400 ))
simpid=""
start_sim() {
  [ -n "$simpid" ] && kill "$simpid" 2>/dev/null && wait "$simpid" 2>/dev/null
  rm -rf "$base/cache"/*
  python3 "$here/gen_snmprec.py" "$base/data" --devices 10 --ifaces 8 --static --elapsed "$1"
  snmpsim-command-responder --data-dir="$base/data" --agent-udpv4-endpoint=127.0.0.1:$PORT \
     --cache-dir="$base/cache" --logging-method=null --process-user=nobody --process-group=nogroup >/dev/null 2>&1 &
  simpid=$!
  for _ in $(seq 60); do snmpget -v2c -c dev9 -t 1 -r 0 127.0.0.1:$PORT 1.3.6.1.2.1.1.5.0 >/dev/null 2>&1 && return; sleep 1; done
  echo "simulator did not start"; exit 2
}
fail=0
for k in $(seq 0 $((STEPS-1))); do
  start_sim $((300 * k))
  T=$((T0 + 300 * k))
  ts=$(date -u -d @$T '+%Y-%m-%d %H:%M:%S')
  ( cd "$base" && LANG=C DONT_FAKE_MONOTONIC=1 TZ=UTC faketime -f "$ts" "$MRTG_PERL_BIN/mrtg" "$base/perl.cfg" >"$base/perl.$k.out" 2>&1 )
  ( cd "$base" && DONT_FAKE_MONOTONIC=1 TZ=UTC faketime -f "$ts" "$LINKSCOPE" "$base/rust.cfg" >"$base/rust.$k.out" 2>&1 )
  diffs=0
  for f in "$base"/perl/*.log; do
    n=$(basename "$f")
    if ! cmp -s "$f" "$base/rust/$n"; then
      diffs=$((diffs+1)); fail=1
      echo "step $k: $n differs"; diff <(head -3 "$f") <(head -3 "$base/rust/$n") | head -6
    fi
  done
  echo "step $k (t=$T): $(ls "$base"/perl/*.log | wc -l) logs compared, $diffs differ; line 1 sample: $(head -1 "$base/rust/wrap32.log")"
done
kill "$simpid" 2>/dev/null
echo "work dir: $base"
exit $fail
