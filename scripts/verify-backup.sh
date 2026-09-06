#!/usr/bin/env bash
set -euo pipefail

backup_directory="${1:?usage: verify-backup.sh BACKUP_DIRECTORY}"
[[ "$backup_directory" == /* ]] || { echo 'Backup directory must be absolute.' >&2; exit 2; }
for required in manifest.json checksums.sha256 database.dump blobs.tar.gz COMPLETE; do
  [[ -s "$backup_directory/$required" ]] || { echo "Backup is incomplete: missing $required." >&2; exit 1; }
done

grep -q '"format_version"[[:space:]]*:[[:space:]]*1' "$backup_directory/manifest.json" || {
  echo 'Unsupported backup manifest format.' >&2
  exit 1
}
grep -q '"verification"[[:space:]]*:[[:space:]]*"passed"' "$backup_directory/manifest.json" || {
  echo 'Backup manifest is not marked verified.' >&2
  exit 1
}

(
  cd "$backup_directory"
  sha256sum --check --strict checksums.sha256 >/dev/null
  expected_complete="$(sha256sum manifest.json | cut -d ' ' -f 1)"
  recorded_complete="$(cut -d ' ' -f 1 COMPLETE)"
  [[ "$expected_complete" == "$recorded_complete" ]]
)

docker run --rm --network none -v "$backup_directory:/backup:ro" postgres:18.4 \
  pg_restore --list /backup/database.dump >/dev/null
gzip -t "$backup_directory/blobs.tar.gz"
if tar -tzf "$backup_directory/blobs.tar.gz" | awk '
  /^\// { bad=1 }
  /(^|\/)\.\.($|\/)/ { bad=1 }
  END { exit bad }
'; then :; else
  echo 'Blob archive contains an unsafe path.' >&2
  exit 1
fi

echo 'Backup verification passed.'
