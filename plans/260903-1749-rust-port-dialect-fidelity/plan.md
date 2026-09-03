---
title: "rust-port-dialect-fidelity"
description: "Rewrite mystquarto in Rust around a typed document IR, targeting modern mystmd v1, fixing 16 verified conversion defects and an unscoped file-orchestration layer"
status: in-progress
priority: P1
effort: "8w"
tags: [rust, port, myst, quarto, converter]
created: 2026-09-03
blockedBy: []
blocks: []
---

# rust-port-dialect-fidelity

## Overview

Plan status: `in-progress`. Completed phases: 5/9.

Port `mystquarto` from Python to Rust, replacing the regex line-scanner with a
typed document IR, and fixing the 16 verified defects catalogued in
[docs/dialect-comparison.md §12](../../docs/dialect-comparison.md).

The port is not the whole point. The current implementation targets **legacy
MyST** (Sphinx roles: `` {cite}`key` ``, `` {numref}` ``, `:name:`) while the
real-world fixture in `article-template/` is **modern mystmd v1** (`@fig:samples`,
`:label:`, `(sec:x)=`, `%` comments). That mismatch — not Python — is why every
cross-reference in the converted output is dead, why `_quarto.yml` contains
`format: {}` and a nonexistent `analysis.ipynb.qmd`, and why the CLI reported
success on a broken conversion.

Rewriting in Rust without fixing the model would produce a faster broken tool.
So the plan does both: the IR redesign carries the correctness fixes, and Rust
carries the distribution and performance win.

### Accepted decisions

| Decision | Choice | Consequence |
|---|---|---|
| MyST dialect | **Modern mystmd v1 only** | Legacy accepted on read, never emitted. Single writer vocabulary |
| Label handling | **Normalize + sidecar map** | `fig:samples` → `fig-samples`; `.mystquarto/labels.json` restores originals on reverse |
| Lossy constructs | **Best-effort + warn + preserve original in a sidecar** | Nothing is destroyed. Preservation moved out of HTML comments — see Red Team finding RT-02 |
| Distribution | **crates.io + npm (npx) + GitHub release binaries** | Drops the PyPI/`uvx` path — a documented breaking change, preceded by a final deprecation release |
| Python source | **Delete once Rust reaches parity** | Kept in-tree as the porting oracle until Phase 9, gated on version control existing |

### Decisions taken during red-team remediation

| # | Question | Decision | Rationale |
|---|---|---|---|
| RD-1 | YAML crate | **No general YAML round-trip.** Read with a safe parser; write with a purpose-built deterministic emitter for the known key set | `saphyr 0.0.12` panics on block scalars (`emitter.rs:241-242 todo!()`) and has no comment API. The claim in the original Phase 3 was false |
| RD-2 | Preservation channel | **Sidecar file** `.mystquarto/preserved.json`, not HTML comments | HTML comments terminate at a blank line in Pandoc; multi-paragraph preserved source injects live markup into rendered output (reproduced) |
| RD-3 | Notebook embeds | **In scope: rewrite `#\| label:` in notebooks inside the conversion set** | Without it `{{< embed >}}` targets do not exist and `quarto render` produces no output, making Goal 4 unreachable. Closes old open question 1 |
| RD-4 | `--strict` semantics | **Three severity classes**; `Lossy-expected` excluded from `--strict` by default | Config-preservation warnings fire on every real MyST project; a gate that is red on correct output is not a gate |
| RD-5 | Threat model | **Untrusted input repositories are in scope** | Users clone third-party manuscript repos (`article-template/` is itself a clone) and publish the rendered output |

### Non-goals

- Rendering. The tool converts source; `myst build` / `quarto render` render it.
- LaTeX/PDF toolchain installation. PDF export failures are environment issues.
- Preserving the Python library API — no documented consumers exist.
- A general Markdown AST library. The IR models only what the two dialects need.
- A general YAML round-trip library (see RD-1).

## Goals

| # | Goal | Priority |
|---|------|----------|
| 1 | Rust binary at behavioral parity with the Python tool, measured per Phase 1's test classification | P1 |
| 2 | All 16 catalogued defects fixed and regression-tested | P1 |
| 3 | Every lossy conversion emits a `file:line` diagnostic; `--strict` gates CI without firing on correct output | P1 |
| 4 | `article-template/` converts to Quarto that **actually renders** via `quarto render`, with cross-references **and citations** resolved | P1 |
| 5 | No conversion can read or write outside its declared roots, follow a symlink out of the input tree, or recurse into its own output | P1 |
| 6 | Round-trip MyST → Quarto → MyST is semantically stable on the corpus | P2 |
| 7 | Published to crates.io + npm with prebuilt binaries for macOS/Linux/Windows | P2 |
| 8 | Python implementation removed after a final deprecation release; docs reflect Rust reality | P2 |

## Phases

| # | Phase | Status | Depends on |
|---|-------|--------|-----------|
| 1 | [Preconditions, baseline audit & defect corpus](./phase-01-baseline-audit.md) | Done | — |
| 2 | [Dialect reference & conversion contract](./phase-02-dialect-reference.md) | Done | 1 |
| 3 | [Rust core: workspace, IR, YAML, orchestration contract](./phase-03-rust-core.md) | Done | 2 |
| 4 | [Readers: MyST + Quarto → IR](./phase-04-readers.md) | Done | 3 |
| 5 | [Writers: IR → MyST + Quarto, label normalization](./phase-05-writers.md) | Done | 4 |
| 6 | [Config & frontmatter mapping](./phase-06-config-frontmatter.md) | Pending | 3, 4 |
| 7 | [Diagnostics & lossy preservation](./phase-07-diagnostics.md) | Pending | 5, 6 |
| 8 | [Test corpus & renderer-backed validation](./phase-08-test-corpus.md) | Pending | 7 |
| 9 | [Ship: packaging, CI, docs, Python removal](./phase-09-ship.md) | Pending | 8 |

**Sequencing is strictly serial.** The original plan claimed Phases 4/5 and 6
could run in parallel; they cannot. Phase 6 needs the engine detection Phase 4
records on `Document` and the label machinery Phase 5 builds, and Phases 2, 4, 5,
6 and 7 all write `mappings.toml`.

**Effort: 41 person-days ≈ 8.2 weeks, one owner.** Phase efforts are
4+2+7+6+6+4+4+5+3. The original plan stated 3-4 weeks against a 32-day sum — a
40% understatement, which red-team review flagged and which the remediation
raised further (assets, path safety, atomicity, notebook relabelling, and the
citation gate were all previously unbudgeted). If the timeline is fixed rather
than the scope, cut here and say so explicitly:

| Cut | Saves | Cost |
|---|---|---|
| Phase 2's `mappings.toml` pipeline (its own gate may already reject it) | ~2d | Mappings live as `match` arms; the reference doc stays documentation-only |
| Phase 9's npm channel | ~1d | crates.io + release binaries only |
| Round-trip Stable-class byte-identity (Goal 6) | ~2d | Conversion still correct; round-trip merely normalized |

Phases 1, 3, 5, 7 and 8 are not compressible without reintroducing a defect
class this plan exists to remove.

## Architecture

```
                    ┌──────────────────┐
   .md (MyST)  ───► │   MystReader     │ ─┐
                    └──────────────────┘  │
                                          ├──►  ┌─────────┐  ──┬─► MystWriter  ──► .md
                    ┌──────────────────┐  │     │ DocIR   │    │
  .qmd (Quarto) ──► │  QuartoReader    │ ─┘     │ + spans │    └─► QuartoWriter ──► .qmd
                    └──────────────────┘        └─────────┘
                                                     │
                                    ┌────────────────┼────────────────┐
                                    │                │                │
                            ┌───────────────┐ ┌─────────────┐ ┌──────────────┐
                            │ LabelRegistry │ │ Diagnostics │ │ PathGuard    │
                            │ (run-scoped)  │ │ file:line   │ │ containment  │
                            └───────────────┘ └─────────────┘ └──────────────┘
                                    │
                          .mystquarto/labels.json
                          .mystquarto/preserved.json
```

Crate layout (Cargo workspace):

| Crate | Responsibility |
|---|---|
| `mystquarto-core` | IR, readers, writers, label registry, diagnostics, config/frontmatter, path guard |
| `mystquarto` | `clap` CLI, file discovery, orchestration, reporting. **This is the published binary crate** — `cargo install mystquarto` resolves the package name, not a binary name |

Binaries: `mystquarto`, `myst2quarto`, `quarto2myst`, all shipped from the
`mystquarto` crate.

### Why an IR instead of a direct port

The Python design transforms text→text in one pass, so it cannot answer
"what kind of thing is this label attached to?" — which is exactly what
correct label normalization requires (a figure's label needs `fig-`, a table's
needs `tbl-`). Defects D1, D2, D3, D11 and D15 all trace to that missing type
information. The IR makes construct type available at write time.

## Success Criteria

- [ ] `cargo test` green; the Phase 1 test classification is satisfied in full
      (data-fixture cases as corpus, CLI/config cases as Rust integration tests)
- [ ] All 16 defects in `docs/dialect-comparison.md` §12 have a named regression
      test whose recorded `python-actual.*` **differs** from `expected.*`
- [ ] `myst2quarto article-template/ -o /tmp/q` produces a tree where
      `quarto render` exits 0, produces HTML, and has zero unresolved
      cross-references (`?@`) **and zero unresolved citations**
- [ ] `quarto2myst` on that output, then `myst build`, exits 0
- [ ] Round-trip MyST→Quarto→MyST is byte-identical on the declared Stable class
      when the sidecar map is present
- [ ] `--strict` exits 0 on a correct conversion of `article-template/` and
      non-zero when a genuine information loss occurs
- [ ] No conversion completes silently when information was dropped
- [ ] Path-safety suite passes: symlink escape, `..` include traversal, include
      cycle, output-inside-input, hostile sidecar — all refused with diagnostics
- [ ] Running any conversion twice produces byte-identical trees and no nesting
- [ ] `--dry-run` writes zero bytes for every flag combination, verified by a
      recursive tree hash
- [ ] `cargo install mystquarto` and `npx mystquarto` both work from a clean machine
- [ ] `src/mystquarto/` and `tests/*.py` removed **after** a tagged commit and a
      final PyPI deprecation release
- [ ] `docs/dialect-comparison.md` matches implemented behavior (audited Phase 8)

## Open Questions

1. **`.ipynb` cell-level conversion.** RD-3 puts notebook *label rewriting* in
   scope. Full cell-level conversion of notebook content is still out of scope —
   notebooks are copied with labels patched. Confirm this is sufficient.
2. **npm package name and scope.** `mystquarto` and `@mystquarto/*` are both
   currently unclaimed on npm; the platform scopes must be reserved in Phase 9
   step 1 or the `optionalDependencies` install path is open to takeover.
3. **Upstream repository.** `pyproject.toml` points at
   `github.com/MaxGhenis/mystquarto`, to which this working copy has no
   connection. Phase 9 publishing assumes rights there.

## Red Team Review

### Session — 2026-09-03
**Findings:** 16 (16 accepted, 0 rejected)
**Severity breakdown:** 6 Critical, 10 High
**Reviewers:** Assumption Destroyer (Scope Auditor), Failure Mode Analyst
(Flow Tracer), Scope & Complexity Critic (Contract Verifier), Security
Adversary (Fact Checker). Verification tier: Full.

Six findings were independently reproduced by the controller before acceptance:
saphyr's block-scalar panic, HTML-comment markup injection, the embed render
failure, D15's direction, symlink dereference, and PyPI's live status.

| # | Finding | Severity | Disposition | Applied To |
|---|---------|----------|-------------|------------|
| RT-01 | `saphyr` preserves neither comments nor block scalars; emitter panics on `Literal`/`Folded`. Phase 3 rationale false; Phase 6's D10 channel unimplementable | Critical | Accept | Phase 3, 6 |
| RT-02 | Preserve-as-HTML-comment injects live markup — Pandoc ends raw HTML at a blank line, not `-->` | Critical | Accept | Phase 7, 4, 5 |
| RT-03 | Notebook-embed policy makes `quarto render` produce no output; Phase 5 and Phase 8 assert opposite outcomes | Critical | Accept | Phase 5, 8 |
| RT-04 | No version control at repo root — Phase 9's "removal is recoverable" is false. PyPI package is live with 3 releases; deprecation unscheduled and mis-ordered | Critical | Accept | Phase 1, 9 |
| RT-05 | Asset copying unowned in the plan; stale assets never refreshed; output-inside-input recursion already on disk; symlinks dereferenced into published output | Critical | Accept | Phase 3 |
| RT-06 | `--in-place` silently deletes sources and clobbers hand-authored config; no atomicity, no partial-failure recovery, `--dry-run` ungated | Critical | Accept | Phase 3 |
| RT-07 | Phase 1's "most cases are (input, expected) pairs" is false; substring assertions dominate and CLI/config tests are unextractable | High | Accept | Phase 1 |
| RT-08 | `LabelRegistry` specified doc-scoped and run-scoped in different phases; flat sidecar cannot represent either; "output root" undefined for `--in-place`/single-file | High | Accept | Phase 5 |
| RT-09 | Sidecar read from untrusted input with no version, size, shape, or direction validation; a single-file run destroys the project-wide map | High | Accept | Phase 5 |
| RT-10 | `--strict` cannot gate CI as specified — config-preservation warnings fire on every real MyST project | High | Accept | Phase 7 |
| RT-11 | Preservation had no reader — "preserved originals round-trip" was unachievable and landed in no phase | High | Accept | Phase 3, 4, 7 |
| RT-12 | D15 catalogued in the wrong direction; corpus layout cannot express reverse-direction defects, so D13/D14/D15 fixtures would pass on day one | High | Accept | Phase 1, docs §12 |
| RT-13 | Phase 4's byte-identical same-dialect round-trip is unsatisfiable against a modern-only writer; IR has no inter-block whitespace | High | Accept | Phase 4 |
| RT-14 | `bibliography` appears nowhere in the plan; the `?@` gate is structurally blind to unresolved citations | High | Accept | Phase 6, 8 |
| RT-15 | Includes have no path containment, cycle detection, or depth bound; `article-template/` contains no includes so D13 is untested | High | Accept | Phase 3, 4, 8 |
| RT-16 | `cargo install mystquarto` names a crate the workspace never creates; npm platform scopes unreserved; no provenance, floating action tags, no `permissions:` block | High | Accept | Phase 3, 9 |

Rolled into the above rather than listed separately: Phase 6's false
`dependencies: [3]`; `mappings.toml`'s fabricated "already needs a TOML parser"
justification and its self-contradicting coverage criterion; `--no-preserve`
contradicting the never-destroy decision; the compile-time-constant "one-line
change" claim; the Python-dependency-in-`cargo test` discovery parity criterion;
the 3-4w effort estimate against a 32-day phase sum.

### Whole-Plan Consistency Sweep
- Files reread: plan.md, phase-01 … phase-09, docs/dialect-comparison.md
- Decision deltas checked: 21 (16 findings + 5 RD decisions)
- Reconciled stale references: see each phase's own edits
- Unresolved contradictions: 0
