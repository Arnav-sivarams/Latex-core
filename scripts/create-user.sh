#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
email="${1:?usage: create-user.sh EMAIL [PASSWORD]}"
args=(create --email "$email")
if [[ $# -gt 1 ]]; then args+=(--password "$2"); fi
exec "$root/latex-core" user "${args[@]}"
