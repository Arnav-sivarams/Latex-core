#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
echo 'scripts/server.sh is a compatibility entry point; using ./latex-core.' >&2
exec "$root/latex-core" "$@"
