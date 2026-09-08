#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")" && pwd)"
cd "$root"
# shellcheck source=scripts/install-common.sh
source "$root/scripts/install-common.sh"
local_image='latex-core-texlive:2026-m7'
default_source='ghcr.io/arnav-sivarams/latex-core-texlive:2026-m7'
expected='sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38'
skip_admin=false
configure_mail=false
verify_only=false
phase=argument-validation

usage() {
  cat <<'EOF'
Usage: ./install.sh [--verify-only|--skip-admin|--configure-mail]

  --verify-only     Run installation checks without applying migrations or restarting services.
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
if [[ "$verify_only" == true ]]; then exec "$root/scripts/verify-install.sh"; fi

on_exit() {
  local status=$?
  trap - EXIT
  if ((status != 0)); then
    echo "Installation failed during phase: $phase (exit $status)." >&2
    "$root/scripts/diagnose-install.sh" || true
    echo 'Correct the named failure and rerun ./install.sh. Existing .env, accounts, blobs, and database volumes were not reset.' >&2
  fi
  exit "$status"
}
trap on_exit EXIT

phase=host-contract
"$root/scripts/check-install-host.sh"

phase=compiler-image-verification
actual="$(docker image inspect "$local_image" --format '{{.Id}}' 2>/dev/null || true)"
if [[ -z "$actual" ]]; then
  source_image="$default_source"
  if [[ -f "$root/.env" ]]; then
    configured_source="$(latex_core_env_value LATEX_CORE_M7_IMAGE_SOURCE "$root/.env")"
    [[ -z "$configured_source" ]] || source_image="$configured_source"
  fi
  echo "Frozen M7 image is missing; pulling published source $source_image ..."
  docker pull "$source_image"
  pulled="$(docker image inspect "$source_image" --format '{{.Id}}')"
  [[ "$pulled" == "$expected" ]] || { echo "Pulled M7 image identity mismatch: expected $expected, got $pulled" >&2; exit 1; }
  docker tag "$source_image" "$local_image"
  actual="$(docker image inspect "$local_image" --format '{{.Id}}')"
fi
platform="$(docker image inspect "$local_image" --format '{{.Os}}/{{.Architecture}}')"
[[ "$actual" == "$expected" ]] || { echo "Frozen M7 image identity mismatch: expected $expected, got $actual" >&2; exit 1; }
[[ "$platform" == linux/amd64 ]] || { echo "Frozen M7 image platform mismatch: expected linux/amd64, got $platform" >&2; exit 1; }
echo 'Frozen M7 compiler image identity and platform verified.'

phase=environment-configuration
if [[ ! -f "$root/.env" ]]; then
  "$root/scripts/generate-local-env.sh"
  echo 'Basic access is bound to 127.0.0.1 for local use or an SSH tunnel; SMTP remains disabled.'
else
  echo 'Existing .env preserved.'
fi
latex_core_init "$root"
python3 "$root/scripts/validate-install-config.py" "$LATEX_CORE_ENV_FILE"

if [[ "$configure_mail" == true ]]; then
  "$root/scripts/configure-smtp.sh"
elif [[ -t 0 ]]; then
  read -r -p 'Configure SMTP password email delivery now? [y/N] ' answer
  case "$answer" in y|Y|yes|YES) "$root/scripts/configure-smtp.sh" ;; esac
fi
python3 "$root/scripts/validate-install-config.py" "$LATEX_CORE_ENV_FILE"

phase=service-startup
"$root/scripts/start-install-services.sh"

phase=admin-bootstrap
if [[ "$skip_admin" != true ]]; then
  user_list="$("$root/latex-core" user list)"
  if grep -q 'v2=admin' <<<"$user_list"; then
    echo 'Admin account already exists; bootstrap skipped.'
  elif [[ -t 0 ]]; then
    "$root/scripts/bootstrap-admin.sh"
  else
    echo 'No Admin exists and input is non-interactive. Run ./scripts/bootstrap-admin.sh from a terminal.' >&2
    exit 1
  fi
fi

phase=final-verification
"$root/scripts/verify-install.sh"
url="$("$root/latex-core" url)"
mail_state=disabled
[[ "$(latex_core_env_value LATEX_CORE_MAIL_ENABLED "$LATEX_CORE_ENV_FILE")" == true ]] && mail_state=configured
phase=complete
trap - EXIT
cat <<EOF
------------------------------------------------------------
LaTeX Core is ready
------------------------------------------------------------

URL: $url
SMTP: $mail_state

Login with the Admin account created during installation.
For a remote basic installation, tunnel the configured HTTP port over SSH.
Institution-facing HTTPS requires operator-provided DNS/TLS/reverse-proxy policy.

Commands:
  ./latex-core status
  ./latex-core doctor
  ./latex-core diagnose
  ./latex-core stop
  ./latex-core restart
------------------------------------------------------------
EOF
