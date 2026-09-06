#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
configuration="${1:?usage: scheduled-backup.sh PROTECTED_CONFIGURATION_FILE}"
[[ "$configuration" == /* && -f "$configuration" ]] || {
  echo 'Scheduled backup configuration must be an existing absolute path.' >&2
  exit 2
}
# The operator owns this file. It must contain only the documented assignments.
# shellcheck disable=SC1090
source "$configuration"
: "${LATEX_CORE_BACKUP_DESTINATION:?missing LATEX_CORE_BACKUP_DESTINATION}"
: "${LATEX_CORE_BACKUP_RETENTION_COUNT:?missing LATEX_CORE_BACKUP_RETENTION_COUNT}"
[[ "$LATEX_CORE_BACKUP_RETENTION_COUNT" =~ ^[1-9][0-9]*$ ]] || {
  echo 'LATEX_CORE_BACKUP_RETENTION_COUNT must be positive.' >&2
  exit 2
}

output="$("$root/scripts/backup-release.sh" "$LATEX_CORE_BACKUP_DESTINATION")"
printf '%s\n' "$output"
new_backup="$(printf '%s\n' "$output" | sed -n 's/^Verified backup: //p')"
[[ -n "$new_backup" && -s "$new_backup/COMPLETE" ]] || {
  echo 'Scheduled backup did not produce a verified restore point.' >&2
  exit 1
}

if [[ -n "${LATEX_CORE_BACKUP_RCLONE_DESTINATION:-}" ]]; then
  command -v rclone >/dev/null 2>&1 || { echo 'rclone is required for configured off-host copy.' >&2; exit 1; }
  remote="$LATEX_CORE_BACKUP_RCLONE_DESTINATION/$(basename "$new_backup")"
  rclone copy "$new_backup" "$remote"
  rclone check "$new_backup" "$remote" --one-way
fi

mapfile -t verified < <(
  find "$LATEX_CORE_BACKUP_DESTINATION" -mindepth 1 -maxdepth 1 -type d -name 'latex-core-*' \
    -exec test -s '{}/COMPLETE' ';' -print | sort -r
)
for ((index=LATEX_CORE_BACKUP_RETENTION_COUNT; index<${#verified[@]}; index++)); do
  candidate="${verified[$index]}"
  [[ "$candidate" == "$LATEX_CORE_BACKUP_DESTINATION"/latex-core-* && -s "$candidate/COMPLETE" ]] || {
    echo 'Retention candidate failed safety validation.' >&2
    exit 1
  }
  rm -rf -- "$candidate"
done
