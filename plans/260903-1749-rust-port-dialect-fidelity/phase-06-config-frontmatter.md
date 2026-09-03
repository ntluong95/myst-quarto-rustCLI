---
phase: 6
title: "Config & frontmatter mapping"
status: done
priority: P1
effort: "4d"
dependencies: [3, 4]
---

# Phase 6: Config & frontmatter mapping

## Overview

Map `myst.yml` ↔ `_quarto.yml` and per-file frontmatter with correct project-type
inference, complete field coverage, and preserved YAML style. Fixes D6, D7, D8,
D9, D10.

Independent of Phases 4/5 after Phase 3 — different files, no shared code.

## Requirements

- Functional: project type inferred per reference §8.1, including the
  `manuscript` case the Python version cannot produce.
- Functional: every field in reference §8.2 and §8.4 is mapped, preserved as a
  comment, or explicitly warned — no silent drops.
- Functional: `exports` entries carrying only a `template:` produce a valid
  `format:`, never `format: {}`.
- Functional: `toc` extension rewriting is type-aware — `.md`→`.qmd`,
  `.ipynb` unchanged.
- Functional: block scalars, key order, and comments survive round-trip.
- Functional: a `bibliography:` is synthesized when a `.bib` exists in the
  conversion set but `myst.yml` omits it (RT-14).
- Non-functional: unknown keys are preserved, not discarded.

> **Red team RT-01, RT-14, and the dependency correction applied.** The original
> phase declared `dependencies: [3]` and claimed independence from Phases 4/5,
> but its step 7 needs the engine Phase 4 records on `Document` and its step 6
> needs Phase 5's label machinery — and Phases 2, 4, 5, 6, 7 all write
> `mappings.toml`. It also made comment round-tripping the sole D10 channel on
> the strength of a false claim about `saphyr`.

## Architecture

### Project type inference (fixes D8)

Ordered rules from reference §8.1, first match wins:

| MyST signal | `project.type` |
|---|---|
| `site.template: book-theme` | `book` |
| `exports[].template` present, or `site.template: article-theme` | `manuscript` |
| `toc` with ≥2 entries, no article template | `book` |
| otherwise | `default` |

`article-template` hits rule 2 on both clauses and becomes `manuscript` —
correct, and unreachable by the Python implementation which returns `book` for
any `toc`.

The `manuscript` type also emits the `manuscript:` block:

```yaml
project:
  type: manuscript
manuscript:
  article: article.qmd
  notebooks:
    - notebook: analysis.ipynb
```

`manuscript.article` derives from `exports[].article`; `notebooks` from `toc`
entries with `.ipynb` extensions.

### Exports → format (fixes D6)

An export with `format:` maps directly. An export with only `template:` requires
inferring the format from the template name suffix:

| Template suffix | Inferred format |
|---|---|
| `*-typst` | `typst` |
| `*-tex`, `*-latex` | `pdf` |
| `*-docx` | `docx` |
| `*-jats` | `jats` |
| unrecognized | `pdf` + warning |

The template name itself is not portable and is preserved as a YAML comment
above the format key, so the information survives even though Quarto cannot use it.

### TOC → chapters (fixes D7)

Extension rewriting is per-extension, not "strip `.md`, append `.qmd`":

| MyST toc entry | Quarto chapter |
|---|---|
| `article.md` | `article.qmd` |
| `article` (no ext) | `article.qmd` |
| `analysis.ipynb` | `analysis.ipynb` (**unchanged**) |
| `data.csv` | preserved + warning (not a chapter) |

### Unmappable config fields (fixes D10)

Reference §8.2 marks `abbreviations`, `open_access`, `venue`, `id` as
unmappable. Per the accepted policy they are preserved as YAML comments:

```yaml
# mystquarto: no Quarto equivalent for these myst.yml fields
# open_access: true
# venue: The Morganton Scientific
# abbreviations:
#   CRISPR: Clustered regularly interspaced short palindromic repeats
```

**The comment is informational; the sidecar is authoritative.** The reverse
conversion reads `.mystquarto/preserved.json` (the same sidecar Phase 7 uses for
block-level preservation), not the comment text. The original design made the
comment itself the round-trip channel, which was fragile in two ways: it broke
entirely if the YAML layer could not emit comments (which `saphyr` cannot), and a
user hand-editing the comment silently corrupted recovery.

Splitting the roles keeps the human-readable note in `_quarto.yml` — genuinely
useful, since it tells the reader what was lost — while making recovery depend on
structured data. A hand-edited or missing comment is then harmless.

### Bibliography synthesis (RT-14)

`myst.yml` has no `bibliography` key; MyST auto-loads `references.bib` from the
project directory **and resolves DOI citation keys over the network** (proven by
`article-template/_build/cache/doi-*.csl.json`). Quarto does neither. A
field-for-field mapping therefore produces a `_quarto.yml` with no bibliography,
citeproc never runs, and every citation renders as literal text — while
`quarto render` exits 0.

