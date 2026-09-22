#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
# shellcheck source=scripts/install-common.sh
source "$root/scripts/install-common.sh"
latex_core_init "$root"
exec "${LATEX_CORE_COMPOSE[@]}" "$@"
