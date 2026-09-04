---
title: Phase 08 test corpus and renderer validation
date: 2026-09-04
summary: "Completed Phase 08 test corpus, round-trip harness, path-safety assertions, and renderer-backed validation with hermetic CSL cache and bug fixes"
---

# Phase 08 test corpus and renderer validation

Completed Phase 08 test corpus, round-trip harness, path-safety assertions, and renderer-backed validation with hermetic CSL cache and bug fixes.

## Key Accomplishments & Fixes
- **Corpus & Round-Trip Runners**: Implemented data-driven test runners (`crates/mystquarto-core/tests/corpus.rs` and `roundtrip.rs`) exercising 34 parity cases, 16 defect fixtures (D01-D16), and new construct fixtures under `tests/corpus/constructs/`. Verified all three round-trip classes (`Stable`, `Normalized`, and `Lossy`).
- **Path-Safety Gating**: Path-guard refusals (directory traversal, include cycles, depth exceeded) now emit `MQ0605` with `Severity::Warning`, properly failing under `--strict`. Include targets in subdirectories remain relative to source documents rather than project root.
- **Renderer-Backed Verification**: `crates/mystquarto/tests/renderer.rs` executes real `quarto render` and `myst build article.md --md --force`. Pre-seeded offline CSL-JSON cache (`tests/fixtures/csl_cache/`) eliminates live network dependencies on Crossref.
- **Table and Figure Fidelity**: Fixed Quarto reader table parser to tolerate blank lines between pipe rows and `: Caption {#tbl-...}`, preserving table cross-references. Filtered out `:id:` and duplicate `:alt:` options when writing MyST figure directives.
- **Bibliography & Asset Isolation**: Quarto build artifact directory `_manuscript` is excluded from discovery and asset walks. DOI bibliography supplement is generated even when no `.bib` exists, and synthesized `.mystquarto/doi-references.bib` is stripped on reverse conversion. RT-02 checks in `scripts/check-refs.sh` verify no leaked preservation markup in rendered HTML.
- **Verification**: `cargo test --workspace` (241 passed), CLI tests (50 passed), renderer tests (2 passed), `cargo clippy` clean with warnings denied, `cargo fmt` clean, and `uv run pytest tests/ -q` (225 passed).

> Historical work record — not durable authority. Prefer docs/specs/ADRs for current decisions.
