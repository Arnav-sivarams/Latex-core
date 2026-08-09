#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
upstream="https://github.com/latex-lsp/tree-sitter-latex.git"
commit="fa8df448fc2c0192a8c2f8cfc97de53cb2b4ecb9"
cli_version="0.24.1"
parser_abi="14"
vendor="$repo_root/crates/parser/vendor/tree-sitter-latex"

usage() {
  echo "usage: $0 [--verify]" >&2
}

fail() {
  echo "error: $*" >&2
  exit 1
}

if (( $# > 1 )); then
  usage
  exit 2
fi
mode="${1:-regenerate}"
if [[ "$mode" != "regenerate" && "$mode" != "--verify" ]]; then
  usage
  exit 2
fi

temp_root="$(mktemp -d)"
trap 'rm -rf "$temp_root"' EXIT
source_copy="$temp_root/source"

if [[ -n "${TREE_SITTER_LATEX_SOURCE:-}" ]]; then
  local_source="$TREE_SITTER_LATEX_SOURCE"
  [[ -d "$local_source" ]] || fail "TREE_SITTER_LATEX_SOURCE is not a directory: $local_source"
  git -C "$local_source" rev-parse --is-inside-work-tree >/dev/null 2>&1 \
    || fail "TREE_SITTER_LATEX_SOURCE is not a Git worktree: $local_source"
  local_head="$(git -C "$local_source" rev-parse HEAD)"
  [[ "$local_head" == "$commit" ]] \
    || fail "TREE_SITTER_LATEX_SOURCE HEAD is $local_head; expected $commit"
  mkdir -p "$source_copy"
  cp -a "$local_source/." "$source_copy/" \
    || fail "failed to copy local Tree-sitter LaTeX checkout"
else
  git init --quiet "$source_copy"
  git -C "$source_copy" remote add origin "$upstream"
  git -C "$source_copy" fetch --quiet --depth 1 origin "$commit" \
    || fail "failed to fetch exact Tree-sitter LaTeX commit from $upstream"
  git -C "$source_copy" checkout --quiet --detach FETCH_HEAD
fi

actual_head="$(git -C "$source_copy" rev-parse HEAD)"
[[ "$actual_head" == "$commit" ]] || fail "generation checkout HEAD is $actual_head; expected $commit"

check_hash() {
  local expected="$1"
  local file="$2"
  local actual
  actual="$(sha256sum "$file" | cut -d ' ' -f 1)"
  [[ "$actual" == "$expected" ]] || fail "SHA-256 mismatch for $file: got $actual"
}

check_hash "9f7dbb192076bebd6052355a4f6c2ce4b1d9d0184019113a1f019cf7b789082f" "$source_copy/grammar.js"
check_hash "617ee120cda4772e0d0998e21e7044f29c460a5f64e1c86c915e604a22e43616" "$source_copy/src/grammar.json"
check_hash "db931e1dddb6273d316db41664df4de1559ef2a803c52e97d217856f8db91351" "$source_copy/src/node-types.json"
check_hash "8a7475b893beb61bd263d3e940a9b78e5b14590d6cc0e0a503f61b92eab198cd" "$source_copy/src/scanner.c"

cp "$source_copy/src/node-types.json" "$temp_root/node-types.json"
if [[ -n "${TREE_SITTER_CLI_BIN:-}" ]]; then
  cli="$TREE_SITTER_CLI_BIN"
  [[ -x "$cli" ]] || fail "TREE_SITTER_CLI_BIN is not executable: $cli"
else
  cargo install tree-sitter-cli --version "$cli_version" --locked --root "$temp_root/tool" \
    || fail "failed to install tree-sitter-cli $cli_version into the temporary tool root"
  cli="$temp_root/tool/bin/tree-sitter"
fi
cli_output="$("$cli" --version)"
[[ "$cli_output" == "tree-sitter $cli_version" ]] \
  || fail "Tree-sitter CLI version is '$cli_output'; expected 'tree-sitter $cli_version'"

(cd "$source_copy" && "$cli" generate --abi "$parser_abi" src/grammar.json)

compare_file() {
  local generated="$1"
  local committed="$2"
  local label="$3"
  cmp --silent "$generated" "$committed" || fail "$label differs from the committed vendored file"
}

compare_file "$source_copy/grammar.js" "$vendor/grammar.js" "grammar.js"
compare_file "$source_copy/src/grammar.json" "$vendor/src/grammar.json" "src/grammar.json"
compare_file "$source_copy/src/scanner.c" "$vendor/src/scanner.c" "src/scanner.c"
compare_file "$temp_root/node-types.json" "$vendor/src/node-types.json" "src/node-types.json"

generated_files=(
  "src/parser.c"
  "src/tree_sitter/alloc.h"
  "src/tree_sitter/array.h"
  "src/tree_sitter/parser.h"
)
if [[ "$mode" == "--verify" ]]; then
  for file in "${generated_files[@]}"; do
    compare_file "$source_copy/$file" "$vendor/$file" "$file"
  done
else
  for file in "${generated_files[@]}"; do
    cp "$source_copy/$file" "$vendor/$file"
  done
fi

sha256sum "$source_copy/src/parser.c"
if [[ "$mode" == "--verify" ]]; then
  echo "Tree-sitter LaTeX vendored grammar verification succeeded."
fi
