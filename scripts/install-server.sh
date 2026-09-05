#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"

echo 'scripts/install-server.sh is a compatibility entry point; using the supported root installer.' >&2
exec "$root/install.sh" "$@"
