#!/usr/bin/env bash
set -u

root="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=scripts/install-common.sh
source "$root/scripts/install-common.sh"
umask 077
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
evidence_root="$root/.install-diagnostics/$stamp-$$"
mkdir -p "$evidence_root"
chmod 700 "$root/.install-diagnostics" "$evidence_root" 2>/dev/null || true

branch="$(git -C "$root" branch --show-current 2>/dev/null || printf detached)"
commit="$(git -C "$root" rev-parse HEAD 2>/dev/null || printf unknown)"
context="$(docker context show 2>/dev/null || printf unavailable)"
endpoint="$(docker context inspect "$context" --format '{{.Endpoints.docker.Host}}' 2>/dev/null || printf unavailable)"
platform="$(docker info --format '{{.OSType}}/{{.Architecture}}' 2>/dev/null || printf unavailable)"

if latex_core_init "$root" 2>/dev/null; then
  project="$LATEX_CORE_PROJECT"
  env_path="$LATEX_CORE_ENV_FILE"
else
  project=unavailable
  env_path="$root/.env"
  LATEX_CORE_COMPOSE=(docker compose --project-name latex-core --env-file "$root/.env" -f "$root/deploy/compose/docker-compose.yml")
fi

{
  printf 'source_commit=%s\nsource_branch=%s\n' "$commit" "$branch"
  printf 'docker_context=%s\ndocker_endpoint=%s\ndocker_platform=%s\n' "$context" "$endpoint" "$platform"
  printf 'compose_project=%s\nenvironment_file=%s\n' "$project" "$env_path"
} >"$evidence_root/identity.txt"

{
  printf '%-10s %-12s %-8s %-8s %-8s %s\n' SERVICE STATE EXIT OOM RESTARTS IMAGE_ID
  for service in postgres api worker caddy; do
    id="$(latex_core_container_id "$service" 2>/dev/null)"
    if [[ -z "$id" ]]; then
      printf '%-10s %s\n' "$service" missing
      continue
    fi
    docker inspect "$id" --format "${service} {{.State.Status}} {{.State.ExitCode}} {{.State.OOMKilled}} {{.RestartCount}} {{.Image}}"
  done
} >"$evidence_root/service-status.txt" 2>&1

{
  for service in postgres api worker caddy; do
    id="$(latex_core_container_id "$service" 2>/dev/null)"
    [[ -n "$id" ]] || continue
    printf '%s\n' "[$service]"
    docker inspect "$id" --format '{{range .Mounts}}{{printf "%s source=%s name=%s -> %s (rw=%t)\n" .Type .Source .Name .Destination .RW}}{{end}}'
  done
} >"$evidence_root/mounts.txt" 2>&1

for service in postgres api worker caddy; do
  "${LATEX_CORE_COMPOSE[@]}" logs --no-color --tail 200 "$service" >"$evidence_root/$service.log" 2>&1 || true
done
chmod 600 "$evidence_root"/* 2>/dev/null || true

cat "$evidence_root/identity.txt"
cat "$evidence_root/service-status.txt"
printf 'Private diagnostic evidence: %s\n' "$evidence_root"
echo 'Raw bounded logs were saved with restrictive permissions and were not printed. Review them for secrets or personal data before sharing; automated handling is not a perfect redaction guarantee.'
