#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
test_root="$(mktemp -d /tmp/latex-core-cli-test.XXXXXX)"
cleanup() { rm -rf -- "$test_root"; }
trap cleanup EXIT
mkdir -p "$test_root/repo/scripts"
cp "$root/latex-core" "$root/install.sh" "$test_root/repo/"
cp "$root/scripts/install-common.sh" "$root/scripts/docker-exec.sh" "$test_root/repo/scripts/"

cd "$test_root"
"$test_root/repo/latex-core" --help | grep -q 'install|start|stop|restart|status'
"$test_root/repo/latex-core" install --help | grep -q -- '--admin-password-stdin'
if "$test_root/repo/latex-core" status >"$test_root/out" 2>"$test_root/err"; then
  echo 'status unexpectedly succeeded without installation' >&2
  exit 1
fi
grep -Fq "$test_root/repo/.env" "$test_root/err"
grep -Fq './latex-core install' "$test_root/err"
if "$test_root/repo/latex-core" install --admin-email uat@example.test >"$test_root/out" 2>"$test_root/err"; then
  echo 'incomplete admin flags unexpectedly succeeded' >&2
  exit 1
fi
grep -q -- '--admin-email requires --admin-password-stdin' "$test_root/err"
echo 'Fresh-shell CLI root and option tests passed.'
