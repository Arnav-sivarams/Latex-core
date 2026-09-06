#!/usr/bin/env bash

# Shared target-identity checks for backup and restore commands. Callers enable
# strict mode before sourcing this file.

recovery_require_project_name() {
  [[ "$1" =~ ^[a-z0-9][a-z0-9_-]{1,62}$ ]] || {
    echo 'Compose project name must contain only lowercase letters, numbers, underscore, and dash.' >&2
    return 1
  }
}

recovery_init_compose() {
  local repository_root="$1" project_name="$2" environment_file="$3"
  recovery_require_project_name "$project_name"
  [[ -f "$environment_file" ]] || { echo 'Compose environment file does not exist.' >&2; return 1; }
  RECOVERY_ENV_FILE="$(cd "$(dirname "$environment_file")" && pwd)/$(basename "$environment_file")"
  RECOVERY_POSTGRES_USER="$(awk -F= '$1=="POSTGRES_USER"{print substr($0,index($0,"=")+1)}' "$RECOVERY_ENV_FILE" | tail -n1)"
  RECOVERY_POSTGRES_DB="$(awk -F= '$1=="POSTGRES_DB"{print substr($0,index($0,"=")+1)}' "$RECOVERY_ENV_FILE" | tail -n1)"
  RECOVERY_POSTGRES_USER="${RECOVERY_POSTGRES_USER:-latex_core}"
  RECOVERY_POSTGRES_DB="${RECOVERY_POSTGRES_DB:-latex_core}"
  export LATEX_CORE_ENV_FILE="$RECOVERY_ENV_FILE"
  RECOVERY_COMPOSE=(
    docker compose
    --project-name "$project_name"
    --env-file "$RECOVERY_ENV_FILE"
    -f "$repository_root/deploy/compose/docker-compose.yml"
  )
}

recovery_container() {
  local service="$1" container_id
  container_id="$("${RECOVERY_COMPOSE[@]}" ps -aq "$service")"
  [[ -n "$container_id" ]] || { echo "Expected $service container is missing." >&2; return 1; }
  local actual_project actual_service
  actual_project="$(docker inspect "$container_id" --format '{{index .Config.Labels "com.docker.compose.project"}}')"
  actual_service="$(docker inspect "$container_id" --format '{{index .Config.Labels "com.docker.compose.service"}}')"
  [[ "$actual_project" == "$RECOVERY_PROJECT" && "$actual_service" == "$service" ]] || {
    echo "Container identity mismatch for $service; refusing operation." >&2
    return 1
  }
  printf '%s\n' "$container_id"
}

