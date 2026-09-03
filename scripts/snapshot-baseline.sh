#!/usr/bin/env bash
# Regenerates every tests/corpus/defects/*/python-actual.* file by re-running
# the Python implementation, exactly as documented in
# tests/corpus/defects/README.md. Byte-identical output on every run is the
# whole point: it's how Phase 1 proves each defect is real, not a fluke of a
# single capture.
set -euo pipefail
cd "$(dirname "$0")/.."

DEFECTS=tests/corpus/defects

capture_body() {
  local dir="$1" module="$2" fn="$3" in_ext="$4" out_ext="$5"
  uv run python -c "
from mystquarto.transforms.${module} import ${fn}
text = open('${DEFECTS}/${dir}/input.${in_ext}').read()
open('${DEFECTS}/${dir}/python-actual.${out_ext}', 'w').write(${fn}(text))
"
}

capture_config() {
  local dir="$1"
  local scratch
  scratch="$(mktemp -d)"
  uv run python -c "
from mystquarto.config import convert_myst_config
convert_myst_config('${DEFECTS}/${dir}/input_dir/myst.yml', '${scratch}')
"
  cp "${scratch}/_quarto.yml" "${DEFECTS}/${dir}/python-actual.yml"
  rm -rf "${scratch}"
}

echo "== body-only fixtures (myst_to_quarto) =="
for dir in d01-colon-labels-unnormalized d02-figure-label-dropped \
           d03-table-caption-label-lost d04-percent-comments-literal \
           d05-heading-target-literal d11-notebook-embed-broken-link \
           d13a-include-myst-to-quarto; do
  echo "  $dir"
  capture_body "$dir" myst_to_quarto convert_myst_to_quarto md qmd
done

echo "== body-only fixtures (quarto_to_myst) =="
for dir in d13b-include-quarto-to-myst d14-knitr-inline-unhandled \
           d15-doi-citation-keys; do
  echo "  $dir"
  capture_body "$dir" quarto_to_myst convert_quarto_to_myst qmd md
done

echo "== D09 (frontmatter — inlines convert_file's exact sequence, see d09-block-scalar-mangled/README.md) =="
uv run python -c "
from mystquarto.frontmatter import extract_frontmatter, myst_to_quarto_frontmatter
from mystquarto.transforms.myst_to_quarto import convert_myst_to_quarto
import yaml
text = open('${DEFECTS}/d09-block-scalar-mangled/input.md').read()
fm, body = extract_frontmatter(text)
new_fm = myst_to_quarto_frontmatter(fm)
transformed_body = convert_myst_to_quarto(body)
fm_yaml = yaml.dump(new_fm, default_flow_style=False, sort_keys=False)
output_text = '---\n' + fm_yaml + '---\n' + transformed_body
open('${DEFECTS}/d09-block-scalar-mangled/python-actual.qmd', 'w').write(output_text)
"

echo "== config fixtures (myst_to_quarto) =="
for dir in d06-export-template-only d07-notebook-chapter-extension \
           d08-article-mistyped-book d10-config-fields-dropped; do
  echo "  $dir"
  capture_config "$dir"
done

echo "== D12 (CLI, --strict) =="
rm -rf /tmp/d12-out
set +e
d12_out="$(uv run myst2quarto "${DEFECTS}/d12-silent-warnings/input_dir" -o /tmp/d12-out --strict 2>&1)"
d12_exit=$?
set -e
{
  echo "Command: uv run myst2quarto tests/corpus/defects/d12-silent-warnings/input_dir -o /tmp/d12-out --strict"
  echo "Exit code: ${d12_exit}"
  echo ""
  echo "--- stdout+stderr ---"
  echo "${d12_out}"
  echo ""
  echo "--- resulting /tmp/d12-out/_quarto.yml ---"
  cat /tmp/d12-out/_quarto.yml
} > "${DEFECTS}/d12-silent-warnings/python-actual.txt"
rm -rf /tmp/d12-out

echo "== D16 (CLI, run twice, output dir nested inside input) =="
d16_dir="${DEFECTS}/d16-output-recursion/input_dir"
out="${d16_dir}/docs-quarto"
rm -rf "$out"
{
  echo "Commands run (from the project root):"
  echo ""
  echo "  find tests/corpus/defects/d16-output-recursion/input_dir -type f | sort"
  echo ""
  echo "  uv run myst2quarto tests/corpus/defects/d16-output-recursion/input_dir \\"
  echo "    -o tests/corpus/defects/d16-output-recursion/input_dir/docs-quarto"
  echo ""
  echo "  find tests/corpus/defects/d16-output-recursion/input_dir -type f | sort"
  echo ""
  echo "  uv run myst2quarto tests/corpus/defects/d16-output-recursion/input_dir \\"
  echo "    -o tests/corpus/defects/d16-output-recursion/input_dir/docs-quarto"
  echo ""
  echo "  find tests/corpus/defects/d16-output-recursion/input_dir -type f | sort"
  echo ""
  echo "--- tree BEFORE any run ---"
  find "$d16_dir" -type f | sort
  uv run myst2quarto "$d16_dir" -o "$out" > /dev/null
  echo ""
  echo "--- tree AFTER run 1 (output dir given as input_dir/docs-quarto) ---"
  find "$d16_dir" -type f | sort
  uv run myst2quarto "$d16_dir" -o "$out" > /dev/null
  echo ""
  echo "--- tree AFTER run 2 (identical command run again) ---"
  find "$d16_dir" -type f | sort
  echo ""
  echo "Capture method: real CLI run twice (\`uv run myst2quarto ... -o"
  echo ".../input_dir/docs-quarto\`), directory tree captured with \`find | sort\`"
  echo "before and after each run. The \`docs-quarto/\` scratch output was deleted"
  echo "after this capture (see README.md) — it is not committed; this file is the"
  echo "frozen record. Root-cause analysis of the nesting mechanism lives in"
  echo "README.md, not here — this file is the raw captured data only."
} > "${DEFECTS}/d16-output-recursion/python-actual.txt"
rm -rf "$out"

echo "Done. Diff against git to confirm byte-identical reproduction:"
echo "  git diff --stat -- ${DEFECTS}"