Rules:
- If a `.bib` exists in the conversion set and no `bibliography` is set,
  synthesize one and emit a diagnostic saying so.
- Diagnose citation keys present in the documents but absent from every reachable
  `.bib`. The two DOI keys in `article.md` are in this category —
  `references.bib` contains only `matplotlib`, `numpy`, `pandas`, `scipy`.
- Do not fetch DOIs. Warn (`MQ03xx`) that MyST resolved them live and Quarto
  requires local entries.

## Related Code Files

- Create: `crates/mystquarto-core/src/config/mod.rs`
- Create: `crates/mystquarto-core/src/config/project_type.rs`
- Create: `crates/mystquarto-core/src/config/myst_to_quarto.rs`
- Create: `crates/mystquarto-core/src/config/quarto_to_myst.rs`
- Create: `crates/mystquarto-core/src/config/exports.rs`
- Create: `crates/mystquarto-core/src/frontmatter.rs`
- Read: `src/mystquarto/config.py`, `frontmatter.py`
- Read: `docs/dialect-comparison.md` §8
- Read: `article-template/myst.yml`, `article-template/docs-quarto/_quarto.yml`

## Implementation Steps

1. Implement `project_type.rs` with the four ordered rules; table-test each,
   using `article-template/myst.yml` as the `manuscript` case.
2. Implement the §8.2 field map as a table in `mappings.toml`
   (`[[config_field]]` rows), so coverage is auditable by reading data.
3. Implement `exports.rs` including template-suffix inference and comment
   preservation.
4. Implement type-aware toc/chapters rewriting.
5. Implement unmappable-field handling: informational comment in `_quarto.yml`
   **plus** the authoritative record in `.mystquarto/preserved.json`; the reverse
   path reads the sidecar. Port Phase 1 bucket B config tests here.
6. Port frontmatter mapping per §8.4. Note `label` currently maps to `id`,
   which Quarto ignores — it should become a heading anchor instead.
7. Handle the R-kernel case: `kernelspec: {name: ir}` → decide `engine: knitr`
   vs `jupyter: ir` based on whether the document uses knitr inline syntax
   (Phase 4 records the engine on `Document`).
8. Write the **exhaustive coverage test**: for every key in
   `article-template/myst.yml`, assert it appears in the output either as a
   mapped field or as a preservation comment. This is the direct fix for D10 —
   a test that no field can be silently dropped.

## Success Criteria

- [x] `article-template/myst.yml` → `project.type: manuscript` (D8)
- [x] `manuscript.article` and `manuscript.notebooks` populated
- [x] `exports: [{template: lapreprint-typst}]` → `format: {typst: {}}` with the
      template name preserved as a comment — never `format: {}` (D6)
- [x] `toc` entry `analysis.ipynb` → `analysis.ipynb`, not `analysis.ipynb.qmd` (D7)
- [x] `abstract: |` block scalar byte-identical after round trip (D9)
- [x] Exhaustive coverage test passes: zero silently dropped keys (D10)
- [x] `subtitle`, `short_title`, `description` all present in output
- [x] `subject` maps to `categories`, not `description`
- [x] Unmappable fields appear as informational comments **and** in
      `.mystquarto/preserved.json`; reverse conversion recovers them from the
      sidecar, and still works when the comment is hand-edited or deleted
- [x] `bibliography:` synthesized when a `.bib` exists and `myst.yml` omits it (RT-14)
- [x] Citation keys absent from every reachable `.bib` are diagnosed, including
      the two DOI keys in `article.md`
- [x] All Phase 1 bucket B config tests pass
- [x] Unknown/future keys pass through rather than being dropped — myst.yml →
      `_quarto.yml` (comment + sidecar); `_quarto.yml` → myst.yml is warned, not
      preserved (no sidecar-recovery channel exists for that direction — see
      Review Outcome below)

## Review Outcome (pre-completion code review)

