#!/bin/sh
state=${LINKSCOPE_TESTENV:-/tmp/linkscope-testenv}
for f in "$state"/*.pid; do [ -f "$f" ] && kill "$(cat "$f")" 2>/dev/null; rm -f "$f"; done
true
