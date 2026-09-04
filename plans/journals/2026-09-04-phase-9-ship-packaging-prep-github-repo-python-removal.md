---
title: "Phase 9 ship: packaging prep, GitHub repo, Python removal"
date: 2026-09-04
summary: Prepared release packaging without publishing; two real bugs only surfaced by actually running CI
---

# Phase 9 ship: packaging prep, GitHub repo, Python removal

## What happened

Continued `plans/260903-1749-rust-port-dialect-fidelity` from Phase 8 (verified
green: 341 Rust tests, 2 renderer tests, 225 legacy pytest, fmt/clippy clean)
into Phase 9 (Ship).

**Scope decision.** This working copy had no git remote and no relationship to
the plan's assumed upstream (`github.com/MaxGhenis/mystquarto`, live on PyPI
under a different account). No registry credentials were available or would
have been used to publish over someone else's package regardless. Resolved
with the user: new target repo `ntluong95/myst-quarto-rustCLI` (created via
`gh repo create`, `main` + `pre-rust-port` pushed), and the user runs the
actual `cargo publish`/`npm publish`/release-tag push themselves — this
session prepared everything short of that.

**Work done:** crate metadata (description/license/repository/keywords) on
both crates, bumped to 0.2.0; `mystquarto-core` pinned as a versioned path
dependency so `cargo publish` resolves correctly; `dist-workspace.toml` +
`.github/workflows/release.yml` generated via cargo-dist 0.32.0 for six
target triples with shell + npm installers, then every action SHA-pinned by
hand; README rewritten, CHANGELOG.md and docs/migration-from-python.md
added; Python source, legacy pytest suite, pyproject.toml, uv.lock, and
Python-only fixtures removed (pre-rust-port tag preserves the tree); CI's
Python job removed.

**Bug 1 — found by actually pushing to a real CI runner, not by trusting
local `cargo test`.** First CI run failed both jobs. Root cause: seven
`include_str!("../../../../article-template/...")` calls inside `#[cfg(test)]`
blocks across four source files plus `tests/readers.rs` reference
`article-template/`, which was `.gitignore`d and never actually committed —
it only worked locally because the directory happened to already exist on
disk. The repo's own `.gitignore` had documented this as a known, deferred
blocker from Phase 1 (a nested `.git` inside the vendored clone made `git
add` create a broken gitlink). A local permission hook blocks this agent
from touching any path containing `.git`, so the user ran
`rm -rf article-template/.git` themselves; the agent then vendored it as
plain tracked content (11 files, ~3.3MB) and re-ran CI to confirm both jobs
green on a fresh checkout.

**Bug 2 — found by validating the release config instead of trusting the
generator.** `dist plan` failed with "release.yml has out of date contents"
— cargo-dist's own drift check rejects hand-edits to its generated file,
which would have failed the release workflow's first job the moment a real
tag was pushed, silently reverting the required SHA-pinning. Fixed with
`allow-dirty = ["ci"]` in `dist-workspace.toml`. Re-ran `dist plan`
afterward: clean six-platform + npm artifact plan, no errors.

## Decision

Phase 9 is `in-progress`, not `done`. Packaging, docs, CI, and Python
removal are complete and verified against a real fresh-checkout CI run
(https://github.com/ntluong95/myst-quarto-rustCLI/actions/runs/33837591965).
Actual registry publish and the PyPI deprecation release are explicitly
deferred — the latter is unsatisfiable from this repo since it has no
relationship to the live PyPI `mystquarto` package. Both plan.md and
phase-09-ship.md were updated to reflect exactly this split rather than
being marked fully done.

## Next steps

User runs, in order: `cargo publish` (core, then CLI crate), `npm login` +
reserve the `mystquarto` name/`@mystquarto` scope, then
`git tag v0.2.0 && git push origin v0.2.0` to trigger the release workflow's
build matrix and GitHub Release. Exact commands are in phase-09-ship.md's
Implementation Notes.

> Historical work record — not durable authority. Prefer docs/specs/ADRs for current decisions.
