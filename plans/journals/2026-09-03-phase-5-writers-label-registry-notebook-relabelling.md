---
title: "Phase 5: writers, label registry, notebook relabelling"
date: 2026-09-03
summary: "IR->text writers with a run-scoped label registry and notebook relabelling; found and fixed 1 critical + 5 high bugs in review, verified end-to-end via quarto render"
---

# Phase 5: writers, label registry, notebook relabelling

## What happened

Phase 5 of the mystquarto Rust port (plans/260903-1749-rust-port-dialect-fidelity/):
IR -> MyST/Quarto text writers, the run-scoped `LabelRegistry` (colon->hyphen
normalization, deterministic collision suffixing), the `.mystquarto/labels.json`
sidecar for round-trip label recovery, notebook cell relabelling so Quarto's
`{{< embed >}}` shortcode actually resolves, and the batch-pipeline wiring that
replaces Phase 3's `run_conversion` stub in the CLI orchestration layer.

Fixed defects D1 (dead cross-references), D2 (dropped figure labels), D3
(dropped table captions/labels), D4 (`%` comments rendered as literal text),
D5 (`(label)=` targets rendered as literal text), D11 (broken notebook-embed
links) — all verified against the real `article-template/` fixture through the
compiled binary and an actual `quarto render` pass: zero unresolved
cross-references, zero unresolved embeds. One real design mistake surfaced
during that empirical check: an invented two-anchor `{{< embed nb#id #id2 >}}`
syntax that Quarto silently ignores — `quarto render` reported "Unable to
resolve crossref" for exactly the id I assumed the second anchor would
register. Fixed by making the figure's own document-level label win as the
single embed id, falling back to the notebook cell's own name only when the
figure has none.

A mandatory code-reviewer pass (before declaring the phase done) found real
bugs the unit tests and the one-document empirical check couldn't reach:

- **Critical**: `mask_code_spans` did `bytes[i] as char` for non-backtick
  bytes — a Latin-1 reinterpretation that corrupts every multi-byte UTF-8
  character. Harmless in Phase 4 (detection-only use); became real data
  corruption the moment Phase 5's `rewrite_line` started reconstructing writer
  output from the masked string. `article-template/` is 100% ASCII, so nothing
  caught it until the review's reproduction (`Café — naïve` -> `CafÃ© â
  naÃ¯ve`).
- **High x5**: `-o` pointed at the input tree bypassed notebook-relabel/sidecar
  safety because the gate keyed off `--in-place` alone, not the actual path
  relationship; the label registry didn't prefer a reference's own file when
  resolving `@fig:x`, so a collision-suffixed label could resolve to the wrong
  figure from inside its own file; the collision-suffix counter could itself
  hand out an already-taken id; `MystWriter` for Quarto->MyST always got an
  empty `known_labels` list, so `rewrite_line` misclassified every inline
  `@id` as a citation and the sidecar-restore path for inline references was
  dead code in production; two documents embedding the same notebook cell
  under different labels silently let the second overwrite the first with no
  warning, leaving a dangling embed.
- **Medium**: notebook relabelling searched the whole file with a plain
  `text.find()`, risking a match inside an unrelated cell's echoed output
  before the real source line; delete-only-after-success and a stale doc
  comment needed reasserting; unescaped `"` in captions/titles could break out
  of Pandoc attribute syntax (a title could inject an arbitrary class).

Every finding got a concrete regression test reproducing the exact failure,
not just a fix. 209 tests passing (up from 128 before the review), clippy and
fmt clean, re-verified end-to-end against the real fixture after all fixes.

## Decision

Committed Phase 4 (readers, sitting uncommitted from an earlier session) and
Phase 5 together in one commit (b70bae9) — the user chose "one commit for
everything uncommitted" over splitting at the phase boundary, since neither
had been committed separately and both now pass together as one coherent,
tested unit.

Two narrower Medium findings from the review were deliberately deferred rather
than fixed inline: sidecar/notebook data can be built from files an aborted
`--in-place` batch never actually wrote (self-corrects via content-hash
staleness on the next run), and a phantom test that never exercises the
sidecar entry-count cap. Spawned as a tracked follow-up task rather than
silently dropped.

## Next steps

Phase 6 (config & frontmatter mapping — `myst.yml` <-> `_quarto.yml`,
`kernelspec` <-> `jupyter`, bibliography synthesis for the RT-14 citation gap)
is next per the plan's phase table. The follow-up task for the two deferred
Medium findings is separate and not blocking.

> Historical work record — not durable authority. Prefer docs/specs/ADRs for current decisions.
