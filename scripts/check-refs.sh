#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 2 ]; then
  printf 'usage: %s <html-file-or-dir> <renderer-log> [citation-key ...]\n' "$0" >&2
  exit 2
fi

html_target=$1
renderer_log=$2
shift 2

if [ -f "$html_target" ]; then
  html_files=("$html_target")
else
  html_files=()
  while IFS= read -r html_file; do
    html_files+=("$html_file")
  done < <(find "$html_target" -type f -name '*.html' | sort)
fi

if [ "${#html_files[@]}" -eq 0 ]; then
  printf 'no rendered HTML files found under %s\n' "$html_target" >&2
  exit 1
fi

if grep -R --line-number --fixed-strings '?@' "${html_files[@]}"; then
  printf 'unresolved Quarto cross-reference marker found\n' >&2
  exit 1
fi

if grep --line-number -E 'Citeproc: citation .+ not found|citation .+ not found' "$renderer_log"; then
  printf 'unresolved citation warning found in renderer log\n' >&2
  exit 1
fi

# RT-02: Ensure no preservation sidecar markers or unescaped preservation leaks into rendered HTML
if grep -R --line-number -E 'mystquarto:preserve|<!-- mystquarto MQ' "${html_files[@]}"; then
  printf 'unstripped or leaked preservation marker found in HTML\n' >&2
  exit 1
fi

for key in "$@"; do
  if grep -R --line-number --fixed-strings "${key}?" "${html_files[@]}"; then
    printf 'literal unresolved citation key found: %s\n' "$key" >&2
    exit 1
  fi
done
