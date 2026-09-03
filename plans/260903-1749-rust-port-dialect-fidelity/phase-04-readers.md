---
phase: 4
title: "Readers: MyST + Quarto → IR"
status: done
priority: P1
effort: "6d"
dependencies: [3]
---

# Phase 4: Readers — MyST + Quarto → IR

## Overview

Parse both dialects into the IR. This is where the modern-MyST decision lands
and where the constructs the Python scanner never saw (`%` comments, `(target)=`,
`{{< shortcode >}}`, knitr inline) first become representable.

> **Red team RT-03, RT-11, RT-12, RT-13, RT-15 applied.** The original phase
> demanded a byte-identical round-trip that its own sibling phase makes
> impossible, had no reader for preservation output, no notebook index for
> embeds, no include containment, and inherited a D15 misdiagnosis that would
> have produced a phantom regression test. Effort raised 5d → 6d.

## Requirements

- Functional: `MystReader` parses modern mystmd v1 **and** accepts legacy roles
  on read (reference §10).
- Functional: `QuartoReader` parses divs, shortcodes, cell options, and both
  inline-code engines.
- Functional: every construct in reference §2/§5/§7/§9 either parses into a
  typed `BlockKind` or into `Unmappable` with its original source retained.
- Functional: no input is ever silently discarded — anything unrecognized
  becomes `Unmappable`, never nothing.
- Functional: both readers recognize the **preservation sidecar marker** and
  restore the original construct (RT-11) — without this, Phase 7's
  "preserved originals round-trip" criterion lands on nobody.
- Functional: a **notebook cell index** maps `#| label:` values to their
  notebook path across the conversion set (RT-03), so `FigureSource::CellRef`
  can carry the file `{{< embed >}}` requires.
- Functional: include targets are resolved through Phase 3's path guard —
  containment, cycle detection, depth cap (RT-15).
- Non-functional: every emitted block carries an accurate `span` and
  `blank_lines_before`.

## Architecture

Both readers share a **fence stack** but differ in what a fence means.

### MystReader

| Input | Produces |
|---|---|
| `` ```{name} arg `` / `:::{name} arg `` + `:key: val` lines | typed block via `mappings.toml` lookup |
| `(label)=` on its own line | `Target` — attaches to the **next** block (fixes D5) |
| `% text` at line start | `Comment { style: Percent }` (fixes D4) |
| `+++` | `BlockBreak` |
| `` ```lang `` (no braces) | `StaticCode` |
| `$$ … $$` | `Math` |
| `[^x]: …` | passthrough paragraph — footnotes are identical in both |
| `[Label]: url` | passthrough — link refs identical in both |

Directive options are read with `:label:` **as the canonical key**, with `:name:`
accepted as a legacy alias (fixes D2). Figure and table captions come from the
directive **body**, not the argument (fixes D3).

`FigureSource` disambiguates `:::{figure} ./img.png` (a `Path`) from
`:::{figure} #nb:analysis` (a `CellRef`) — the distinction the Python code
collapsed into a broken image link (fixes D11).

### QuartoReader

| Input | Produces |
|---|---|
| `` ```{lang} `` + `#\| k: v` | `CodeCell` |
| `::: {.callout-* title="…" collapse=…}` | `Admonition` |
| `::: {.panel-tabset}` | `TabSet`, splitting on `##` headings |
| `::: {.column-margin}` | `Margin` |
| `::: {#fig-x}` … `:::` | `Figure` (div form — multi-block captions) |
| `![alt](src){#fig-x width=…}` | `Figure` (inline form) |
| `$$ … $$ {#eq-x}` | `Math` with label |
| pipe rows + `: Caption {#tbl-x}` | `Table` |
| `{{< include _f.qmd >}}` | `Include` (fixes D13) |
| `{{< embed nb.ipynb#fig-x >}}` | `Embed` |
| `{{< video/pagebreak/meta/var … >}}` | `Unmappable` + reason |
| `` `r expr` `` | inline knitr eval, engine=`knitr` (fixes D14) |
| `` `{python} expr` `` / `` `{r} expr` `` | inline Jupyter eval |
| preservation marker + sidecar entry | the original `BlockKind`, restored (RT-11) |

