#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
# shellcheck source=scripts/recovery-common.sh
source "$root/scripts/recovery-common.sh"

destination_root="${1:?usage: backup-release.sh ABSOLUTE_DESTINATION_ROOT}"
[[ "$destination_root" == /* ]] || { echo 'Backup destination must be absolute.' >&2; exit 2; }
case "$destination_root" in
  /|"$root"|"$root"/*) echo 'Backup destination must be outside the repository.' >&2; exit 2 ;;
esac
mkdir -p "$destination_root"
destination_root="$(cd "$destination_root" && pwd)"

source_env="${LATEX_CORE_ENV_FILE:-$root/.env}"
[[ "$source_env" == /* ]] || source_env="$root/$source_env"
[[ -f "$source_env" ]] || { echo 'Source environment file does not exist.' >&2; exit 1; }
RECOVERY_PROJECT="${COMPOSE_PROJECT_NAME:-$(awk -F= '$1=="COMPOSE_PROJECT_NAME"{print substr($0,index($0,"=")+1)}' "$source_env" | tail -n1)}"
RECOVERY_PROJECT="${RECOVERY_PROJECT:-latex-core}"
recovery_init_compose "$root" "$RECOVERY_PROJECT" "$source_env"

phase='identity-check'
release_commit="$(git rev-parse HEAD)"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
recovery_point="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
backup_name="latex-core-${timestamp}-${release_commit:0:12}"
partial="$destination_root/.${backup_name}.partial.$$"
final="$destination_root/$backup_name"
[[ ! -e "$partial" && ! -e "$final" ]] || { echo 'Backup destination already exists.' >&2; exit 1; }
mkdir "$partial"

api_was_running=false
worker_was_running=false
backup_succeeded=false
resume_services() {
  if [[ "$api_was_running" == true || "$worker_was_running" == true ]]; then
    phase='service-resume'
    services=()
    [[ "$api_was_running" == true ]] && services+=(api)
    [[ "$worker_was_running" == true ]] && services+=(worker)
    "${RECOVERY_COMPOSE[@]}" up -d "${services[@]}" >/dev/null
  fi
}
on_exit() {
  status=$?
  failed_phase="$phase"
  resume_services || true
  if [[ "$backup_succeeded" != true ]]; then
    recovery_record_event BACKUP FAILED "$failed_phase" "$backup_name" "$release_commit" "${schema_version:-}" "$recovery_point"
    echo "Backup failed during $failed_phase; partial output was not marked complete." >&2
  fi
  exit "$status"
}
trap on_exit EXIT

postgres_id="$(recovery_container postgres)"
api_id="$(recovery_container api)"
worker_id="$(recovery_container worker)"
[[ "$(docker inspect "$api_id" --format '{{.State.Status}}')" == running ]] && api_was_running=true
[[ "$(docker inspect "$worker_id" --format '{{.State.Status}}')" == running ]] && worker_was_running=true
blob_volume="$(recovery_volume_for_mount api /var/lib/latex-core/blobs)"
postgres_volume="$(recovery_volume_for_mount postgres /var/lib/postgresql)"
printf 'Verified project %s with PostgreSQL volume %s and BlobStore volume %s.\n' \
  "$RECOVERY_PROJECT" "$postgres_volume" "$blob_volume"

phase='write-quiescence'
"${RECOVERY_COMPOSE[@]}" stop -t 30 api worker >/dev/null
for container_id in "$api_id" "$worker_id"; do
  [[ "$(docker inspect "$container_id" --format '{{.State.Status}}')" == exited ]] || {
    echo 'Application writer did not stop; refusing an inconsistent backup.' >&2
    exit 1
  }
done

phase='database-durability-check'
durability="$(docker exec "$postgres_id" psql -X -At -U "$RECOVERY_POSTGRES_USER" -d "$RECOVERY_POSTGRES_DB" -c "SELECT current_setting('fsync')||','||current_setting('synchronous_commit')||','||current_setting('full_page_writes')")"
[[ "$durability" == 'on,on,on' ]] || { echo 'PostgreSQL durability settings are not all enabled.' >&2; exit 1; }
schema_version="$(docker exec "$postgres_id" psql -X -At -U "$RECOVERY_POSTGRES_USER" -d "$RECOVERY_POSTGRES_DB" -c "SELECT max(version) FROM public._sqlx_migrations WHERE success")"

phase='database-dump'
docker exec "$postgres_id" pg_dump -Fc --no-owner --no-privileges \
  -U "$RECOVERY_POSTGRES_USER" "$RECOVERY_POSTGRES_DB" >"$partial/database.dump"

phase='blob-verification'
recovery_validate_blob_volume "$blob_volume"
recovery_verify_referenced_blobs "$blob_volume"

phase='blob-archive'
docker run --rm --network none -v "$blob_volume:/source:ro" -v "$partial:/backup" alpine:3.21 \
  tar czf /backup/blobs.tar.gz -C /source .

phase='artifact-verification'
docker run --rm --network none -v "$partial:/backup:ro" postgres:18.4 \
  pg_restore --list /backup/database.dump >/dev/null
gzip -t "$partial/blobs.tar.gz"
database_size="$(stat -c %s "$partial/database.dump")"
blob_size="$(stat -c %s "$partial/blobs.tar.gz")"
database_hash="$(sha256sum "$partial/database.dump" | cut -d ' ' -f 1)"
blob_hash="$(sha256sum "$partial/blobs.tar.gz" | cut -d ' ' -f 1)"
printf '%s  database.dump\n%s  blobs.tar.gz\n' "$database_hash" "$blob_hash" >"$partial/checksums.sha256"
cat >"$partial/manifest.json" <<EOF
{
  "format_version": 1,
  "application": "latex-core",
  "source_project": "$RECOVERY_PROJECT",
  "backup_name": "$backup_name",
  "release_commit": "$release_commit",
  "schema_version": $schema_version,
  "recovery_point_utc": "$recovery_point",
  "consistency_method": "quiesced-api-worker-plus-postgresql-custom-dump-and-immutable-blob-volume",
  "database": {"file": "database.dump", "bytes": $database_size, "sha256": "$database_hash"},
  "blobs": {"file": "blobs.tar.gz", "bytes": $blob_size, "sha256": "$blob_hash"},
  "required_secret_identifiers": ["POSTGRES_PASSWORD", "LATEX_CORE_MAIL_SECRET_KEY", "LATEX_CORE_SMTP_PASSWORD_IF_CONFIGURED"],
  "verification": "passed"
}
EOF
manifest_hash="$(sha256sum "$partial/manifest.json" | cut -d ' ' -f 1)"
printf '%s  manifest.json\n' "$manifest_hash" >"$partial/COMPLETE"
"$root/scripts/verify-backup.sh" "$partial" >/dev/null

phase='atomic-publication'
mv "$partial" "$final"
recovery_record_event BACKUP SUCCESS complete "$backup_name" "$release_commit" "$schema_version" "$recovery_point"
backup_succeeded=true
trap - EXIT
resume_services
printf 'Verified backup: %s\nRecovery point: %s\n' "$final" "$recovery_point"
