#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")" && pwd)"
cd "$root"
local_image='latex-core-texlive:2026-m7'
default_source='ghcr.io/arnav-sivarams/latex-core-texlive:2026-m7'
expected='sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38'
skip_admin=false
configure_mail=false
verify_only=false

usage() {
  cat <<'EOF'
Usage: ./install.sh [--verify-only|--skip-admin|--configure-mail]

  --verify-only     Run read-only installation checks.
  --skip-admin      Do not prompt to create the first Admin.
  --configure-mail  Configure SMTP interactively during installation.
EOF
}

while (($#)); do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --verify-only) verify_only=true ;;
    --skip-admin) skip_admin=true ;;
    --configure-mail) configure_mail=true ;;
    *) usage >&2; exit 2 ;;
  esac
  shift
done

if [[ "$verify_only" == true ]]; then
  exec ./scripts/verify-install.sh
fi

case "$(uname -s 2>/dev/null || true)" in
  Linux)
    if [[ -r /proc/version ]] && grep -qi microsoft /proc/version 2>/dev/null; then
      echo 'Environment: WSL2/Linux'
    else
      echo 'Environment: Linux'
    fi
    ;;
  Darwin) echo 'Environment: macOS' ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    echo 'Run LaTeX Core from WSL2/Ubuntu with Docker Desktop WSL integration.' >&2
    exit 1
    ;;
  *) echo 'Unsupported environment. See docs/INSTALL_LOCAL.md.' >&2; exit 1 ;;
esac

command -v git >/dev/null 2>&1 || { echo 'Missing prerequisite: Git. See docs/INSTALL_LOCAL.md.' >&2; exit 1; }
command -v docker >/dev/null 2>&1 || { echo 'Missing prerequisite: Docker CLI. See docs/INSTALL_LOCAL.md.' >&2; exit 1; }
docker info >/dev/null 2>&1 || { echo 'Missing prerequisite: reachable Docker daemon. See docs/INSTALL_LOCAL.md.' >&2; exit 1; }
docker compose version >/dev/null 2>&1 || { echo 'Missing prerequisite: Docker Compose v2. See docs/INSTALL_LOCAL.md.' >&2; exit 1; }

actual="$(docker image inspect "$local_image" --format '{{.Id}}' 2>/dev/null || true)"
if [[ -z "$actual" ]]; then
  source_image="$default_source"
  if [[ -f .env ]]; then
    configured_source="$(awk -F= '$1=="LATEX_CORE_M7_IMAGE_SOURCE"{print substr($0,index($0,"=")+1)}' .env | tail -n1)"
    [[ -z "$configured_source" ]] || source_image="$configured_source"
  fi
  echo "Frozen M7 image is missing; pulling $source_image ..."
  if ! docker pull "$source_image"; then
    echo "Unable to pull $source_image. Confirm internet access and that the GHCR package is public, then rerun ./install.sh." >&2
    exit 1
  fi
  pulled="$(docker image inspect "$source_image" --format '{{.Id}}')"
  [[ "$pulled" == "$expected" ]] || { echo "Pulled M7 image has unexpected identity: $pulled" >&2; exit 1; }
  docker tag "$source_image" "$local_image"
  actual="$(docker image inspect "$local_image" --format '{{.Id}}')"
fi
[[ "$actual" == "$expected" ]] || { echo "Frozen M7 image identity mismatch: expected $expected, got $actual" >&2; exit 1; }
echo 'Frozen M7 compiler image verified.'

if [[ ! -f .env ]]; then
  ./scripts/generate-local-env.sh
else
  echo 'Existing .env preserved.'
fi

if [[ "$configure_mail" == true ]]; then
  ./scripts/configure-smtp.sh
elif [[ -t 0 ]]; then
  read -r -p 'Configure SMTP password email delivery now? [y/N] ' answer
  case "$answer" in y|Y|yes|YES) ./scripts/configure-smtp.sh ;; esac
fi

echo 'Starting LaTeX Core (the first image build can take several minutes) ...'
start_log="$(mktemp /tmp/latex-core-install-start.XXXXXX)"
trap 'rm -f "$start_log"' EXIT
./latex-core start >"$start_log" 2>&1 &
start_pid=$!
elapsed=0
while kill -0 "$start_pid" 2>/dev/null; do
  if ((elapsed >= 600)); then
    kill "$start_pid" 2>/dev/null || true
    wait "$start_pid" 2>/dev/null || true
    echo 'LaTeX Core startup exceeded 10 minutes. Inspect ./latex-core logs.' >&2
    exit 1
  fi
  if ((elapsed % 20 == 0)); then printf '  startup in progress (%ss)\n' "$elapsed"; fi
  sleep 5
  elapsed=$((elapsed + 5))
done
if ! wait "$start_pid"; then
  sed -n '1,120p' "$start_log" >&2
  exit 1
fi

ready=false
for attempt in $(seq 1 60); do
  status_output="$(./latex-core status 2>/dev/null || true)"
  if grep -Eq '^Status[[:space:]]+Running$' <<<"$status_output"; then ready=true; break; fi
  ((attempt % 6)) || echo '  waiting for services ...'
  sleep 5
done
[[ "$ready" == true ]] || { echo 'Services did not become ready within 5 minutes. Run ./latex-core logs.' >&2; exit 1; }
./latex-core doctor

if [[ "$skip_admin" != true ]]; then
  user_list="$(./latex-core user list)"
  if grep -q 'v2=admin' <<<"$user_list"; then
    echo 'Admin account already exists; skipping bootstrap.'
  elif [[ -t 0 ]]; then
    ./scripts/bootstrap-admin.sh
  else
    echo 'No Admin exists and input is not interactive. Rerun ./scripts/bootstrap-admin.sh from a terminal.' >&2
    exit 1
  fi
fi

./scripts/verify-install.sh
url="$(./latex-core url)"
mail_state=disabled
grep -q '^LATEX_CORE_MAIL_ENABLED=true$' .env && mail_state=configured
cat <<EOF
------------------------------------------------------------
LaTeX Core is ready
------------------------------------------------------------

URL:
$url

Login with the Admin account you created.

Professor walkthrough:
docs/PROFESSOR_TEST_GUIDE.md

SMTP:
$mail_state

Commands:

./latex-core status
./latex-core doctor
./latex-core stop
./latex-core restart

------------------------------------------------------------
EOF
