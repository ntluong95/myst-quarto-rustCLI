# D02 — `:::{figure}` label dropped entirely (myst_to_quarto)

**Defect statement.** A MyST figure's crossref id is set with `:label:`. The
converter reads a different, legacy-only option name (`:name:`), so the
`{#fig-...}` attribute is silently omitted from the Quarto output and the
figure becomes unreferenceable.

**Reference-doc section.** docs/dialect-comparison.md §10 "Legacy read-only
surface" — `:name:` directive option is the legacy MyST spelling that maps to
modern `:label:`; §3.1 "Defining a label" (figure row: `:label:` →
`{#fig-...}`). Catalogued as D2 in §12.

**Root cause.** `_transform_figure` in
`src/mystquarto/transforms/myst_to_quarto.py:279-302` does
`name = frame.options.get("name", "")` — it never looks at `"label"`, which
is the option the input directive actually sets.

**Capture command.**

```bash
uv run python -c "
from mystquarto.transforms.myst_to_quarto import convert_myst_to_quarto
text = open('tests/corpus/defects/d02-figure-label-dropped/input.md').read()
open('tests/corpus/defects/d02-figure-label-dropped/python-actual.qmd', 'w').write(convert_myst_to_quarto(text))
"
```

**Verification.** `expected.qmd` rendered with `quarto render --to html` in a
scratch dir (minimal `_quarto.yml`, a 1x1 placeholder PNG at
`images/samples.png`). Exit 0, no `ERROR`.

**Result vs. prediction.** Matched exactly — output is
`![Sample distribution across sites.](images/samples.png){width="80%"}` with
no `#id` at all.
