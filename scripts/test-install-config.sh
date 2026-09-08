#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
test_root="$(mktemp -d /tmp/latex-core-install-config-test.XXXXXX)"
cleanup() { rm -rf -- "$test_root"; }
trap cleanup EXIT

export COMPOSE_PROJECT_NAME=latex-core-config-test
export HTTP_PORT=18081
export LATEX_CORE_POSTGRES_PORT=15439
"$root/scripts/generate-local-env.sh" --output "$test_root/valid.env" >/dev/null
"$root/scripts/validate-install-config.py" "$test_root/valid.env" >/dev/null
grep -qx 'HTTP_PORT=18081' "$test_root/valid.env"
grep -qx 'LATEX_CORE_POSTGRES_PORT=15439' "$test_root/valid.env"

sed 's/^ALLOW_REGISTRATION=false$/ALLOW_REGISTRATION=invalid/' "$test_root/valid.env" >"$test_root/invalid-bool.env"
chmod 600 "$test_root/invalid-bool.env"
if "$root/scripts/validate-install-config.py" "$test_root/invalid-bool.env" >"$test_root/out" 2>"$test_root/error"; then
  echo 'invalid boolean unexpectedly passed' >&2
  exit 1
fi
grep -q 'ALLOW_REGISTRATION must be true or false' "$test_root/error"

if HTTP_PORT=19999 "$root/scripts/validate-install-config.py" "$test_root/valid.env" >"$test_root/out" 2>"$test_root/error"; then
  echo 'conflicting export unexpectedly passed' >&2
  exit 1
fi
grep -q 'exported configuration conflicts with .env: HTTP_PORT' "$test_root/error"
if grep -qE '18081|19999' "$test_root/error"; then
  echo 'conflict report exposed configuration values' >&2
  exit 1
fi

sed 's|^LATEX_CORE_PUBLIC_BASE_URL=.*$|LATEX_CORE_PUBLIC_BASE_URL="http://localhost:18081"|' "$test_root/valid.env" >"$test_root/quoted.env"
chmod 600 "$test_root/quoted.env"
"$root/scripts/validate-install-config.py" "$test_root/quoted.env" >/dev/null

# A deployed main .env predates these feature-branch settings. Their runtime
# defaults must remain compatible so an update does not require reinstalling or
# replacing credentials/data.
sed -E '/^(COMPOSE_PROJECT_NAME|HTTP_BIND_ADDRESS|LATEX_CORE_POSTGRES_PORT|SESSION_TTL_SECONDS|QUEUE_LEASE_SECONDS|QUEUE_MAX_ATTEMPTS|LATEX_CORE_BACKUP_|LATEX_CORE_MAIL_|LATEX_CORE_SMTP_|LATEX_CORE_PUBLIC_BASE_URL)=/d' \
  "$test_root/valid.env" >"$test_root/legacy-main.env"
chmod 600 "$test_root/legacy-main.env"
env -u COMPOSE_PROJECT_NAME -u LATEX_CORE_POSTGRES_PORT \
  "$root/scripts/validate-install-config.py" "$test_root/legacy-main.env" >/dev/null

unset COMPOSE_PROJECT_NAME HTTP_PORT LATEX_CORE_POSTGRES_PORT
"$root/scripts/generate-local-env.sh" --output "$test_root/defaults.env" >/dev/null
grep -qx 'HTTP_PORT=9000' "$test_root/defaults.env"
grep -qx 'LATEX_CORE_POSTGRES_PORT=9001' "$test_root/defaults.env"
grep -qx 'HTTP_BIND_ADDRESS=127.0.0.1' "$test_root/defaults.env"
grep -qx 'LATEX_CORE_PUBLIC_BASE_URL=http://localhost:9000' "$test_root/defaults.env"

echo 'Installer configuration tests passed.'
