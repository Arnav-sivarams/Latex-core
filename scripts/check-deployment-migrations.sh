#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=scripts/install-common.sh
source "$root/scripts/install-common.sh"
latex_core_init "$root"

postgres_id="$(latex_core_container_id postgres)"
[[ -n "$postgres_id" ]] || {
  echo 'Migration check failed: the selected deployment has no PostgreSQL container.' >&2
  exit 1
}
[[ "$(docker inspect -f '{{.State.Status}}' "$postgres_id" 2>/dev/null || true)" == running ]] || {
  echo 'Migration check failed: the selected deployment PostgreSQL container is not running.' >&2
  exit 1
}

expected=''
for file in "$root"/migrations/[0-9][0-9][0-9][0-9]_*.sql; do
  [[ -f "$file" ]] || { echo 'Migration check failed: no source migrations were found.' >&2; exit 1; }
  base="${file##*/}"
  version="${base%%_*}"
  checksum="$(sha384sum "$file")"
  checksum="${checksum%% *}"
  expected+="$((10#$version))|$checksum|t"$'\n'
done
expected="${expected%$'\n'}"

# PostgreSQL tooling runs inside the database container; no host psql package
# is required. SQLx records SHA-384 checksums in _sqlx_migrations.
installed="$("${LATEX_CORE_COMPOSE[@]}" exec -T postgres sh -ceu '
  PGPASSWORD="$POSTGRES_PASSWORD" psql -X -v ON_ERROR_STOP=1 \
    -U "$POSTGRES_USER" -d "$POSTGRES_DB" -At -F "|" \
    -c "SELECT version, encode(checksum, '\''hex'\''), success FROM public._sqlx_migrations ORDER BY version"
')" || {
  echo 'Migration check failed: PostgreSQL is unreachable or the SQLx migration table is missing.' >&2
  exit 1
}

if [[ "$installed" != "$expected" ]]; then
  expected_versions="$(cut -d'|' -f1 <<<"$expected" | paste -sd, -)"
  installed_versions="$(cut -d'|' -f1 <<<"$installed" | paste -sd, -)"
  echo 'Migration check failed: database versions/checksums do not match this checkout.' >&2
  printf 'Source versions:  %s\nInstalled versions: %s\n' "$expected_versions" "${installed_versions:-none}" >&2
  echo 'Do not replace application containers. Take and verify a backup, review compatibility, build the candidate images, then run the documented SQLx migration command.' >&2
  exit 1
fi

echo 'Database migration versions and checksums match this checkout.'
