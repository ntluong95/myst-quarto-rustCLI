# D14 — knitr inline `` `r expr` `` unhandled (quarto_to_myst)

**Defect statement.** Quarto's knitr engine supports a legacy-R-Markdown-style
bare inline-code form, `` `r expr` `` (no `{python}`/`{r}` fence-like
prefix), for inline computed values. The converter's inline-code regex only
matches the Jupyter-style `` `{python} expr` `` form, so `` `r expr` `` is
left completely untouched — it passes through as a literal, unevaluated
backtick span (which MyST would also just render as literal inline code,
not as anything computed).

**Reference-doc section.** docs/dialect-comparison.md §5 "Inline constructs"
— "Inline eval (R, knitr)" row: ➖ (MyST) / `` `r expr` `` (Quarto) / ❌
"knitr-only. See §6". §6 "Execution model", consequence 2: "`` `r expr` ``
inline (knitr) has no MyST form. Convert to `` {eval}`expr` `` **and warn**
that IRkernel is required." Catalogued as D14 in §12.

**Root cause.** `_INLINE_CODE_RE` in
`src/mystquarto/transforms/quarto_to_myst.py:12`:
```python
_INLINE_CODE_RE = re.compile(r"`\{python\}\s+([^`]+)`")
```
This pattern requires a literal `{python}` prefix inside the backticks. It
has no alternate branch, and no other regex in the module recognizes a bare
`` `r ...` `` span, so `transform_quarto_inline`
(quarto_to_myst.py:146-178) never touches it.

**Capture command.**

```bash
uv run python -c "
from mystquarto.transforms.quarto_to_myst import convert_quarto_to_myst
text = open('tests/corpus/defects/d14-knitr-inline-unhandled/input.qmd').read()
open('tests/corpus/defects/d14-knitr-inline-unhandled/python-actual.md', 'w').write(convert_quarto_to_myst(text))
"
```

**Verification.** `expected.md` built with `myst build expected.md --md
--force` in a scratch dir. Exit 0. Note: this specific `--md` re-export
target emits a non-fatal `⛔️ Unsupported node type: inlineExpression`
diagnostic (still exit 0) — this is a known limitation of myst's own
plain-markdown *round-trip exporter* for the `{eval}` role's AST node type,
not evidence the syntax itself is invalid. Confirmed separately: `myst build
--html` (with a minimal `site: {template: book-theme}` config) builds the
same content into a full site page with **zero** errors, proving `` {eval}`mean(x)` `` is valid, buildable modern MyST (per §5's ✅ row for this
exact construct) — the `--md` exporter's limitation is orthogonal to the
fixture's correctness.

**Result vs. prediction.** Matched exactly — `python-actual.md` is
byte-identical to `input.qmd`: `` `r mean(x)` `` passes through unchanged.

**Scope note (per the spec).** A fully correct fix must *also* warn that
IRkernel is required and add `kernelspec: {name: ir, display_name: R}` to
frontmatter (§6, consequence 1). This fragment has no frontmatter, so no
frontmatter fabrication was added to the fixture — the warning requirement is
recorded here in prose instead.
