#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
destination="${1:?usage: backup-release.sh DESTINATION_DIRECTORY}"
mkdir -p "${destination}"
destination_absolute="$(cd "${destination}" && pwd)"
worker_id="$(docker compose -f deploy/compose/docker-compose.yml ps -q worker)"
blob_volume="$(docker inspect "${worker_id}" --format '{{range .Mounts}}{{if eq .Destination "/var/lib/latex-core/blobs"}}{{.Name}}{{end}}{{end}}')"
test -n "${blob_volume}"
docker compose -f deploy/compose/docker-compose.yml exec -T postgres pg_dump -U "${POSTGRES_USER:-latex_core}" "${POSTGRES_DB:-latex_core}" >"${destination}/database.sql"
docker run --rm -v "${blob_volume}:/source:ro" -v "${destination_absolute}:/backup" alpine:3.21 tar czf /backup/blobs.tar.gz -C /source .
printf 'Backup written to %s\n' "${destination}"
