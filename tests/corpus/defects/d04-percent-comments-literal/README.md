# D04 — `%` line comments emitted as literal visible text (myst_to_quarto)

**Defect statement.** MyST treats a line starting with `%` as a comment (not
rendered). Quarto has no equivalent syntax and expects an HTML comment
(`<!-- ... -->`) instead. The converter has no handling for `%`-prefixed
lines at all, so the "comment" is emitted as a literal visible paragraph in
the Quarto output.

**Reference-doc section.** docs/dialect-comparison.md §9 "Comments and
structural syntax" — `% comment` at line start → `<!-- comment -->`.
Catalogued as D4 in §12.

**Root cause.** Neither `src/mystquarto/scanner.py` (no rule for lines
starting with `%`) nor `transform_inline` in
`src/mystquarto/transforms/myst_to_quarto.py:104-131` (no `%`-comment check)
handles this construct — a `%`-prefixed line is just a "regular line" that
passes through `inline_fn` unchanged, and no regex in `transform_inline`
matches it.

**Capture command.**

```bash
uv run python -c "
from mystquarto.transforms.myst_to_quarto import convert_myst_to_quarto
text = open('tests/corpus/defects/d04-percent-comments-literal/input.md').read()
open('tests/corpus/defects/d04-percent-comments-literal/python-actual.qmd', 'w').write(convert_myst_to_quarto(text))
"
```

**Verification.** `expected.qmd` rendered with `quarto render --to html` in a
scratch dir (minimal `_quarto.yml`). Exit 0, no `ERROR`.

**Result vs. prediction.** Matched exactly — both input lines pass through
byte-for-byte identical, including the literal `%` line.
