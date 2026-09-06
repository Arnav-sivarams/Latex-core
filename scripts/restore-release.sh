#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
# shellcheck source=scripts/recovery-common.sh
source "$root/scripts/recovery-common.sh"

usage() {
  echo 'usage: restore-release.sh BACKUP_DIRECTORY --target-project NAME --target-env ABSOLUTE_ENV_FILE' >&2
  exit 2
}

backup_directory="${1:-}"
[[ -n "$backup_directory" ]] || usage
shift
target_project=''
target_env=''
while (($#)); do
  case "$1" in
    --target-project) target_project="${2:-}"; shift 2 ;;
    --target-env) target_env="${2:-}"; shift 2 ;;
    *) usage ;;
  esac
done
[[ "$backup_directory" == /* && "$target_env" == /* && -n "$target_project" ]] || usage
backup_directory="$(cd "$backup_directory" && pwd)"
target_env="$(cd "$(dirname "$target_env")" && pwd)/$(basename "$target_env")"
[[ -f "$target_env" ]] || { echo 'Target environment file does not exist.' >&2; exit 1; }

"$root/scripts/verify-backup.sh" "$backup_directory"
backup_commit="$(sed -n 's/.*"release_commit"[[:space:]]*:[[:space:]]*"\([0-9a-f]*\)".*/\1/p' "$backup_directory/manifest.json")"
backup_schema="$(sed -n 's/.*"schema_version"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p' "$backup_directory/manifest.json")"
backup_name="$(sed -n 's/.*"backup_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$backup_directory/manifest.json")"
recovery_point="$(sed -n 's/.*"recovery_point_utc"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$backup_directory/manifest.json")"
current_commit="$(git rev-parse HEAD)"
[[ "$backup_commit" == "$current_commit" ]] || {
  echo "Backup release $backup_commit does not match restore release $current_commit." >&2
  exit 1
}
expected_schema="$(find migrations -maxdepth 1 -type f -name '*.sql' -printf '%f\n' | sort | tail -n1 | cut -d_ -f1 | sed 's/^0*//')"
[[ "$backup_schema" == "$expected_schema" ]] || {
  echo "Backup schema $backup_schema does not match expected schema $expected_schema." >&2
  exit 1
}

mail_enabled="$(awk -F= '$1=="LATEX_CORE_MAIL_ENABLED"{print tolower($2)}' "$target_env" | tail -n1)"
[[ "${mail_enabled:-false}" == false ]] || {
  echo 'Restored targets must set LATEX_CORE_MAIL_ENABLED=false.' >&2
  exit 1
}
staging_root="$(awk -F= '$1=="WORKER_STAGING_HOST_ROOT"{print substr($0,index($0,"=")+1)}' "$target_env" | tail -n1)"
[[ "$staging_root" == /tmp/latex-core-* && "$staging_root" != /tmp/latex-core-worker-staging ]] || {
  echo 'Restored targets require a unique /tmp/latex-core-* worker staging path.' >&2
  exit 1
}

RECOVERY_PROJECT="$target_project"
recovery_init_compose "$root" "$RECOVERY_PROJECT" "$target_env"
existing_container="$(docker ps -aq --filter "label=com.docker.compose.project=$RECOVERY_PROJECT" | head -n1)"
existing_volume="$(docker volume ls -q --filter "label=com.docker.compose.project=$RECOVERY_PROJECT" | head -n1)"
[[ -z "$existing_container" && -z "$existing_volume" ]] || {
  echo 'Restore target is not empty. Use a new project name; in-place overwrite is intentionally unsupported.' >&2
  exit 1
}

phase='target-postgres-start'
restore_succeeded=false
on_exit() {
  status=$?
  if [[ "$restore_succeeded" != true ]]; then
    "${RECOVERY_COMPOSE[@]}" stop postgres >/dev/null 2>&1 || true
    echo "Restore failed during $phase. The source installation and backup were not modified." >&2
  fi
  exit "$status"
}
trap on_exit EXIT

"${RECOVERY_COMPOSE[@]}" up -d postgres >/dev/null
postgres_id="$(recovery_container postgres)"
for attempt in $(seq 1 60); do
  [[ "$(docker inspect "$postgres_id" --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}')" == healthy ]] && break
  ((attempt % 10)) || printf 'Waiting for restored PostgreSQL (%ss).\n' "$((attempt * 2))"
  sleep 2
done
[[ "$(docker inspect "$postgres_id" --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}')" == healthy ]] || {
  echo 'Restored PostgreSQL did not become healthy.' >&2
  exit 1
}
postgres_volume="$(recovery_volume_for_mount postgres /var/lib/postgresql)"
blob_volume="${RECOVERY_PROJECT}_latex_core_blob_data"
docker volume create \
  --label "com.docker.compose.project=$RECOVERY_PROJECT" \
  --label com.docker.compose.volume=latex_core_blob_data \
  "$blob_volume" >/dev/null
printf 'Verified empty restore project %s with PostgreSQL volume %s and BlobStore volume %s.\n' \
  "$RECOVERY_PROJECT" "$postgres_volume" "$blob_volume"

phase='database-restore'
docker exec -i "$postgres_id" pg_restore --exit-on-error --no-owner --no-privileges \
  -U "$RECOVERY_POSTGRES_USER" -d "$RECOVERY_POSTGRES_DB" <"$backup_directory/database.dump"

phase='blob-restore'
docker run --rm --network none -v "$blob_volume:/target" -v "$backup_directory:/backup:ro" alpine:3.21 \
  tar xzf /backup/blobs.tar.gz -C /target

phase='cross-store-verification'
recovery_validate_blob_volume "$blob_volume"
recovery_verify_referenced_blobs "$blob_volume"

phase='external-side-effect-suppression'
docker exec -i "$postgres_id" psql -X -v ON_ERROR_STOP=1 \
  -U "$RECOVERY_POSTGRES_USER" -d "$RECOVERY_POSTGRES_DB" <<'SQL' >/dev/null
TRUNCATE latex_core.sessions;
UPDATE latex_core.email_outbox
SET status='EXPIRED', secret_ciphertext=NULL, secret_nonce=NULL, claimed_at=NULL,
    last_error='suppressed in restored installation'
WHERE status IN ('PENDING','SENDING','FAILED');
SQL
recovery_record_event RESTORE_DRILL SUCCESS complete "$backup_name" "$backup_commit" "$backup_schema" "$recovery_point"

phase='application-start'
"${RECOVERY_COMPOSE[@]}" up -d api worker caddy >/dev/null
for attempt in $(seq 1 60); do
  api_id="$("${RECOVERY_COMPOSE[@]}" ps -q api)"
  [[ -n "$api_id" && "$(docker inspect "$api_id" --format '{{.State.Status}}')" == running ]] && break
  ((attempt % 10)) || printf 'Waiting for restored application (%ss).\n' "$((attempt * 2))"
  sleep 2
done
api_id="$(recovery_container api)"
[[ "$(docker inspect "$api_id" --format '{{.State.Status}}')" == running ]] || {
  echo 'Restored API did not start.' >&2
  exit 1
}

restore_succeeded=true
trap - EXIT
printf 'Restore verified and started.\nProject: %s\nRecovery point: %s\n' "$RECOVERY_PROJECT" "$recovery_point"
