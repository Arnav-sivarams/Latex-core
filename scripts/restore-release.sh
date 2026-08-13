#!/usr/bin/env bash
set -euo pipefail
source_directory="${1:?usage: restore-release.sh BACKUP_DIRECTORY}"
test -f "${source_directory}/database.sql"
test -f "${source_directory}/blobs.tar.gz"
source_directory_absolute="$(cd "${source_directory}" && pwd)"
worker_id="$(docker compose -f deploy/compose/docker-compose.yml ps -q worker)"
blob_volume="$(docker inspect "${worker_id}" --format '{{range .Mounts}}{{if eq .Destination "/var/lib/latex-core/blobs"}}{{.Name}}{{end}}{{end}}')"
test -n "${blob_volume}"
docker compose -f deploy/compose/docker-compose.yml exec -T postgres psql -v ON_ERROR_STOP=1 -U "${POSTGRES_USER:-latex_core}" "${POSTGRES_DB:-latex_core}" <"${source_directory}/database.sql"
docker run --rm -v "${blob_volume}:/target" -v "${source_directory_absolute}:/backup:ro" alpine:3.21 sh -c 'tar xzf /backup/blobs.tar.gz -C /target'
printf 'Restore completed from %s\n' "${source_directory}"
