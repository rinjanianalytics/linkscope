#!/bin/sh
# Start the linkscope test agents on 127.0.0.1:
#   20171  net-snmp snmpd (community "public", v3 users from snmpd.conf)
#   20161  snmpsim, static counters, communities dev0..devN-1 (parity tests)
#   20162  snmpsim, advancing counters (benchmarks)
set -e
here=$(cd "$(dirname "$0")" && pwd)
state=${LINKSCOPE_TESTENV:-/tmp/linkscope-testenv}
devices=${DEVICES:-50}
ifaces=${IFACES:-24}
mkdir -p "$state/snmpd" "$state/cache-static" "$state/cache-live"
chown nobody:nogroup "$state/cache-static" "$state/cache-live"
python3 "$here/gen_snmprec.py" "$state/static" --devices "$devices" --ifaces "$ifaces" --static
python3 "$here/gen_snmprec.py" "$state/live" --devices "$devices" --ifaces "$ifaces"
SNMP_PERSISTENT_DIR="$state/snmpd" snmpd -C -c "$here/snmpd.conf" -Lf "$state/snmpd.log" -p "$state/snmpd.pid"
for kind in static:20161 live:20162; do
  name=${kind%%:*}; port=${kind##*:}
  snmpsim-command-responder --data-dir="$state/$name" --agent-udpv4-endpoint=127.0.0.1:$port \
    --cache-dir="$state/cache-$name" --logging-method=null --process-user=nobody --process-group=nogroup \
    > "$state/sim-$name.log" 2>&1 &
  echo $! > "$state/sim-$name.pid"
done
echo "test agents starting (state in $state)"
