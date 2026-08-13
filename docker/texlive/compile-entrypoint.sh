#!/usr/bin/env bash
set -euo pipefail

engine=''
shell_policy=''
synctex=''
main=''
while (($#)); do
  case "$1" in
    --engine) (($# >= 2)) || exit 64; engine="$2"; shift 2 ;;
    --shell-policy) (($# >= 2)) || exit 64; shell_policy="$2"; shift 2 ;;
    --synctex) (($# >= 2)) || exit 64; synctex="$2"; shift 2 ;;
    --main) (($# >= 2)) || exit 64; main="$2"; shift 2 ;;
    *) echo "unsupported argument: $1" >&2; exit 64 ;;
  esac
done
case "$engine" in
  latex) mode=-pdfps ;;
  pdflatex) mode=-pdf ;;
  lualatex) mode=-lualatex ;;
  xelatex) mode=-xelatex ;;
  *) exit 64 ;;
esac
case "$shell_policy" in safe|restricted) profile="/opt/latex-core/${shell_policy}.latexmkrc" ;; *) exit 64 ;; esac
case "$synctex" in 0) synctex_option=() ;; 1) synctex_option=(-synctex=1) ;; *) exit 64 ;; esac
[[ -n "$main" && "$main" != /* && "$main" != *$'\n'* && "$main" != *$'\r'* ]] || exit 64

mkdir -p /tmp/home /tmp/texmf-home /tmp/texmf-var /tmp/texmf-config /work/.latex-core-out
cd /work
main_dir="$(dirname -- "$main")"
if [[ "$main_dir" == "." ]]; then
  source_base=/work
else
  source_base="/work/$main_dir"
fi
[[ -d "$source_base" ]] || exit 66
find "$source_base" -mindepth 1 \( -path /work/.latex-core-out -o -path '/work/.latex-core-out/*' \) -prune -o -type d -print0 |
  while IFS= read -r -d '' directory; do
    relative_dir="${directory#"$source_base/"}"
    mkdir -p -- "/work/.latex-core-out/$relative_dir"
  done
exec latexmk -cd -norc -r "$profile" "$mode" -interaction=nonstopmode -file-line-error -halt-on-error -recorder "${synctex_option[@]}" -outdir=/work/.latex-core-out "./$main"
