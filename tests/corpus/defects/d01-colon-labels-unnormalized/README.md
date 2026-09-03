# D01 — Colon labels emitted unchanged into Quarto (myst_to_quarto)

**Defect statement.** MyST allows free-form, colon-namespaced labels
(`fig:samples`, `eq:chi-squared`). Quarto requires a hyphen-separated,
type-prefixed identifier and rejects colons outright. The converter never
normalizes labels, so every cross-reference and math label that uses the
MyST convention is emitted into Quarto verbatim and dead on arrival.

**Reference-doc section.** docs/dialect-comparison.md §3.4 "Normalization
rules (MyST → Quarto)" — `fig:samples` → `fig-samples`, `eq:chi-squared` →
`eq-chi-squared`. Catalogued as D1 in §12.

**Root cause.** `_transform_math` in
`src/mystquarto/transforms/myst_to_quarto.py` reads
`frame.options.get("label", "")` and writes it into `$$ {{#{label}}}` with no
normalization step at all (myst_to_quarto.py:305-317). The bare `@fig:samples`
citation-like reference in prose is untouched by any inline role regex in
`transform_inline` (myst_to_quarto.py:104-131) — none of the patterns match a
bare `@key` outside a `{role}`...`` construct, so it passes straight through
too.

**Capture command.**

```bash
uv run python -c "
from mystquarto.transforms.myst_to_quarto import convert_myst_to_quarto
text = open('tests/corpus/defects/d01-colon-labels-unnormalized/input.md').read()
open('tests/corpus/defects/d01-colon-labels-unnormalized/python-actual.qmd', 'w').write(convert_myst_to_quarto(text))
"
```

**Verification.** `expected.qmd` was rendered with `quarto render --to html`
in a scratch dir with a minimal `_quarto.yml` (`project: {type: default}`).
Exit 0. Quarto emits `WARNING ... Unable to resolve crossref @fig-samples`
(expected — the fixture is a single-paragraph fragment with no actual figure
defined) but no `ERROR`.

**Result vs. prediction.** Matched exactly — `python-actual.qmd` is identical
to the controller's prediction: `@fig:samples` and `$$ {#eq:chi-squared}`
pass through with colons intact.
