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

echo 'Installer configuration tests passed.'