An adversarial review (mirroring Phase 5's) found **2 Critical + 8 High + 6
Medium + 3 Low** real bugs the 262 green tests did not catch — same failure
class as Phase 5: correctness gaps unit tests happened not to exercise, not
missing test *coverage* in the sense of untested lines.

**Fixed this pass (Critical + High, user-scoped):**
- Critical: [`yaml/emit.rs`](../../crates/mystquarto-core/src/yaml/emit.rs)'s
  `quote_scalar_string` emitted a literal line break inside a double-quoted
  scalar for any multi-line string value — invalid YAML, `quarto render`
  failed on the whole project. Fixed: newlines are escaped (`\n`); multi-line
  `parts.abstract` is forced back to `|` block style on write.
- Critical: [`yaml/surgery.rs`](../../crates/mystquarto-core/src/yaml/surgery.rs)'s
  `segment_text` treated a blank line *inside* a multi-paragraph block
  scalar as the top-level key separator it is everywhere else, silently
  deleting the scalar's tail on an unrelated key edit. Fixed: block-scalar
  state is now tracked so blank/indented lines inside one stay in its
  continuation until a genuinely dedented line ends it.
- High: stale `.mystquarto/preserved.json` reverted user edits (asset copy
  ran after the sidecar write and `.mystquarto` wasn't in the asset skip
  list; an empty preserved-fields set skipped the write instead of clearing
  it). Fixed both.
- High: unknown/future `project.*`/`site.*` keys (including §8.2's
  unimplemented `math` row) were silently dropped on the forward direction;
  now preserved via the same comment + sidecar channel as the four named
  unmappable fields. Reverse direction (`_quarto.yml` → myst.yml) now warns
  on an unrecognized top-level key instead of silently narrowing — it does
  not preserve into output, since no sidecar-recovery channel exists for
  that direction (a real, disclosed scope gap, not implemented this pass).
- High: manuscript `toc` entries beyond the article/notebooks, book
  `part:`-grouped chapters, and `book.appendices` were silently dropped;
  now preserved/flattened with a warning.
- High: a bare-string MyST author entry (`authors: [- Name]`) was silently
  discarded on the forward direction; now widened to `{name: ...}` like the
  existing bare-string-affiliation handling.
- High: frontmatter-mapping warnings (`label`, `math` drops at §8.4) never
  reached `RunReport`/stderr; `QuartoWriter::write`/`MystWriter::write` now
  return them and `pipeline.rs` threads them into `BatchWarning`.
- The `article_template_fixture_drops_no_field` test now actually enumerates
  the fixture's `project.*`/`site.*` keys and asserts each is mapped or
  preserved, instead of a fixed list of `assert!(contains(...))` calls that
  couldn't fail on a newly-dropped field.

**Deferred (Medium + Low, explicit user decision — not fixed this pass):**
`find_bib_file` follows symlinks (reads outside the input root) and is
uncapped; `_quarto.yml` per-format options (`format.pdf.documentclass`, …)
are silently dropped despite a doc comment claiming otherwise; two
template-only exports mapping to the same Quarto format emit a duplicate
YAML key; a non-`.md`/`.ipynb` toc entry (`data.csv`) becomes a bogus
`.qmd` chapter; `config::sidecar::read` has no diagnostic channel for a
refused sidecar; `apply_edits` failure silently falls back to unmapped
frontmatter; `bibliography.rs`'s hand-rolled key parser misses
space-before-brace and parenthesis BibTeX entry forms; a heading/blockquote/
math-block citation isn't scanned for the RT-14 diagnostic; `kernelspec`
without `name` silently assumes Python. None of these lose data or break a
render on their own (unlike the two Criticals); each is a real but bounded
gap. Re-run the review focused on these before Phase 9 (ship) if closing
them is wanted.

Verified end-to-end after the fix: `cargo test --workspace` (275 pass, 0
fail, 8 ignored), `cargo clippy --workspace --all-targets` clean, `cargo fmt
--all -- --check` clean, and a real `myst2quarto article-template/` →
`quarto render --to html` (graceful Citeproc warnings, same as before) plus
direct reproductions of both Criticals' exact failure scenarios (a folded
`description: >` value; a two-paragraph `abstract: |` surviving a
`kernelspec` → `jupyter` edit).

## Risk Assessment

**Risk: comment-as-round-trip-channel is fragile.** Comments are not structured
data; hand-editing them breaks the reverse parse.
*Signal:* reverse-parse failures on hand-edited configs.
*Response:* parse defensively — a malformed preservation comment is ignored with
an informational diagnostic, never a hard error. The user's hand edit always
wins over the converter's bookkeeping.

**Risk: `manuscript` project type needs files the converter does not create.**
Quarto manuscripts expect a particular layout.
*Signal:* `quarto render` fails on the generated manuscript project in Phase 8.
*Response:* Phase 8 validates against the real renderer for exactly this reason.
If manuscript layout needs more scaffolding, either generate it or fall back to
`default` with a warning — decide with renderer evidence in hand, not by guessing
now.

**Risk: template-suffix inference guesses wrong.** `lapreprint-typst` → `typst`
is a reasonable read of the name, not a documented rule.
*Signal:* Quarto rejects the format, or the output does not match the intended
journal template.
*Response:* always warn on inference so the user knows a guess was made, and
accept a user-supplied override rather than relying on a fix shipping. Correcting
`mappings.toml` is **not** a one-line change as the original text claimed — it is
compiled in via `include_str!`, so a correction requires a full release across
crates.io, npm and six binary triples, with no local patch path.
