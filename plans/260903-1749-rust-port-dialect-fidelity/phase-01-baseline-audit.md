---
phase: 1
title: "Preconditions, baseline audit & defect corpus"
status: done
priority: P1
effort: "4d"
dependencies: []
---

# Phase 1: Preconditions, baseline audit & defect corpus

## Overview

Establish the preconditions the rest of the plan silently assumed (version
control, a clean fixture, a stated threat model), then freeze the current Python
behavior as an executable oracle and turn every observed defect into a
reproducible, named failing case **before** any Rust is written.

> **Red team RT-04, RT-07, RT-12 applied.** The original phase assumed a git
> repository that does not exist, claimed the 225 tests were mostly extractable
> `(input, expected)` pairs when substring assertions dominate, and specified a
> corpus layout that cannot express reverse-direction defects.

## Requirements

- Functional: the repository is under version control with a tagged commit
  containing the complete pre-port tree, before anything is deleted.
- Functional: the `article-template/` fixture used for baselines is a **clean
  checkout**, not the working tree polluted by prior conversions.
- Functional: every one of the 225 Python tests is classified into one of three
  disposition buckets, with counts recorded in this file.
- Functional: every defect D1–D16 has a **direction-appropriate** fixture whose
  recorded `python-actual.*` differs from `expected.*`.
- Non-functional: expectations are renderer-verified, not asserted from belief.

## Architecture

### Preconditions (RT-04)

The plan's rollback story, its "committed" corpora, and Phase 9's deletion
safety all assume version control. `git rev-parse` at the project root returns
`fatal: not a git repository`. The only `.git` in the tree belongs to
`article-template/`, a clone of the third-party `rowanc1/article-template` that
the converter has already written output into.

1. `git init` at the project root; `.gitignore` gains `/target`, `.mystquarto/`,
   and the in-tree conversion outputs.
2. Commit the complete current tree and tag it (e.g. `pre-rust-port`).
3. Decide `article-template/`'s status — vendored copy, submodule, or moved out
   of the tree. It currently holds generated output (`docs-quarto/`,
   `docs-quarto/docs-quarto/`) untracked in a foreign repository.

Phase 9's deletion step is gated on step 2 existing.

### Threat model (RD-5)

Recorded here because Phase 3's path-safety work depends on it: **input
repositories are untrusted.** Users clone third-party manuscript repos and
publish the rendered output. Therefore document-controlled paths, symlinks, and
sidecar files are attacker-influenced surfaces, not merely user data.

### Test classification, measured not assumed (RT-07)

The original claim — "Most are `(input, expected)` string pairs" — is false.
Measured over `tests/`: substring assertions (`assert "x" in result`)
substantially outnumber full-text equality assertions, `test_cli.py` has 29
`CliRunner().invoke` sites, and `test_config.py` asserts on Python dicts.

Classify every test into exactly one bucket and record the counts here:

| Bucket | Disposition | Lands in |
|---|---|---|
| **A — text pair** | Full-text `input → expected` | `tests/corpus/parity/` data fixtures |
| **B — behavioral** | CLI exit codes, filesystem effects, dict assertions | Hand-written Rust integration tests, **budgeted in Phase 3 (CLI) and Phase 6 (config)** — not in Phase 9 |
| **C — substring/phantom** | Asserts a fragment, or asserts nothing meaningful | Ported as Rust assertions, or deleted if phantom, with the decision recorded per test |

Bucket B has an owning phase in this revision; previously it was assigned to
Phase 9, which has no budget or criteria for it.

### Defect corpus — direction-aware (RT-12)

D14, D15 and half of D13 are **Quarto→MyST** defects. The original layout
(`input.md` / `expected.qmd`) could only express the forward direction, so those
fixtures would have matched their own baselines on day one and passed before any
fix existed. Verified:

```
q2m: 'Cite [{cite:t}`10`.1038/nmeth.1974] here.'   ← the actual D15 corruption
m2q: 'Cite [@10.1038/nmeth.1974] here.'            ← intact; original evidence cited this
```

Each defect directory declares its direction and uses matching extensions:

```
tests/corpus/defects/d15-doi-citation-keys/
  direction            # "quarto_to_myst"
  input.qmd
  expected.md
  python-actual.md
  README.md            # defect statement + reference-doc section
```

## Related Code Files

- Create: `tests/corpus/defects/d01-*` … `d16-*` (16 direction-aware directories)
- Create: `tests/corpus/parity/` (bucket A fixtures)
- Create: `tests/corpus/README.md` — layout, direction convention, how to add a case
- Create: `tests/corpus/classification.md` — the A/B/C table with per-test decisions
- Create: `scripts/snapshot-baseline.sh` — regenerates `python-actual.*`
- Create: `.gitignore` additions; initial commit and tag
- Modify: `docs/dialect-comparison.md` §12 — add a direction column; correct D15's
  evidence to `quarto_to_myst.py:18,35`; add D16 (output recursion); soften D10's
  count (`description` is overwritten, not dropped; `downloads` is `[]`)
