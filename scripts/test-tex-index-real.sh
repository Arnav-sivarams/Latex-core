#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
: "${TEXLIVE_BIN_DIR:?TEXLIVE_BIN_DIR must identify TeX Live 2026 binaries}"
cd "$repo_root"
cargo test -p tex-index --features texlive-tests --test real_texlive -- --nocapture
