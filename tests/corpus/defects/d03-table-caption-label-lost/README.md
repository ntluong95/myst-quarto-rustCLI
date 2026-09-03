# D03 — `:::{table}` caption and label both lost (myst_to_quarto)

**Defect statement.** In modern MyST, a table's caption is the directive's
**body** text (the paragraph before the pipe table), not its argument line.
The converter reads the caption from `frame.argument` — which is empty for
the modern body-caption form — and reads the id from `.options.get("name")`
instead of `"label"`. Both are empty, so the Quarto output has no `: Caption
{#tbl-...}` trailer at all; the caption paragraph is left as a disconnected
leading paragraph above the table instead.

**Reference-doc section.** docs/dialect-comparison.md §2 "Table + caption"
row: caption is the directive body in MyST, a trailing line in Quarto.
Catalogued as D3 in §12.

**Root cause.** `_transform_table` in
`src/mystquarto/transforms/myst_to_quarto.py:380-393` uses
`caption = frame.argument.strip()` and `name = frame.options.get("name", "")`.
Neither the caption-from-body nor the label-not-name issue is handled.

**Capture command.**

```bash
uv run python -c "
from mystquarto.transforms.myst_to_quarto import convert_myst_to_quarto
text = open('tests/corpus/defects/d03-table-caption-label-lost/input.md').read()
open('tests/corpus/defects/d03-table-caption-label-lost/python-actual.qmd', 'w').write(convert_myst_to_quarto(text))
"
```

**Verification.** `expected.qmd` rendered with `quarto render --to html` in a
scratch dir (minimal `_quarto.yml`). Exit 0, no `ERROR`.

**Result vs. prediction.** Matched exactly — the caption paragraph
("Summary of results across all sites.") and a blank line pass through
unchanged ahead of the raw pipe-table rows; no `{#tbl-results}` trailer is
emitted anywhere.