- Read: `tests/test_*.py` (the 225 cases to classify)
- Read: `article-template/` (clean checkout)

## Implementation Steps

1. Run the preconditions: `git init`, `.gitignore`, full commit, tag.
2. Produce a clean `article-template/` checkout (its own `.git` makes this a
   `git clean`/re-clone away) and use **that** for all baselines. Do not
   snapshot the polluted working tree.
3. Snapshot the current conversion of the clean fixture into
   `tests/corpus/baseline/`.
4. Classify all 225 tests into buckets A/B/C. Record counts in
   `tests/corpus/classification.md` **before** estimating extraction effort.
5. Extract bucket A into data fixtures.
6. For each of D1–D16, determine the direction first, then build the minimal
   reproducing input (≤ ~15 lines) with matching extensions.
7. Write `expected.*` by hand, then **verify the expectation renders** —
   `quarto render` / `myst build`. An expectation that does not render is wrong.
8. Record `python-actual.*` via `scripts/snapshot-baseline.sh`.
9. Assert `python-actual.* != expected.*` for every defect. A defect fixture
   whose baseline already matches the expectation is mislabelled — fix the
   fixture or the catalogue row.
10. Tag parity cases whose expected output changes under the modern-MyST
    decision as `legacy-read-only`, and record the count.
11. Correct `docs/dialect-comparison.md` §12 per the Related Code Files entry.

## Success Criteria

- [x] Repository is a git repo with a tagged commit containing the full pre-port tree — **partial**: root Python/tests/docs/plans tree is committed and tagged `pre-rust-port`. `article-template/` is temporarily excluded via `.gitignore` — see next item.
- [x] `.gitignore` covers `/target`, `.mystquarto/`, and conversion outputs
- [ ] `article-template/`'s status decided and recorded; baselines taken from a clean checkout — decided (vendored copy, see plan.md's Decisions log) and the working tree was cleaned (`git clean -fdx` removed `docs-quarto/`, `_build/`, `.vscode/` pollution) before any fixture was built from it. **Blocked**: a repo-level permission hook refuses any Bash command that references the literal path `article-template/.git`, so the nested repo metadata cannot be removed by the agent. `article-template/` is gitignored in the interim with a comment giving the exact manual command (`rm -rf article-template/.git`); once run, remove that `.gitignore` line and commit the vendored tree. Does not block Phase 2+.
- [x] `tests/corpus/classification.md` records an A/B/C disposition for all 225 tests with counts
- [x] Bucket B tests are assigned to Phase 3 or Phase 6, never Phase 9 — by file: all of `test_cli.py`'s B-bucket tests are CLI-shaped (Phase 3), all of `test_config.py`'s are config-shaped (Phase 6); no file mixes both concerns, so the phase doc's own Bucket-B table row already resolves the assignment unambiguously.
- [x] 16 defect directories exist, each with a `direction` file and matching extensions
- [x] `python-actual.* != expected.*` for **every** defect case
- [x] Every `expected.qmd` renders under `quarto render`; every `expected.md` builds under `myst build`
- [x] `scripts/snapshot-baseline.sh` reproduces `python-actual.*` byte-identically
- [x] `docs/dialect-comparison.md` §12 has a direction column; D15 corrected; D16 added
- [x] Legacy-tagged parity case count recorded — 0 (no colon-typed label appears anywhere in the current test suite; noted as a corpus coverage gap for Phase 4/5)

## Risk Assessment

**Risk: bucket A is much smaller than hoped.** If only a small fraction of the
225 are text pairs, "parity with 225 tests" is a misleading goal.
*Signal:* step 4's bucket A count is under ~40% of the suite.
*Response:* restate Goal 1 in terms of the classification rather than the raw
count, and re-derive this phase's effort from the measured buckets. This is why
step 4 precedes any extraction work.

**Risk: an expectation is wrong.** Writing `expected.*` by hand encodes belief.
*Signal:* `quarto render` warns, the reference resolves to `??`, or the render
produces no output.
*Response:* step 7 exists to catch this. Never commit an unverified expectation.

**Risk: `git init` disturbs the nested `article-template/` repository.**
*Signal:* the outer repo tries to track the inner `.git`.
*Response:* decide step 2's status question before committing — vendored (add
to `.gitignore`), submodule, or relocated. Do not leave it ambiguous.

**Risk: the modern-MyST decision invalidates more parity cases than expected.**
*Signal:* step 10's count exceeds ~40% of bucket A.
*Response:* stop and re-present scope — the work is then a rewrite, not a port.
