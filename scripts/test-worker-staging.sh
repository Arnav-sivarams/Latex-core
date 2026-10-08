#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd -P)"
test_root="$(mktemp -d /tmp/latex-core-staging-test.XXXXXX)"
trap 'rm -rf -- "$test_root"' EXIT
mkdir -p "$test_root/repo/scripts" "$test_root/repo/deploy/compose" "$test_root/fake-bin"
cp "$root/scripts/verify-worker-staging.sh" "$root/scripts/install-common.sh" "$root/scripts/docker-exec.sh" "$root/scripts/generate-local-env.sh" "$test_root/repo/scripts/"
cp "$root/.env.example" "$test_root/repo/"
cp "$root/deploy/compose/docker-compose.yml" "$test_root/repo/deploy/compose/"
export TEST_STAGING_PATH="$test_root/latex-core-staging"
COMPOSE_PROJECT_NAME=latex-core-staging-test WORKER_STAGING_HOST_ROOT="$TEST_STAGING_PATH" \
  "$test_root/repo/scripts/generate-local-env.sh" --output "$test_root/repo/.env" >/dev/null
mkdir -p "$TEST_STAGING_PATH"
cat > "$test_root/fake-bin/docker" <<'DOCKER'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$*" == 'context show' ]]; then echo default
elif [[ "$*" == 'context inspect default --format {{.Endpoints.docker.Host}}' ]]; then echo unix:///var/run/docker.sock
elif [[ "$*" == 'info --format {{.OperatingSystem}}' ]]; then
  if [[ "$TEST_MOUNT_MODE" == desktop-other-daemon ]]; then echo Linux; else echo 'Docker Desktop'; fi
elif [[ "${1:-}" == info ]]; then exit 0
elif [[ "$*" == *' ps -aq worker' ]]; then echo worker-id
elif [[ "${1:-}" == inspect ]]; then
  python3 -c '
import json,os
mode=os.environ["TEST_MOUNT_MODE"]; staging=os.environ["TEST_STAGING_PATH"]
source = staging if mode in ("exact","readonly","volume") else "/wrong/path"
if mode.startswith("desktop"): source = "/run/desktop/mnt/host/wsl/docker-desktop-bind-mounts/Ubuntu/hash"
print(json.dumps([{"Type":"volume" if mode=="volume" else "bind", "Source":source, "Destination":staging, "RW":mode!="readonly"}]))'
elif [[ "$*" == *' exec -T worker '* ]]; then
  [[ "$TEST_MOUNT_MODE" != desktop-wrong-directory ]] || exit 1
  probe="${@: -2:1}"; nonce="${@: -1}"
  [[ "$(cat -- "$probe")" == "$nonce" ]]
else exit 1
fi
DOCKER
chmod +x "$test_root/fake-bin/docker"
export PATH="$test_root/fake-bin:$PATH"
for mode in exact desktop; do
  TEST_MOUNT_MODE="$mode" "$test_root/repo/scripts/verify-worker-staging.sh"
done
for mode in wrong readonly volume desktop-wrong-directory desktop-other-daemon; do
  if TEST_MOUNT_MODE="$mode" "$test_root/repo/scripts/verify-worker-staging.sh" > "$test_root/output" 2>&1; then
    echo "Invalid $mode staging unexpectedly passed." >&2
    exit 1
  fi
done
if compgen -G "$TEST_STAGING_PATH/.latex-core-mount-check.*" > /dev/null; then
  echo 'Staging verification left a probe behind.' >&2
  exit 1
fi
echo 'Worker staging: 7 mount cases passed; probes removed.'
