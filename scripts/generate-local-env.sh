#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
force=false
target=''
usage() {
  cat <<'EOF'
Usage: ./scripts/generate-local-env.sh [--output ABSOLUTE_PATH] [--force]

Create .env from .env.example with cryptographically random local secrets.
Existing .env files are preserved unless --force is supplied.
EOF
}
while (($#)); do
  case "$1" in
    --force) force=true; shift ;;
    --output) target="${2:-}"; shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done
target="${target:-$root/.env}"
[[ "$target" == /* ]] || { echo 'Output path must be absolute.' >&2; exit 2; }
example="$root/.env.example"
[[ -f "$example" ]] || { echo '.env.example is missing.' >&2; exit 1; }
if [[ -e "$target" && "$force" != true ]]; then
  echo '.env already exists; keeping it unchanged.'
  exit 0
fi
random_hex() {
  if command -v openssl >/dev/null 2>&1; then openssl rand -hex 32
  elif command -v python3 >/dev/null 2>&1; then python3 -c 'import secrets; print(secrets.token_hex(32))'
  else echo 'OpenSSL or Python 3 is required to generate local secrets.' >&2; return 1; fi
}
random_base64_32() {
  if command -v openssl >/dev/null 2>&1; then openssl rand -base64 32 | tr -d '\n'
  elif command -v python3 >/dev/null 2>&1; then python3 -c 'import base64,secrets; print(base64.b64encode(secrets.token_bytes(32)).decode())'
  else echo 'OpenSSL or Python 3 is required to generate local secrets.' >&2; return 1; fi
}
postgres_password="$(random_hex)"
mail_key="$(random_base64_32)"
project_name="${COMPOSE_PROJECT_NAME:-latex-core}"
staging_default="$root/.runtime/${project_name}-worker-staging"
target_directory="$(dirname "$target")"
[[ -d "$target_directory" ]] || { echo 'Output directory does not exist.' >&2; exit 1; }
temporary="$(mktemp "$target_directory/.latex-core-env.tmp.XXXXXX")"
trap 'rm -f "$temporary"' EXIT
umask 077
while IFS= read -r line || [[ -n "$line" ]]; do
  case "$line" in
    POSTGRES_PASSWORD=*) printf 'POSTGRES_PASSWORD=%s\n' "$postgres_password" ;;
    DATABASE_URL=*) printf 'DATABASE_URL=postgresql://latex_core:%s@postgres:5432/latex_core\n' "$postgres_password" ;;
    COMPOSE_PROJECT_NAME=*) printf 'COMPOSE_PROJECT_NAME=%s\n' "$project_name" ;;
    HTTP_PORT=*) printf 'HTTP_PORT=%s\n' "${HTTP_PORT:-8080}" ;;
    HTTP_BIND_ADDRESS=*) printf 'HTTP_BIND_ADDRESS=%s\n' "${HTTP_BIND_ADDRESS:-127.0.0.1}" ;;
    LATEX_CORE_POSTGRES_PORT=*) printf 'LATEX_CORE_POSTGRES_PORT=%s\n' "${LATEX_CORE_POSTGRES_PORT:-54329}" ;;
    WORKER_STAGING_HOST_ROOT=*) printf 'WORKER_STAGING_HOST_ROOT=%s\n' "${WORKER_STAGING_HOST_ROOT:-$staging_default}" ;;
    LATEX_CORE_PUBLIC_BASE_URL=*) printf 'LATEX_CORE_PUBLIC_BASE_URL=%s\n' "${LATEX_CORE_PUBLIC_BASE_URL:-http://localhost:${HTTP_PORT:-8080}}" ;;
    LATEX_CORE_MAIL_SECRET_KEY=*) printf 'LATEX_CORE_MAIL_SECRET_KEY=%s\n' "$mail_key" ;;
    *) printf '%s\n' "$line" ;;
  esac
done <"$example" >"$temporary"
chmod 600 "$temporary" 2>/dev/null || true
mv -f "$temporary" "$target"
trap - EXIT
chmod 600 "$target" 2>/dev/null || true
printf 'Created %s with generated local secrets (mode 600 where supported).\n' "$target"
