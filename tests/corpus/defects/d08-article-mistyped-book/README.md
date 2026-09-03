# D08 — Article mis-typed as `book` (myst_to_quarto, config)

**Extension exception.** Config-level fixture; uses `.yml` throughout
(`input.yml` / `expected.yml` / `python-actual.yml`) per the phase-01 spec,
alongside the full `input_dir/myst.yml`.

**Defect statement.** The converter treats the mere *presence* of a
`project.toc` key as sufficient evidence that a MyST project is a Quarto
`book`, ignoring `site.template`. The `article-template` fixture (and this
reproduction) is an article — `site.template: article-theme` — that happens
to also declare a `toc`; it is mis-typed as `book` instead of `manuscript`.

**Reference-doc section.** docs/dialect-comparison.md §8.1 "Project type
inference" — the inference table's first-match-wins order puts
`site.template: article-theme` (→ `manuscript`) ahead of the generic
"`toc` present, no article template" (→ `book`) fallback. The doc explicitly
calls out this exact bug: "The current Python implementation treats **any**
`project.toc` as a book. ... Correct target is `manuscript`." Catalogued as
D8 in §12.

**Root cause.** `_is_book_project` in `src/mystquarto/config.py:10-23`:
```python
site = myst_config.get("site", {})
if site.get("template") == "book-theme":
    return True
project = myst_config.get("project", {})
if "toc" in project:
    return True
return False
```
It never checks for `site.template == "article-theme"` (or any
manuscript-indicating signal) before falling through to the blanket `"toc"
in project` check.

**Capture command.**

```bash
uv run python -c "
from mystquarto.config import convert_myst_config
convert_myst_config('tests/corpus/defects/d08-article-mistyped-book/input_dir/myst.yml', '/tmp/d08-capture')
"
# then copy /tmp/d08-capture/_quarto.yml -> tests/corpus/defects/d08-article-mistyped-book/python-actual.yml
```

**Verification.** `expected.yml` parsed as valid YAML. Used as `_quarto.yml`
for a trivial one-page project (`index.qmd`). `quarto render` recognizes
`project.type: manuscript`, exits 0, builds `_manuscript/index.html`, no
`ERROR`.

**Result vs. prediction.** Matched exactly —
`{project: {type: book}, book: {title: Sample Article, chapters: [article.qmd]}}`.
