# D05 — `(sec:x)=` heading target emitted as literal visible text (myst_to_quarto)

**Defect statement.** MyST labels a heading (or any block) by placing
`(label)=` on the line immediately before it. Quarto has no equivalent
target-before-block syntax; a heading's id is an inline `{#sec-...}`
attribute on the heading line itself. The converter has no handling for the
`(label)=` line at all, so it is emitted as a literal, visible paragraph
directly above the (also unmodified) heading.

**Reference-doc section.** docs/dialect-comparison.md §3.1 "Defining a
label" — heading row: `(sec:data-analysis)=` on the line before the heading
(MyST) → `## Heading {#sec-data-analysis}` (Quarto). Catalogued as D5 in §12.

**Root cause.** No code path in `src/mystquarto/scanner.py` or
`src/mystquarto/transforms/myst_to_quarto.py` recognizes the `(label)=`
target-before-block pattern; it is treated as an ordinary paragraph line.

**Capture command.**

```bash
uv run python -c "
from mystquarto.transforms.myst_to_quarto import convert_myst_to_quarto
text = open('tests/corpus/defects/d05-heading-target-literal/input.md').read()
open('tests/corpus/defects/d05-heading-target-literal/python-actual.qmd', 'w').write(convert_myst_to_quarto(text))
"
```

**Verification.** `expected.qmd` rendered with `quarto render --to html` in a
scratch dir (minimal `_quarto.yml`). Exit 0, no `ERROR`.

**Result vs. prediction.** Matched exactly — both lines (`(sec:data-analysis)=`
and `## Data Analysis`) pass through byte-for-byte unchanged.