### Notebook cell index (RT-03)

`:::{figure} #nb:analysis` names a label, not a file. Quarto's
`{{< embed path#cell >}}` requires the path. Before parsing documents, walk every
`.ipynb` and `.qmd` in the conversion set, read each cell's `#| label:`, and build
`label → (notebook_path, cell_index)`. `FigureSource::CellRef` is populated from
this index; an unresolved label becomes `Unmappable` rather than a guessed
filename. Phase 5 also writes to this index when relabelling cells.

### Inline scanning

Inline handling stays regex-based but becomes **single-pass and type-aware**
rather than a sequence of independent substitutions. The Python version applies
eight `re.sub` calls in order; each one can corrupt the output of the previous
(defect D15: `[@10.1038/nmeth.1974]` is destroyed because `[\w-]+` stops at the
`.`).

The replacement is one alternation matching all inline forms, dispatching on
which capture group fired:

- citation keys follow **Pandoc's actual rule** — internal punctuation allowed,
  trailing punctuation excluded — so `[@10.1038/nmeth.1974]` survives *and*
  `We used @numpy.` captures `numpy`, not `numpy.`. A naive `[\w.:/\-]*` fixes
  D15 while introducing a new trailing-punctuation defect; the current Python
  code happens to get the trailing case right, so this is a regression risk, not
  a free win. Both cases are corpus fixtures;
- `@` disambiguation checks the reference registry, not a prefix blacklist —
  `@fig-x` is a cross-ref because `fig-x` is a known label, not because it
  starts with `fig-`;
- **code spans are masked before inline transforms run.** Text inside
  `` `…` `` and fenced blocks is replaced with placeholders, transformed
  content is substituted, then placeholders restored. The Python version
  transforms role syntax inside code spans, corrupting documentation that
  *discusses* the syntax — which `article-template` does extensively.

## Related Code Files

- Create: `crates/mystquarto-core/src/reader/mod.rs`
- Create: `crates/mystquarto-core/src/reader/myst.rs`
- Create: `crates/mystquarto-core/src/reader/quarto.rs`
- Create: `crates/mystquarto-core/src/reader/fence.rs` — shared fence stack
- Create: `crates/mystquarto-core/src/reader/inline.rs` — single-pass scanner
- Create: `crates/mystquarto-core/src/reader/mask.rs` — code-span masking
- Read: `src/mystquarto/scanner.py` (fence semantics to preserve)
- Read: `docs/dialect-comparison.md` §2, §5, §7, §9, §10

## Implementation Steps

1. Port the fence stack: open/close matching, fence-count and indent rules.
   These are correct in the Python version; keep the semantics, change the
   output type.
2. Implement code-span masking and its tests **first** — later inline work is
   unsound without it.
3. Implement `inline.rs` as one alternation with group dispatch. Test the DOI
   key case (D15) and the `50% of users` case (a `%` that is not a comment).
4. `MystReader`: fences → typed blocks via `mappings.toml`; then `(label)=`,
   `%` comments, `+++`.
5. Implement `Target` attachment: a `Target` block binds to the next
   non-blank block; if none follows, it stays a standalone `Target` and the
   writer decides.
6. `QuartoReader`: divs, then shortcodes, then inline-code engines.
7. Detect the Quarto engine per document (knitr vs Jupyter) from cell languages
   and inline-code forms; record it on `Document` — Phase 5 needs it to decide
   the MyST `kernelspec`.
8. Implement the notebook cell index (RT-03) and the preservation-marker reader
   (RT-11), including a `Preserved` round-trip test.
9. Wire include resolution through Phase 3's path guard; add cycle and depth
   tests. **`article-template/` contains no includes**, so D13 needs its own
   fixtures — a traversal target, a two-file cycle, and a nested-in-list case.
