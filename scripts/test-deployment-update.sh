#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
test_root="$(mktemp -d /tmp/latex-core-update-test.XXXXXX)"
cleanup() { rm -rf -- "$test_root"; }
trap cleanup EXIT

mkdir -p "$test_root/repo/scripts" "$test_root/repo/deploy/compose" "$test_root/repo/migrations" "$test_root/fake-bin"
cp "$root/.env.example" "$test_root/repo/.env.example"
cp "$root/scripts/generate-local-env.sh" "$test_root/repo/scripts/"
cp "$root/scripts/install-common.sh" "$test_root/repo/scripts/"
cp "$root/scripts/validate-install-config.py" "$test_root/repo/scripts/"
cp "$root/scripts/check-deployment-migrations.sh" "$test_root/repo/scripts/"
cp "$root/scripts/check-install-ports.sh" "$test_root/repo/scripts/"
cp "$root/scripts/update-deployment.sh" "$test_root/repo/scripts/"
cp "$root/deploy/compose/docker-compose.yml" "$test_root/repo/deploy/compose/"
cp "$root/migrations/0001_initial_schema.sql" "$test_root/repo/migrations/"

COMPOSE_PROJECT_NAME=latex-core-update-test \
HTTP_PORT=19000 LATEX_CORE_POSTGRES_PORT=19001 \
WORKER_STAGING_HOST_ROOT="$test_root/latex-core-staging" \
  "$test_root/repo/scripts/generate-local-env.sh" --output "$test_root/repo/.env" >/dev/null

export FAKE_DOCKER_LOG="$test_root/docker-calls.log"
cat >"$test_root/fake-bin/docker" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$FAKE_DOCKER_LOG"
if [[ "${1:-}" == compose ]]; then
  case " $* " in
    *' config --services '*)
      printf 'postgres\napi\nworker\n'
      [[ "${FAKE_DOCKER_MODE:-}" == missing-service ]] || printf 'caddy\n'
      ;;
    *' ps -aq postgres '*) printf 'postgres-id\n' ;;
    *' ps -aq api '*) printf 'api-id\n' ;;
    *' ps -aq worker '*) printf 'worker-id\n' ;;
    *' exec -T postgres '*) printf '%s\n' "$FAKE_MIGRATION_LINE" ;;
    *' build '*)
      [[ "${FAKE_DOCKER_MODE:-}" != build-fail ]] || exit 42
      ;;
  esac
elif [[ "${1:-}" == inspect ]]; then
  case "$*" in
    *Health*) printf 'healthy\n' ;;
    *Mounts*) printf 'latex-core-update-test_latex_core_blob_data\n' ;;
    *) printf 'running\n' ;;
  esac
fi
EOF
chmod +x "$test_root/fake-bin/docker"

cat >"$test_root/fake-bin/ss" <<'EOF'
#!/usr/bin/env bash
printf 'LISTEN 0 128 127.0.0.1:19000 0.0.0.0:*\n'
EOF
chmod +x "$test_root/fake-bin/ss"

export PATH="$test_root/fake-bin:$PATH"
checksum="$(sha384sum "$test_root/repo/migrations/0001_initial_schema.sql")"
export FAKE_MIGRATION_LINE="1|${checksum%% *}|t"

if "$test_root/repo/scripts/update-deployment.sh" invalid >"$test_root/out" 2>"$test_root/error"; then
  echo 'invalid service selection unexpectedly passed' >&2
  exit 1
fi
grep -q 'Unsupported update service: invalid' "$test_root/error"

: >"$FAKE_DOCKER_LOG"
if FAKE_DOCKER_MODE=missing-service "$test_root/repo/scripts/update-deployment.sh" api >"$test_root/out" 2>"$test_root/error"; then
  echo 'missing Compose service unexpectedly passed' >&2
  exit 1
fi
grep -q 'missing required service: caddy' "$test_root/error"
if grep -Eq '(^| )build( |$)|(^| )up( |$)' "$FAKE_DOCKER_LOG"; then
  echo 'missing-service preflight attempted a build or replacement' >&2
  exit 1
fi

: >"$FAKE_DOCKER_LOG"
if FAKE_DOCKER_MODE=build-fail "$test_root/repo/scripts/update-deployment.sh" api >"$test_root/out" 2>"$test_root/error"; then
  echo 'failed image build unexpectedly passed' >&2
  exit 1
fi
grep -q 'Update failed during phase: image-build (exit 42)' "$test_root/error"
grep -q 'No application container replacement was requested.' "$test_root/error"
grep -Eq '(^| )build api($| )' "$FAKE_DOCKER_LOG"
if grep -Eq '(^| )up( |$)' "$FAKE_DOCKER_LOG"; then
  echo 'build failure attempted a container replacement' >&2
  exit 1
fi

if "$test_root/repo/scripts/check-install-ports.sh" >"$test_root/out" 2>"$test_root/error"; then
  echo 'occupied host port unexpectedly passed' >&2
  exit 1
fi
grep -q 'HTTP port 19000 is already occupied by a host process; no service was stopped.' "$test_root/error"

echo 'Deployment update failure-path tests passed.'
