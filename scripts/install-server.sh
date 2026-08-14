#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
image='latex-core-texlive:2026-m7'
expected='sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38'
command -v docker >/dev/null || { echo 'Docker is required.' >&2; exit 1; }
docker compose version >/dev/null || { echo 'Docker Compose v2 is required.' >&2; exit 1; }
if ! docker image inspect "$image" >/dev/null 2>&1; then
  [[ -n "${M7_IMAGE_TAR:-}" && -f "${M7_IMAGE_TAR}" ]] || { echo "Frozen M7 image missing; set M7_IMAGE_TAR to its tar file." >&2; exit 1; }
  docker load --input "$M7_IMAGE_TAR"
fi
actual="$(docker image inspect "$image" --format '{{.Id}}')"
[[ "$actual" == "$expected" ]] || { echo "Expected frozen image $expected, got $actual" >&2; exit 1; }
if [[ ! -f .env ]]; then
  password="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
  umask 077
  printf 'POSTGRES_PASSWORD=%s\nDATABASE_URL=postgresql://latex_core:%s@postgres:5432/latex_core\nPOSTGRES_DB=latex_core\nPOSTGRES_USER=latex_core\nHTTP_PORT=8080\nSESSION_COOKIE_SECURE=false\nALLOW_REGISTRATION=false\nCOMPILER_IMAGE=%s\nTEX_ENVIRONMENT_ID=texlive-2026-sha256-364cea85dc8ba5e2a5131f1d8088142d08c9733d24d2644880d6eae86f751d5a\nWORKER_CONCURRENCY=1\nWORKER_STAGING_HOST_ROOT=/tmp/latex-core-worker-staging\nQUEUE_GLOBAL_RUNNING=2\nQUEUE_PER_USER_RUNNING=1\nQUEUE_PER_USER_OUTSTANDING=8\n' "$password" "$password" "$expected" >.env
  echo 'Created .env with a generated PostgreSQL password.'
else
  echo 'Keeping existing .env unchanged.'
fi
staging="$(awk -F= '$1=="WORKER_STAGING_HOST_ROOT"{print $2}' .env | tail -n1)"; staging="${staging:-/tmp/latex-core-worker-staging}"
mkdir -p "$staging"
docker compose -f deploy/compose/docker-compose.yml up -d --build
ready=false
for _ in $(seq 1 30); do
  if docker compose -f deploy/compose/docker-compose.yml exec -T worker /usr/local/bin/latex-core-doctor; then ready=true; break; fi
  sleep 2
done
if [[ "$ready" != true ]]; then echo 'Services did not become ready.' >&2; exit 1; fi
if [[ -w /usr/local/bin ]]; then
  install -m 755 "$root/latex-core" /usr/local/bin/latex-core
  echo 'Installed latex-core at /usr/local/bin/latex-core.'
else
  echo 'Using repository CLI: ./latex-core (install globally with administrator permissions if desired).'
fi
echo 'LaTeX Core is ready.'
echo 'Create a user: latex-core user create alice@example.com'
echo 'Health: latex-core doctor'
echo 'Backup: latex-core backup /path/to/backup'
