---
phase: 8
title: "Test corpus & renderer-backed validation"
status: pending
priority: P1
effort: "5d"
dependencies: [7]
---

# Phase 8: Test corpus & renderer-backed validation

## Overview

Prove the port. Run the Phase 1 corpus against the Rust implementation, and —
the part no amount of unit testing substitutes for — feed the converted output
to the **real renderers** and require them to succeed with resolved references.

`quarto 1.9.36` and `myst v1.10.1` are already installed on this machine, so
this validation is available immediately rather than being aspirational.

> **Red team RT-03, RT-10, RT-14, RT-15 applied.** The original phase asserted
> `quarto render` exits 0 while Phase 5 expected it to fail; used a `?@` grep
> that is structurally blind to unresolved citations; required `--strict` to
> exit 1 on a *correct* conversion; and validated D13 against a fixture that
> contains no includes. Effort raised 4d → 5d.

## Requirements

- Functional: all Phase 1 parity cases pass against the Rust binary.
- Functional: all 15 defect cases pass — each output now matches `expected.*`
  and differs from the recorded `python-actual.*`.
- Functional: `article-template/` → Quarto renders under `quarto render` with
  zero unresolved cross-references.
- Functional: that Quarto output → MyST builds under `myst build`.
- Non-functional: the renderer tests are gated behind a feature flag so
  `cargo test` works on machines without Quarto/MyST installed.

## Architecture

### Four test tiers

| Tier | Runs | Gate | Answers |
|---|---|---|---|
| Unit | always | `cargo test` | Is each rule right? |
| Corpus | always | `cargo test` | Does the whole document convert right? |
| Round-trip | always | `cargo test` | Is information preserved? |
| **Renderer** | opt-in | `cargo test --features renderer-tests` | Does the output actually work? |

Renderer tests are opt-in because they need external binaries; they run in CI on
a job that installs both, and locally on demand. They are the tier that catches
what the others structurally cannot — that an expectation was wrong.

### Reference-resolution checking

Exit status alone is insufficient, and so is the `?@` grep the original phase
specified. Three distinct failure modes, each needing its own check:

| Failure | Quarto's behavior | Check |
|---|---|---|
| Unresolved **cross-reference** | renders `?@fig-x`, exits 0 | grep HTML for `?@` |
| Unresolved **citation** | renders `key?` or literal `@key`, exits 0, Citeproc warning | grep HTML for the literal-key pattern **and** fail on Citeproc "not found" (RT-14) |
| Bad **embed target** | errors, **produces no HTML** | assert the HTML file exists |
| Preserved content leaking as markup | renders it live, exits 0 | assert no executable markup in HTML (RT-02) |

The citation check is not optional polish. `myst.yml` has no `bibliography` key
— MyST auto-loads `references.bib` and resolves DOIs over the network, Quarto
does neither. Without Phase 6's synthesis and this check, every citation in the
document dies while all four original criteria stay green: exactly defect D12
reproduced inside the test that exists to prevent it.

`scripts/check-refs.sh` performs all four. The MyST equivalent parses
`myst build`'s warning output for unresolved references.

### Round-trip classes

Not every construct round-trips byte-identically, and pretending otherwise
produces either false failures or weakened tests. Three declared classes:

| Class | Guarantee | Example |
|---|---|---|
| **Stable** | byte-identical MyST→Quarto→MyST | headings, paragraphs, citations, math, modern-MyST-only files |
| **Normalized** | identical after re-normalization | whitespace in directive options, attribute order |
| **Lossy** | documented divergence + a diagnostic | `{danger}`→`{important}`, `%` comment paragraph splitting, `hide-input`→`code-fold` |

Code cells move from Stable to Lossy: reference §2 marks the
`hide-input` → `code-fold` mapping ⚠️, so classing them Stable contradicted the
reference doc. Legacy-role files are Normalized, never Stable — Phase 5's
modern-only writer makes byte-identity impossible for them by design.

Each corpus file declares its class in a header comment. A file in the Stable
class that diverges is a bug; a file in the Lossy class that *doesn't* warn is
also a bug. Both are asserted.

## Related Code Files

- Create: `crates/mystquarto-core/tests/corpus.rs` — data-driven runner
- Create: `crates/mystquarto-core/tests/roundtrip.rs`
- Create: `crates/mystquarto-cli/tests/cli.rs` — flag behavior, exit codes
- Create: `crates/mystquarto-cli/tests/renderer.rs` — feature-gated
- Create: `scripts/check-refs.sh` — the `?@` grep check
- Modify: `tests/corpus/**` — add round-trip class headers
- Modify: `.github/workflows/ci.yml` — add the renderer job
- Read: `docs/dialect-comparison.md` (audited against behavior in step 7)

