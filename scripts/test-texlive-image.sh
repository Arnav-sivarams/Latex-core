#!/usr/bin/env bash
set -euo pipefail
script_dir="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"
cd -- "$repo_root"
command -v docker >/dev/null || { echo 'Docker is required' >&2; exit 1; }
docker info >/dev/null
image_ref="${1:-latex-core-texlive:2026-m7}"
image_id="$(docker image inspect "$image_ref" --format '{{.Id}}')"
[[ "$image_id" =~ ^sha256:[0-9a-f]{64}$ ]] || { echo 'immutable local image ID required' >&2; exit 1; }
run=(docker run --rm --pull=never --network=none --read-only --tmpfs=/tmp:rw,noexec,nosuid,nodev,size=256m,mode=1777 --user=10001:10001)
[[ "$("${run[@]}" --entrypoint=/usr/bin/id "$image_id" -u)" == 10001 ]]
"${run[@]}" --entrypoint=/opt/texlive/2026/bin/x86_64-linux/tlmgr "$image_id" --version | grep -q 2026
for tool in latex pdflatex lualatex xelatex latexmk bibtex biber makeglossaries dvips kpsewhich tlmgr; do
  "${run[@]}" --entrypoint=/usr/bin/env "$image_id" "$tool" --version >/dev/null
done
biber_cache="$("${run[@]}" --entrypoint=/opt/texlive/2026/bin/x86_64-linux/biber "$image_id" --cache)"
case "$biber_cache" in
  /opt/latex-core/biber-runtime|/opt/latex-core/biber-cache/*) ;;
  *) echo 'Biber cache must resolve under /opt/latex-core' >&2; exit 1 ;;
esac
makeindex_output="$("${run[@]}" --entrypoint=/usr/bin/env "$image_id" makeindex 2>&1)"
grep -q '^This is makeindex, version' <<<"$makeindex_output"

# ps2pdf has no conventional --version contract; validate real conversion instead.
(
  ps2pdf_pdf="$(mktemp)"
  trap 'rm -f "$ps2pdf_pdf"' EXIT

  printf '%s\n' \
    '%!PS-Adobe-3.0' \
    '%%Pages: 1' \
    '%%BoundingBox: 0 0 100 100' \
    '%%Page: 1 1' \
    '/Helvetica findfont 12 scalefont setfont' \
    '10 50 moveto' \
    '(latex-core ps2pdf self-test) show' \
    'showpage' \
    '%%EOF' \
  | "${run[@]}" -i \
      --entrypoint=/usr/bin/env \
      "$image_id" \
      ps2pdf - - \
      >"$ps2pdf_pdf"

  test -s "$ps2pdf_pdf"
  test "$(head -c 5 "$ps2pdf_pdf")" = '%PDF-'
)

"${run[@]}" --entrypoint=/usr/local/libexec/latex-core/tex-index "$image_id" build --bin-dir /opt/texlive/2026/bin/x86_64-linux --output /tmp/tool-index.json >/dev/null
for file in article.cls book.cls amsmath.sty tikz.sty beamer.cls IEEEtran.cls acmart.cls biblatex.sty glossaries.sty plain.bst; do
  "${run[@]}" --entrypoint=/opt/texlive/2026/bin/x86_64-linux/kpsewhich "$image_id" "$file" | grep -q .
done
temporary="$(mktemp -d)"
chmod 0777 "$temporary"
cleanup() { rm -rf "$temporary"; }
trap cleanup EXIT
test_output_mount="--mount=type=bind,source=$temporary,target=/out"
index_report="$("${run[@]}" "$test_output_mount" --entrypoint=/usr/local/libexec/latex-core/tex-index "$image_id" build --bin-dir /opt/texlive/2026/bin/x86_64-linux --output /out/regenerated.json)"
environment_id="$(sed -n 's/^environment_id=//p' <<<"$index_report")"
[[ "$environment_id" =~ ^texlive-2026-sha256-[0-9a-f]{64}$ ]] || { echo 'invalid TeX environment ID' >&2; exit 1; }
[[ -s "$temporary/regenerated.json" ]] || { echo 'regenerated environment index is missing or empty' >&2; exit 1; }
"${run[@]}" --entrypoint=/bin/cat "$image_id" /opt/latex-core/tex-environment-index.json > "$temporary/embedded.json"
[[ -s "$temporary/embedded.json" ]] || { echo 'embedded environment index is missing or empty' >&2; exit 1; }
cmp -- "$temporary/embedded.json" "$temporary/regenerated.json"
printf 'IMAGE_ID=%s\nTEX_ENVIRONMENT_ID=%s\n' "$image_id" "$environment_id"
