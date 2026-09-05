#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
email=''
password_stdin=false
usage() {
  cat <<'EOF'
Usage:
  ./scripts/bootstrap-admin.sh
  ./scripts/bootstrap-admin.sh --email EMAIL --password-stdin

The automation form reads exactly one password line from standard input.
Passwords are never accepted as command-line arguments.
EOF
}
case "${1:-}" in
  '') ;;
  --help|-h) usage; exit 0 ;;
  --email)
    [[ $# -eq 3 && "${3:-}" == --password-stdin ]] || { usage >&2; exit 2; }
    email="${2:-}"
    password_stdin=true
    ;;
  *) usage >&2; exit 2 ;;
esac
if [[ "$password_stdin" == true ]]; then
  IFS= read -r password || { echo 'No password received on standard input.' >&2; exit 2; }
fi
user_list="$("$root/latex-core" user list </dev/null)"
if grep -q 'v2=admin' <<<"$user_list"; then
  unset password 2>/dev/null || true
  echo 'Admin account already exists; skipping bootstrap.'
  exit 0
fi
if [[ "$password_stdin" != true ]]; then
  read -r -p 'Admin email: ' email
  read -r -s -p 'Password: ' password
  printf '\n'
  read -r -s -p 'Confirm password: ' confirmation
  printf '\n'
  [[ "$password" == "$confirmation" ]] || { echo 'Passwords do not match.' >&2; exit 2; }
fi
[[ "$email" == *@* ]] || { echo 'Admin email is invalid.' >&2; exit 2; }
(( ${#password} >= 12 && ${#password} <= 256 )) || { echo 'Password must be 12-256 characters.' >&2; exit 2; }
printf '%s\n' "$password" | "$root/latex-core" user bootstrap-admin --email "$email" --password-stdin
unset password
echo 'Admin bootstrap complete.'