## Implementation Steps

1. Write the data-driven corpus runner: walk `tests/corpus/`, convert, compare
   to `expected.*`, report with `similar` diffs.
2. Run the parity tier. Expect failures; triage each as (a) a real port bug or
   (b) an intentional change from the modern-MyST decision. Update the expected
   output only for (b), and only with the Phase 1 `legacy-read-only` tag as
   justification.
3. Run the defect tier. Each must now match `expected.*`.
4. Implement the round-trip classes and their assertions.
5. Implement the renderer tier: convert `article-template/`, `quarto render`,
   run `check-refs.sh`; then reverse-convert and `myst build`.
6. Add the CI job installing Quarto and MyST.
7. **Audit `docs/dialect-comparison.md` against actual behavior.** Every row
   claiming ✅ must have a passing corpus case; every ⚠️/❌ row must have a case
   producing the expected diagnostic. Correct the doc where reality differs —
   the doc is the contract, and an inaccurate contract is worse than none.
8. Measure coverage; identify constructs in reference §2 with no corpus case and
   add minimal cases for them.

## Success Criteria

- [ ] All bucket A parity cases pass, or are documented as intentional
      modern-MyST changes; bucket B tests pass in their owning phases
- [ ] All 16 defect cases pass, each in its declared direction
- [ ] `quarto render` on converted `article-template/` exits 0 **and produces HTML**
- [ ] `check-refs.sh` finds zero `?@` markers, zero unresolved citations, and
      zero Citeproc "not found" warnings (RT-14)
- [ ] No executable markup from preserved content appears in the rendered HTML (RT-02)
- [ ] `myst build` on the reverse conversion exits 0 with no unresolved-reference
      warnings
- [ ] `--strict` exits **0** on the correct conversion; `--strict=all` exits 1 (RT-10)
- [ ] Path-safety suite passes end to end: symlink escape, `..` include traversal,
      include cycle, depth cap, output-inside-input, hostile sidecar
- [ ] Converting `article-template/` twice produces byte-identical trees with no nesting
- [ ] D13 has its own include fixtures — the article-template fixture has none (RT-15)
- [ ] Every corpus file declares a round-trip class and honors it
- [ ] Every Lossy-class file produces at least one diagnostic
- [ ] Every reference §2 construct has at least one corpus case
- [ ] `cargo test` passes without Quarto/MyST installed
- [ ] `cargo test --features renderer-tests` passes with both installed
- [ ] Reference doc audited; discrepancies corrected

## Risk Assessment

**Risk: the renderer rejects output for reasons unrelated to conversion.**
Missing LaTeX, absent Python packages, or notebook execution failures will fail
`quarto render` even when the Markdown is perfect.
*Signal:* render failures whose errors mention kernels, LaTeX, or packages.
*Response:* render to **HTML only** and disable execution (`--no-execute` /
`execute: false`). The question under test is whether the *markup* is valid, not
whether the science runs. Explicitly out of scope: PDF, which needs LaTeX that
is not installed — the same gap that ended the previous session.

**Risk: `myst build` in CI needs network access.** `article.md` cites two DOI
keys absent from `references.bib`; MyST resolves them over the network, as
`article-template/_build/cache/doi-*.csl.json` shows.
*Signal:* CI failures mentioning Crossref, DNS, or rate limits.
*Response:* commit the two cached CSL-JSON responses as fixtures and point the
CI run at the cache, so the renderer tier is hermetic. Do not make CI depend on
a third-party API's availability.

**Risk: many parity cases fail and triage becomes the whole phase.** If Phase 1
step 6 found a large legacy-tagged set, this is expected volume, not surprise.
*Signal:* triage exceeds ~2 days.
*Response:* Phase 1's risk gate should have caught this. If it reaches here
anyway, batch the triage by construct rather than by case — failures will
cluster on a handful of transforms.

**Risk: the reference doc is wrong somewhere and tests were written to match
it.** Then tests pass and the tool is still wrong.
*Signal:* the renderer tier disagrees with a green corpus test.
*Response:* the renderer is authoritative over both the doc and the corpus.
Fix the expectation and the doc row together, and note it in step 7's audit.
