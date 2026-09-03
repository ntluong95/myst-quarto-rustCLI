# D11 — Notebook output embed unhandled, produces broken image link (myst_to_quarto)

**Defect statement.** MyST's `:::{figure} #nb:cell-label` syntax embeds a
Jupyter notebook cell's rendered output as a figure — the argument
(`#nb:analysis`) is a cell reference, not a file path. The converter has no
special case for a `#`-prefixed figure argument; it treats it exactly like a
normal image path, producing `![...](#nb:analysis)` — a link that resolves
to nothing (an in-page anchor, not the notebook output) once rendered.

**Reference-doc section.** docs/dialect-comparison.md §7 "Composition:
includes and embeds" — "Embed notebook output" row:
`:::{figure} #nb:cell` (MyST) → `{{< embed nb.ipynb#fig-cell >}}` (Quarto).
§3.4's normalization table also has the `nb:analysis` → `fig-analysis` row
(notebook cell label → figure-prefixed label, "per target usage").
Catalogued as D11 in §12.

**Root cause.** `_transform_figure` in
`src/mystquarto/transforms/myst_to_quarto.py:279-302` does
`path = frame.argument.strip()` unconditionally and builds `![caption](path)`
— there is no check for a `#nb:`-prefixed (or any `#`-prefixed) argument that
would route it through embed-shortcode construction instead. The `:label:`
option is also dropped here for the same reason as D02 (`name = ...get("name")`,
not `"label"`).

**Capture command.**

```bash
uv run python -c "
from mystquarto.transforms.myst_to_quarto import convert_myst_to_quarto
text = open('tests/corpus/defects/d11-notebook-embed-broken-link/input.md').read()
open('tests/corpus/defects/d11-notebook-embed-broken-link/python-actual.qmd', 'w').write(convert_myst_to_quarto(text))
"
```

**Verification.** `expected.qmd` rendered with `quarto render --to html` in a
scratch dir containing a minimal `analysis.ipynb` whose single cell carries
`"label": "fig-analysis-output"` metadata **and** a pre-populated `outputs`
array (a fake `image/png` display output) — Quarto's embed feature requires
the referenced cell to already have output to embed; it does not execute
notebooks itself for this check. This notebook is scratch wrapper context
only, not part of the committed `expected.qmd`. Exit 0, no `ERROR`
("Rendering notebook previews ... Output created").

**Result vs. prediction.** Matched exactly —
`![Output of the regression analysis cell.](#nb:analysis)`, no attributes.

**Scope note (per the spec).** The correct Quarto form
(`{{< embed analysis.ipynb#fig-analysis-output >}}`) requires the *notebook
cell's own label* to also be rewritten from `nb:analysis` to
`fig-analysis-output` — a cross-file edit into `analysis.ipynb` itself. That
cross-file relabeling is out of scope for this single-file fixture; it is
Phase 5 / RD-3's job. This fixture only asserts the embed-shortcode form is
produced for the referencing document.
