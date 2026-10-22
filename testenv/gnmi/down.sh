#!/usr/bin/env bash
# Remove the gNMI lab: containers and networks. Images and generated/ are kept.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
docker compose --project-directory "$here" -f "$here/compose.yaml" down --remove-orphans
