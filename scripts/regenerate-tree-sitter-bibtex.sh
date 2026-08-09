#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
upstream="https://github.com/latex-lsp/tree-sitter-bibtex.git"
commit="8d04ed27b3bc7929f14b7df9236797dab9f3fa66"
cli_version="0.24.1"
abi="14"
vendor="$repo_root/crates/parser/vendor/tree-sitter-bibtex"
mode="${1:-regenerate}"
[[ "$mode" == "regenerate" || "$mode" == "--verify" ]] || { echo "usage: $0 [--verify]" >&2; exit 2; }
fail(){ echo "error: $*" >&2; exit 1; }
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
source_dir="$tmp/source"
if [[ -n "${TREE_SITTER_BIBTEX_SOURCE:-}" ]]; then
  [[ -d "$TREE_SITTER_BIBTEX_SOURCE" ]] || fail "source override is not a directory"
  [[ "$(git -C "$TREE_SITTER_BIBTEX_SOURCE" rev-parse HEAD)" == "$commit" ]] || fail "source override has wrong commit"
  mkdir "$source_dir"; cp -a "$TREE_SITTER_BIBTEX_SOURCE/." "$source_dir/"
else
  git init --quiet "$source_dir"; git -C "$source_dir" remote add origin "$upstream"
  git -C "$source_dir" fetch --quiet --depth 1 origin "$commit" || fail "failed to fetch exact commit"
  git -C "$source_dir" checkout --quiet --detach FETCH_HEAD
fi
check(){ [[ "$(sha256sum "$2" | cut -d ' ' -f 1)" == "$1" ]] || fail "source hash mismatch: $2"; }
check 8b153a42b82a394a2ee455264d95a654296a5991f5047fefed95b0ca7e9094cd "$source_dir/grammar.js"
check 1ea2e0e77561b251909d370f42704ea94048557815b1fc209971830db2b89f74 "$source_dir/src/grammar.json"
check 874d3fa1efb4927ece21e5bb02db625d10fad19160aeb91e1164c1726fc9344e "$source_dir/src/node-types.json"
cp "$source_dir/src/node-types.json" "$tmp/node-types.json"
if [[ -n "${TREE_SITTER_CLI_BIN:-}" ]]; then cli="$TREE_SITTER_CLI_BIN"; else cargo install tree-sitter-cli --version "$cli_version" --locked --root "$tmp/tool"; cli="$tmp/tool/bin/tree-sitter"; fi
[[ "$($cli --version)" == "tree-sitter $cli_version" ]] || fail "wrong Tree-sitter CLI version"
(cd "$source_dir" && "$cli" generate --abi "$abi" src/grammar.json)
cmp -s "$tmp/node-types.json" "$source_dir/src/node-types.json" || fail "node-types semantics changed"
files=(src/parser.c src/tree_sitter/alloc.h src/tree_sitter/array.h src/tree_sitter/parser.h)
for file in "${files[@]}"; do if [[ "$mode" == "--verify" ]]; then cmp -s "$source_dir/$file" "$vendor/$file" || fail "$file differs"; else cp "$source_dir/$file" "$vendor/$file"; fi; done
check f80451def942254e45a600258f7da6bb2b1a33878d8419448ffb50930782f8a2 "$source_dir/src/parser.c"
[[ "$mode" == "--verify" ]] && echo "Tree-sitter BibTeX vendored grammar verification succeeded."
