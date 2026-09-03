# D07 — `analysis.ipynb.qmd` nonexistent chapter file (myst_to_quarto, config)

**Extension exception.** Config-level fixture; uses `.yml` throughout
(`input.yml` / `expected.yml` / `python-actual.yml`) per the phase-01 spec,
alongside the full `input_dir/myst.yml`.

**Defect statement.** MyST book `toc` entries can reference either markdown
source (`intro.md`) or a Jupyter notebook (`analysis.ipynb`) directly — the
extension is meaningful and type-dependent (`.md` files get converted and
renamed to `.qmd`; `.ipynb` files are read/executed as-is by Quarto and keep
their extension). The converter's extension rewrite only strips a literal
`.md` suffix and then unconditionally appends `.qmd` to **everything**,
producing the nonexistent chapter file `analysis.ipynb.qmd`.

**Reference-doc section.** docs/dialect-comparison.md §8.2 field mapping row
`toc[].file` → `book.chapters[]`: "Extension rewrite must be **type-aware**:
`.md`→`.qmd`, `.ipynb`→`.ipynb` (**unchanged**)." Catalogued as D7 in §12.

**Root cause.** `_toc_to_chapters` in `src/mystquarto/config.py:26-44`:
```python
if name.endswith(".md"):
    name = name[:-3]
chapters.append(f"{name}.qmd")
```
`"analysis.ipynb"` does not end with `.md`, so the strip is skipped, and
`.qmd` is appended onto the untouched `.ipynb` name regardless.

**Capture command.**

```bash
uv run python -c "
from mystquarto.config import convert_myst_config
convert_myst_config('tests/corpus/defects/d07-notebook-chapter-extension/input_dir/myst.yml', '/tmp/d07-capture')
"
# then copy /tmp/d07-capture/_quarto.yml -> tests/corpus/defects/d07-notebook-chapter-extension/python-actual.yml
```

**Verification.** `expected.yml` parsed as valid YAML. Used as `_quarto.yml`
for a trivial book project (`index.qmd` added as the book's home page for the
render check only — the committed `expected.yml`'s `chapters:` list itself
stays exactly `[intro.qmd, analysis.ipynb]`, per the fixture's minimal scope;
`index.qmd` is wrapper context needed only because Quarto book projects
require a home page). `quarto render` exits 0, builds `_book/index.html`, no
`ERROR`.

**Result vs. prediction.** Matched exactly —
`book.chapters: [intro.qmd, analysis.ipynb.qmd]`. (This config also happens
to trigger the `_is_book_project` "any toc" bug from D8, but since
`site.template: book-theme` is *also* set here, book-typing is correct either
way for this fixture — D07 isolates only the extension-rewrite defect.)