recovery_volume_for_mount() {
  local service="$1" destination="$2" container_id volume_name
  container_id="$(recovery_container "$service")"
  volume_name="$(docker inspect "$container_id" --format "{{range .Mounts}}{{if and (eq .Type \"volume\") (eq .Destination \"$destination\")}}{{.Name}}{{end}}{{end}}")"
  [[ -n "$volume_name" ]] || { echo "Expected persistent volume at $destination is missing." >&2; return 1; }
  printf '%s\n' "$volume_name"
}

recovery_record_event() {
  local operation="$1" status="$2" phase="$3" backup_name="${4:-}" release_commit="${5:-}" schema_version="${6:-}" recovery_point="${7:-}"
  local postgres_id
  postgres_id="$(recovery_container postgres 2>/dev/null)" || return 0
  docker exec -i "$postgres_id" psql -X -v ON_ERROR_STOP=1 \
    -U "$RECOVERY_POSTGRES_USER" -d "$RECOVERY_POSTGRES_DB" \
    -v operation="$operation" -v status="$status" -v phase="$phase" \
    -v backup_name="$backup_name" -v release_commit="$release_commit" \
    -v schema_version="$schema_version" -v recovery_point="$recovery_point" <<'SQL' >/dev/null 2>&1 || true
INSERT INTO latex_core.operator_recovery_events
    (id,operation,status,phase,backup_name,release_commit,schema_version,recovery_point)
VALUES (
    gen_random_uuid(), :'operation', :'status', :'phase', NULLIF(:'backup_name',''),
    NULLIF(:'release_commit',''), NULLIF(:'schema_version','')::bigint,
    NULLIF(:'recovery_point','')::timestamptz
);
SQL
}

recovery_validate_blob_volume() {
  local volume_name="$1"
  docker run --rm --network none -v "$volume_name:/source:ro" alpine:3.21 sh -ceu '
    test -d /source/sha256
    test -z "$(find /source -type l -print -quit)"
    test -z "$(find /source -mindepth 1 ! -path /source/sha256 ! -path "/source/sha256/*" -print -quit)"
    find /source/sha256 -type f -print | while IFS= read -r file; do
      name="${file##*/}"
      case "$name" in
        .tmp-blob-*) continue ;;
        *[!0-9a-f]*|"") echo "Unexpected BlobStore entry." >&2; exit 1 ;;
      esac
      test "${#name}" -eq 64 || { echo "Unexpected BlobStore entry." >&2; exit 1; }
      first="$(printf %s "$name" | cut -c 1-2)"
      second="$(printf %s "$name" | cut -c 3-4)"
      test "$file" = "/source/sha256/$first/$second/$name" || {
        echo "BlobStore entry is outside its content-addressed path." >&2
        exit 1
      }
      actual="$(sha256sum "$file" | cut -d " " -f 1)"
      test "$actual" = "$name" || { echo "Blob hash mismatch." >&2; exit 1; }
    done
  '
}

recovery_verify_referenced_blobs() {
  local volume_name="$1" postgres_id
  postgres_id="$(recovery_container postgres)"
  docker exec -i "$postgres_id" psql -X -At -v ON_ERROR_STOP=1 \
    -U "$RECOVERY_POSTGRES_USER" -d "$RECOVERY_POSTGRES_DB" <<'SQL' |
SELECT DISTINCT hash FROM (
    SELECT manifest_blob_hash AS hash FROM latex_core.snapshots
    UNION ALL SELECT blob_hash FROM latex_core.compilation_artifacts
    UNION ALL SELECT artifact_manifest_blob_hash FROM latex_core.compile_cache
    UNION ALL SELECT blob_hash FROM latex_core.template_files
    UNION ALL SELECT blob_hash FROM latex_core.team_project_files
    UNION ALL SELECT draft_blob_hash FROM latex_core.member_drafts
    UNION ALL SELECT previous_blob_hash FROM latex_core.team_project_audit
    UNION ALL SELECT new_blob_hash FROM latex_core.team_project_audit
    UNION ALL SELECT blob_hash FROM latex_core.front_matter_pack_files
    UNION ALL SELECT logo_blob_hash FROM latex_core.application_branding
    UNION ALL
    SELECT operation ->> 'blob_hash'
    FROM latex_core.workspace_events,
         LATERAL jsonb_array_elements(payload -> 'operations') AS operation
    WHERE operation ->> 'op' = 'put_file'
) referenced WHERE hash IS NOT NULL ORDER BY hash;
SQL
  docker run --rm -i --network none -v "$volume_name:/source:ro" alpine:3.21 sh -ceu '
    count=0
    while IFS= read -r hash; do
      test "${#hash}" -eq 64 || { echo "Invalid referenced blob hash." >&2; exit 1; }
      first="$(printf %s "$hash" | cut -c 1-2)"
      second="$(printf %s "$hash" | cut -c 3-4)"
      file="/source/sha256/$first/$second/$hash"
      test -f "$file" || { echo "Referenced blob is missing." >&2; exit 1; }
      actual="$(sha256sum "$file" | cut -d " " -f 1)"
      test "$actual" = "$hash" || { echo "Referenced blob failed SHA-256 verification." >&2; exit 1; }
      count=$((count + 1))
    done
    printf "Verified %s referenced blobs.\n" "$count"
  '
}
