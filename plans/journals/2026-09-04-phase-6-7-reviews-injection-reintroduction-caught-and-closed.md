---
title: "Phase 6 & 7 reviews: injection reintroduction caught and closed"
date: 2026-09-04
summary: "Two adversarial reviews this session caught 2 Critical+8 High (Phase 6) and 3 Critical+3 High (Phase 7) bugs before ship, including a reintroduction of the RT-02 markup-injection class."
---

# Phase 6 & 7 reviews: injection reintroduction caught and closed

## What happened

### Phase 6 review (config & frontmatter mapping)
Reviewed the already-implemented Phase 6 code. Found and fixed:
- **Critical**: `yaml/emit.rs`'s scalar quoting didn't escape embedded newlines — any multi-line config value (a folded `description: >`, a multi-paragraph abstract) produced YAML `quarto render` couldn't parse.
- **Critical**: `yaml/surgery.rs`'s frontmatter-editing segmenter treated a blank line *inside* a block scalar as the top-level key separator, silently truncating multi-paragraph `abstract:` content on any unrelated key edit (e.g. `kernelspec` → `jupyter`).
- 8 High findings: stale `.mystquarto/preserved.json` reverting user edits (asset-copy ordering bug), unknown/future config keys silently dropped both directions, manuscript-toc/book-part/appendices entries dropped on reverse conversion, a bare-string author entry silently discarded, frontmatter-mapping warnings never surfacing to the user.

Root cause pattern across all of these: correctness gaps the 262 existing green tests happened not to exercise — same class of issue Phase 5's review had already found once.

### Phase 7 implementation + review (diagnostics & lossy preservation)
Built from scratch: a unified `Diagnostic`/`Severity` system replacing four separate ad hoc warning types; a block-content preservation sidecar (`.mystquarto/preserved.json`, schema-unified with Phase 6's config-field sidecar after discovering they'd otherwise collide on the same file); `--strict`/`--strict=all` promotion; `.mystquarto/suppress.toml` code baselining; a human diagnostic renderer.

A second adversarial review then found:
- **Critical**: `--in-place` could permanently delete a preserved construct with zero recovery — the "output writes into input tree, refuse without `--force`" gate blocked the *sidecar write* but not the *source deletion* that follows it.
- **Critical**: content restored from the sidecar during a reverse conversion could be reparsed through the *wrong* dialect's parser, silently changing meaning and — for a body containing its own code fence — letting trailing lines escape as live, unescaped markup. This is exactly the injection class (RT-02) this phase exists to close, reopened through a path the original regression test didn't cover.
- **Critical**: a crafted directive name could hijack which sidecar entry an unrelated marker resolved to, substituting one preserved block's content for another's.
- 3 High findings: a CLI parsing break on `--strict <path>` (clap greedily consuming the positional); the single most common preservation disposition never actually emitting a diagnostic, so `--strict=all` passed "by accident"; `--strict` correctly failing on `article-template/`'s two genuinely-missing bibliography citations, contradicting the phase's own stated success criterion.

## Decision

- Fixed all Criticals and Highs in both phases; deferred Mediums/Lows as documented follow-ups in each phase's own doc and `docs/diagnostics.md` — user-scoped decision, made explicitly rather than silently.
- C2 (cross-dialect reparse) fixed via a schema change: every preservation-sidecar entry now records which dialect (`Myst`/`Quarto`/`Unknown`) its content was captured in, and a reader refuses to ever reparse an entry recorded as foreign to itself — closing the injection path structurally rather than patching the specific reparse heuristic that let it through.
- H3 (`--strict` failing on `article-template/`) resolved as an accepted, documented deviation: the fixture's two DOI citations genuinely are missing from `references.bib`; the diagnostic is correct, the phase spec's assumption about the fixture's own bibliography completeness was not.
- Both phases verified against a real `quarto render` (not just `cargo test`), including reproducing the injection scenario end-to-end against the actual renderer both before and after the fix.

## Next steps

- Phase 8 (test corpus & renderer-backed validation) and Phase 9 (ship: packaging, CI, docs, Python removal) remain.
- Deferred Medium/Low items from both reviews are documented inline (`phase-06-config-frontmatter.md`, `phase-07-diagnostics.md`, `docs/diagnostics.md`) and worth a follow-up pass before Phase 9 ships, particularly: `find_bib_file` symlink-following (Phase 6), and `suppress.toml`'s single-line-only parser plus its single-file-mode path bug (Phase 7).

> Historical work record — not durable authority. Prefer docs/specs/ADRs for current decisions.