10. Property test, **scoped**: for every corpus file *declared Stable*, reading
    then writing back in the **same** dialect is a no-op (`read(x) |> write == x`).

    The original phase required this on **every** corpus file, which is
    impossible by construction: Phase 4 accepts legacy roles and canonicalizes
    `:name:` → `:label:`, while Phase 5 emits modern MyST only, so any file
    containing `{cite}`, `{numref}`, `{ref}`, `{eq}`, `{doc}` or `:name:` cannot
    round-trip byte-identically — by accepted decision. Byte-identity applies to
    modern-MyST-only inputs; legacy-tagged files assert *normalized* equality
    instead. `blank_lines_before` (Phase 3 IR) is what makes even the Stable
    subset achievable.

## Success Criteria

Verified 2026-09-03 via `cargo test` and `cargo clippy --workspace --all-targets -- -D warnings`; targeted reader regression coverage is present in `crates/mystquarto-core/tests/readers.rs`.

- [x] Same-dialect round-trip is byte-identical on every **Stable-class** corpus
      file, and normalized-equal on legacy-tagged files (RT-13)
- [x] `article-template/article.md` parses with **zero** `Unmappable` blocks
      except `:::{figure} #nb:analysis`, which parses as `Figure { src: CellRef }`
- [x] `% comment` produces `Comment`, `50% of users` does not (D4)
- [x] `(sec:data-analysis)=` produces `Target` bound to the following heading (D5)
- [x] `:label:` and `:name:` both populate `Block.label`; `:label:` wins (D2)
- [x] Figure/table captions come from the directive body (D3)
- [x] `[@10.1038/nmeth.1974]` survives inline scanning intact **in the
      Quarto→MyST direction**, where the corruption actually occurs (D15) —
      the original criterion was direction-neutral and would have passed against
      the unfixed tool
- [x] `We used @numpy.` captures `numpy`, not `numpy.` — no trailing-punctuation
      regression from the D15 fix
- [x] A `Preserved` block reads back to its original `BlockKind`, byte-identical
      after re-emission (RT-11)
- [x] `#nb:analysis` resolves to `analysis.ipynb` via the notebook index; an
      unresolved label yields `Unmappable`, never a guessed filename (RT-03)
- [x] Include traversal, cycle, depth-cap and nested-in-list cases are refused
      with diagnostics; D13 has its own fixtures since the fixture has no includes
- [x] Role syntax inside code spans is not transformed
- [x] `{{< include >}}` and `{{< embed >}}` parse to typed blocks (D13)
- [x] `` `r expr` `` parses with `engine: knitr` recorded (D14)
- [x] Every block has a span that points at the right source line

## Risk Assessment

**Risk: `@` disambiguation needs labels that are not yet known.** Deciding
whether `@foo` is a citation or a cross-ref requires the label registry, but
labels appear throughout the document — and in *other* files.
*Signal:* forward references resolve wrongly (a `@fig-x` before `fig-x` is
defined).
*Response:* make it two-pass — collect all labels across the conversion set
first, then resolve inline references. Assume this is required; the single-pass
alternative only works for backward references and the corpus has forward ones.

**Risk: tab-set round-trip is ambiguous.** Quarto encodes tab labels as `##`
headings, so a real `##` heading inside a tabset is indistinguishable from a
tab boundary.
*Signal:* corpus round-trip differs on tabset content.
*Response:* document the ambiguity, treat every `##` at tabset top level as a
tab boundary (matching Quarto's own renderer), and warn when a tabset contains
deeper structure. Do not attempt to be cleverer than the renderer.

**Risk: masking breaks on nested/uneven backticks.** ``` ``code with ` inside`` ```
is legal CommonMark.
*Signal:* masking test failures on multi-backtick spans.
*Response:* match opening and closing runs by length, per CommonMark's code-span
rule. Test the 1-, 2-, and 3-backtick cases explicitly.
