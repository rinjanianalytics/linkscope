#!/usr/bin/env bash
# Bring up the gNMI lab: two SR Linux routers with traffic through both.
# Safe to run again: secrets and certificates are kept, the configuration is reapplied.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
cd "$here"
gen=$here/generated
gnmic_image=ghcr.io/openconfig/gnmic:0.49.0
declare -A mgmt_ip=([srl1]=10.20.0.11 [srl2]=10.20.0.12)
routers=(srl1 srl2)

# Random letters and digits, safe in a CLI string and a URL.
random_word() { openssl rand -base64 48 | tr -dc 'A-Za-z0-9' | head -c "$1"; }

# Secrets: the gNMI user's password and the SNMP community.
mkdir -p "$gen"
chmod 700 "$gen"
umask 077
[[ -s $gen/linkscope.pass ]] || random_word 24 > "$gen/linkscope.pass"
[[ -s $gen/snmp.community ]] || random_word 20 > "$gen/snmp.community"

# A lab CA, and a server certificate per router (SAN: its name and mgmt IP).
if [[ ! -s $gen/ca.pem ]]; then
  openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=linkscope-gnmi lab CA" \
    -keyout "$gen/ca.key" -out "$gen/ca.pem" 2>/dev/null
fi
for r in "${routers[@]}"; do
  [[ -s $gen/$r.pem ]] && continue
  openssl req -newkey rsa:2048 -nodes -subj "/CN=$r" \
    -keyout "$gen/$r.key" -out "$gen/$r.csr" 2>/dev/null
  printf 'subjectAltName=DNS:%s,IP:%s\nextendedKeyUsage=serverAuth\nkeyUsage=digitalSignature,keyEncipherment\n' \
    "$r" "${mgmt_ip[$r]}" > "$gen/$r.ext"
  openssl x509 -req -in "$gen/$r.csr" -CA "$gen/ca.pem" -CAkey "$gen/ca.key" -CAcreateserial \
    -days 825 -extfile "$gen/$r.ext" -out "$gen/$r.pem" 2>/dev/null
  rm -f "$gen/$r.csr" "$gen/$r.ext"
done
# The CA certificate is public; let other users (and containers) read it.
chmod 644 "$gen/ca.pem"

start=$(date +%s)
docker compose up -d

# Wait until the management server has loaded its initial configuration
# (the same check containerlab makes).
for r in "${routers[@]}"; do
  echo -n "waiting for $r to boot "
  for _ in $(seq 1 150); do
    if docker compose exec -T "$r" cat /etc/opt/srlinux/devices/app_ephemeral.mgmt_server.ready_for_config \
        2>/dev/null | grep -q 'loaded initial configuration'; then
      break
    fi
    echo -n .
    sleep 2
  done
  echo
done

# The generated part of a router's configuration: TLS, gNMI server, user, SNMP.
secrets_cfg() {
  local r=$1
  cat <<EOF
set / system tls profile linkscope key "$(cat "$gen/$r.key")"
set / system tls profile linkscope certificate "$(cat "$gen/$r.pem")"
set / system tls profile linkscope authenticate-client false

set / system grpc-server mgmt admin-state enable
set / system grpc-server mgmt network-instance mgmt
set / system grpc-server mgmt port 57400
set / system grpc-server mgmt services [ gnmi gnsi ]
set / system grpc-server mgmt tls-profile linkscope
delete / system grpc-server mgmt default-tls-profile
# The SR Linux default, written out: 60 RPCs a minute per router, all clients together.
set / system grpc-server mgmt rate-limit 60

set / system configuration role linkscope-ro rule / action read
set / system aaa authorization role linkscope-ro services [ gnmi ]
set / system aaa authentication user linkscope password "$(cat "$gen/linkscope.pass")"
set / system aaa authentication user linkscope role [ linkscope-ro ]

set / system snmp access-group linkscope security-level no-auth-no-priv community-entry linkscope community "$(cat "$gen/snmp.community")"
set / system snmp network-instance mgmt admin-state enable
EOF
}

# Apply the router's file and the generated part in a private candidate, commit and save.
for r in "${routers[@]}"; do
  out=$( { cat "$r.cfg"; secrets_cfg "$r"; echo "commit save"; } |
    docker compose exec -T "$r" sr_cli -d --pre-command "enter candidate private" 2>&1 ) || true
  if ! grep -q 'All changes have been committed' <<<"$out"; then
    echo "$r: configuration failed:" >&2
    grep -v -E '^\s*[-+ ]' <<<"$out" >&2 || true
    exit 1
  fi
  echo "$r: configuration committed and saved"
done

# Wait until both routers answer gNMI over TLS with the lab CA and the linkscope user.
for r in "${routers[@]}"; do
  echo -n "waiting for gNMI on $r "
  ok=
  for _ in $(seq 1 60); do
    if docker run --rm --network linkscope-gnmi_mgmt -v "$gen/ca.pem:/ca.pem:ro" \
        -e GNMIC_PASSWORD="$(cat "$gen/linkscope.pass")" "$gnmic_image" \
        -a "${mgmt_ip[$r]}:57400" --tls-ca /ca.pem -u linkscope --timeout 5s \
        capabilities >/dev/null 2>&1; then
      ok=1
      break
    fi
    echo -n .
    sleep 2
  done
  echo
  [[ -n $ok ]] || { echo "$r does not answer gNMI" >&2; exit 1; }
done

echo "lab ready in $(( $(date +%s) - start ))s"
echo "  srl1 ${mgmt_ip[srl1]}  gNMI :57400 (OpenConfig and native), SNMP v2c :161"
echo "  srl2 ${mgmt_ip[srl2]}  gNMI :57400 (native only), SNMP v2c :161"
echo "  user linkscope, password in $gen/linkscope.pass"
echo "  SNMP community in $gen/snmp.community, CA in $gen/ca.pem"
