#!/usr/bin/env bash
set -euo pipefail

printf '%s\n' \
  'V2 migration planner' \
  'Mode: READ-ONLY SOURCE SNAPSHOT' \
  'Live LaTeX Core database: NOT ACCESSED'

usage() {
  printf 'Usage: %s --dump /absolute/path/database.dump --output /absolute/path/output-directory [--strict-ready]\n' "$0"
}

fail() {
  printf 'error: %s\n' "$1" >&2
  exit 1
}

dump_path=''
output_path=''
strict_ready=false

while (($# > 0)); do
  case "$1" in
    --dump)
      (($# >= 2)) || fail '--dump requires a value'
      dump_path=$2
      shift 2
      ;;
    --output)
      (($# >= 2)) || fail '--output requires a value'
      output_path=$2
      shift 2
      ;;
    --strict-ready)
      strict_ready=true
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      usage >&2
      fail "unknown argument: $1"
      ;;
  esac
done

[[ -n "$dump_path" ]] || fail '--dump is required'
[[ -n "$output_path" ]] || fail '--output is required'
[[ "$dump_path" == /* ]] || fail '--dump must be an absolute path'
[[ "$output_path" == /* ]] || fail '--output must be an absolute path'
[[ -f "$dump_path" && -s "$dump_path" ]] || fail 'dump must be an existing, non-empty regular file'

script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
repository_root=$(cd "$script_directory/../.." && pwd -P)

if [[ -e "$output_path" && ! -d "$output_path" ]]; then
  fail 'output path exists and is not a directory'
fi
mkdir -p -- "$output_path"
output_directory=$(cd "$output_path" && pwd -P)
[[ "$output_directory" != "$repository_root" ]] || fail 'output directory must not be the repository root'
[[ -w "$output_directory" ]] || fail 'output directory is not writable'

shopt -s nullglob dotglob
existing_output=("$output_directory"/*)
shopt -u nullglob dotglob
((${#existing_output[@]} == 0)) || fail 'output directory must be empty'

command -v docker >/dev/null 2>&1 || fail 'Docker is not available'
docker info >/dev/null 2>&1 || fail 'Docker daemon is not available'

postgres_image='postgres:18.4'
if ! docker image inspect "$postgres_image" >/dev/null 2>&1; then
  printf 'PostgreSQL image %s is not local; pulling it now.\n' "$postgres_image"
  docker pull "$postgres_image" >/dev/null
fi

dump_checksum_line=$(sha256sum -- "$dump_path")
dump_sha256=${dump_checksum_line%% *}
container_name="latex-core-v2-planner-$$-${RANDOM}"
container_started=false

cleanup() {
  if [[ "$container_started" == true ]]; then
    docker rm -f "$container_name" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

printf 'Starting isolated PostgreSQL restore (%s).\n' "$container_name"
docker run -d --rm \
  --name "$container_name" \
  --network none \
  --tmpfs /var/lib/postgresql:rw,nosuid,size=1g \
  -e POSTGRES_HOST_AUTH_METHOD=trust \
  -e POSTGRES_USER=planner \
  -e POSTGRES_DB=legacy \
  -e PGDATA=/var/lib/postgresql/data \
  -v "$dump_path:/input/database.dump:ro" \
  -v "$script_directory/sql:/planner:ro" \
  -v "$output_directory:/output:rw" \
  "$postgres_image" >/dev/null
container_started=true

# The official image briefly starts a bootstrap server. Require three
# consecutive successful probes so restore cannot race the final restart.
consecutive_ready=0
for attempt in $(seq 1 90); do
  if docker exec "$container_name" pg_isready -U planner -d legacy >/dev/null 2>&1; then
    consecutive_ready=$((consecutive_ready + 1))
  else
    consecutive_ready=0
  fi
  if ((consecutive_ready >= 3)); then
    break
  fi
  if ((attempt == 90)); then
    docker logs "$container_name" >&2
    fail 'isolated PostgreSQL did not become ready'
  fi
  sleep 1
done

docker exec "$container_name" pg_restore --list /input/database.dump >/dev/null
docker exec "$container_name" pg_restore \
  --exit-on-error \
  --no-owner \
  --no-privileges \
  -U planner \
  -d legacy \
  /input/database.dump

docker exec \
  --user "$(id -u):$(id -g)" \
  "$container_name" \
  psql -X -q -v ON_ERROR_STOP=1 \
  -v "dump_sha256=$dump_sha256" \
  -v 'source_commit=2fe37787e86d7bc3ac10bd761549466ea6a8e0d2' \
  -v 'target_commit=170e94611c7228e9da88bc5c5c87122b8d36b896' \
  -U planner \
  -d legacy \
  -f /planner/plan.sql

required_outputs=(
  migration-plan.json
  summary.txt
  reconciliation.tsv
  users.tsv
  user-role-decisions.tsv
  personal-papers.tsv
  paper-team-plan.tsv
  legacy-teams.tsv
  team-memberships.tsv
  research-groups.tsv
  private-work.tsv
  templates.tsv
  file-policy-plan.tsv
  unresolved.tsv
  README.txt
)
for output_file in "${required_outputs[@]}"; do
  [[ -f "$output_directory/$output_file" ]] || fail "planner did not create $output_file"
  chmod 600 "$output_directory/$output_file"
done

if ! grep -Eq '"ready_for_destructive_migration"[[:space:]]*:[[:space:]]*false' \
  "$output_directory/migration-plan.json"; then
  fail 'C2 safety invariant violated: readiness is not false'
fi

printf 'Migration plan written to %s\n' "$output_directory"
if [[ "$strict_ready" == true ]]; then
  printf 'Strict readiness check failed: unresolved migration decisions remain.\n' >&2
  exit 2
fi
