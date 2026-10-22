#!/usr/bin/env bash
set -euo pipefail

# Never use the development/staging database for browser acceptance.
root="$(cd "$(dirname "$0")/.." && pwd)"
source "$root/scripts/install-common.sh"
latex_core_select_docker "$root"
cd "$root"
export LATEX_CORE_COURSE_TEXLIVE_IMAGE="${LATEX_CORE_COURSE_TEXLIVE_IMAGE:-sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38}"
latex_core_docker image inspect "$LATEX_CORE_COURSE_TEXLIVE_IMAGE" >/dev/null
container="latex-core-origin-test-$$"
created=false
cleanup() {
  if [[ "$created" == true ]]; then latex_core_docker rm -f "$container" >/dev/null; fi
}
trap cleanup EXIT
latex_core_docker run -d --name "$container" \
  --label latex-core.purpose=disposable-origin-tests \
  -e POSTGRES_USER=origin_audit -e POSTGRES_PASSWORD=synthetic-test-only \
  -p 127.0.0.1::5432 --tmpfs /var/lib/postgresql postgres:18.4 >/dev/null
created=true
ready=false
for attempt in {1..30}; do
  if latex_core_docker exec "$container" pg_isready -h 127.0.0.1 -U origin_audit >/dev/null 2>&1; then
    ready=true
    break
  fi
  sleep 1
done
[[ "$ready" == true ]] || { echo 'Disposable PostgreSQL did not become ready.' >&2; exit 1; }
port="$(latex_core_docker port "$container" 5432)"
port="${port##*:}"
export REVIEW_PORT="${REVIEW_PORT:-9000}"
for target in ${REVIEW_TARGETS:-chromium:localhost chromium:127.0.0.1 chromium:network firefox:network}; do
  browser="${target%%:*}"
  origin="${target#*:}"
  database="origin_${browser}_${origin//./_}"
  latex_core_docker exec "$container" createdb -U origin_audit "$database"
  export TEST_DATABASE_URL="postgresql://origin_audit:synthetic-test-only@127.0.0.1:$port/$database"
  echo "Browser acceptance: $browser $origin (fresh synthetic database)"
  REVIEW_BROWSER="$browser" REVIEW_HOST="$origin" cargo test -p server --features database-tests \
    review_regressions_browser_acceptance -- --nocapture
done
