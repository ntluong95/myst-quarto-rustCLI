# D10 — 8 config fields dropped, `description` overwritten by `subject` (myst_to_quarto, config)

**Extension exception.** Config-level fixture; uses `.yml` throughout
(`input.yml` / `expected.yml` / `python-actual.yml`) per the phase-01 spec,
alongside the full `input_dir/myst.yml`.

**Defect statement.** `myst_to_quarto_config` only ever reads a small,
hard-coded subset of `project.*` keys. Everything else — even fields that
have a documented Quarto analogue — is silently dropped. Worse, `subject` is
mapped onto the **same** output key (`description`) that the real
`description` field should occupy, so `subject`'s value silently overwrites
the real description rather than being additive or renamed to `categories`.

**Reference-doc section.** docs/dialect-comparison.md §8.2 "Field mapping" —
rows for `subtitle`, `short_title`, `description`, `subject`, `open_access`,
`venue`, `banner`, `abbreviations`, `id`, `site.template` all note "Currently
dropped" or "Currently mapped to `description`, which is wrong". Catalogued
as D10 in §12 (which also records the corrected count: `description` is
*overwritten*, not dropped; `downloads` was `[]` so its loss is vacuous —
this fixture includes `downloads: []` for completeness but does not count it
among the "8 dropped" fields).

**Root cause.** `myst_to_quarto_config` in `src/mystquarto/config.py:112-169`
only reads: `title`, `authors`, `toc` (book branch); `bibliography`,
`exports`, `github`, `license`, `keywords`, `date`, `subject` (both
branches). `subtitle`, `short_title`, `id`, `open_access`, `venue`, `banner`,
`abbreviations`, and `site.template` have no corresponding `if` clause
anywhere in the function, so they are never even inspected. The `subject`
clause (`config.py:166-167`) is `result["description"] = project["subject"]`
— note there is no earlier line copying `project["description"]` into
`result["description"]`, so this is the *only* write to that key.

**Capture command.**

```bash
uv run python -c "
from mystquarto.config import convert_myst_config
convert_myst_config('tests/corpus/defects/d10-config-fields-dropped/input_dir/myst.yml', '/tmp/d10-capture')
"
# then copy /tmp/d10-capture/_quarto.yml -> tests/corpus/defects/d10-config-fields-dropped/python-actual.yml
```

**Verification.** `expected.yml` parsed as valid YAML. Used as `_quarto.yml`
for a trivial one-page project (`index.qmd`, plus a placeholder
`banner.png` for the `image:` reference). `quarto render` exits 0, no
`ERROR`; the metadata dump confirms `subtitle`, `short-title`, `description`
(the real one), `categories: [Statistics]`, and `image: banner.png` all
parsed as intended.

**Mapping decisions for `expected.yml`** (per §8.2, row by row):
- `subtitle` → `subtitle` (direct, ✅)
- `short_title` → `short-title` (non-standard Quarto field, kept as
  document metadata per §8.4's "keep as metadata" guidance for the
  frontmatter analogue of this same field)
- `description` → `description` (the real description text, not `subject`'s)
- `subject` → `categories` (list-wrapped, per §8.2 ⚠️ row)
- `banner` → `image` (per §8.2 ⚠️ row; `thumbnail` is absent here so no
  banner/thumbnail collision to warn about)
- `id`, `open_access`, `venue`, `abbreviations` → preserved as `#`-prefixed
  YAML comments (❌ rows — "no Quarto feature. Preserve as comment", per
  §8.2 and §11's unmappable-inventory policy)
- `site.template: article-theme` → preserved as a comment noting the
  intended theme (§8.2 marks `site.template` → `format.html.theme` ⚠️, but
  "theme names do not correspond" — there is no article-theme → Quarto HTML
  theme lookup table, so a comment is the only non-fabricated option)
- `downloads: []` → omitted (vacuous; explicitly noted as harmless in §12's
  corrected D10 entry)

**Result vs. prediction.** Matched exactly —
`python-actual.yml` is `{title: Sample Article, description: Statistics}`.
Every other field is absent; `description` holds `subject`'s value.
