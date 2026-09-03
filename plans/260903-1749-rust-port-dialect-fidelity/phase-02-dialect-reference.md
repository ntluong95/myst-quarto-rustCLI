---
phase: 2
title: "Dialect reference & conversion contract"
status: done
priority: P1
effort: "2d"
dependencies: [1]
---

# Phase 2: Dialect reference & conversion contract

## Overview

Turn `docs/dialect-comparison.md` from prose into a **machine-readable contract**
that the implementation reads and the tests assert against. The reference doc
already exists (written during planning); this phase makes it authoritative
rather than decorative.

> **Red team applied.** Step 0 below runs this phase's own abort gate *before*
> building the pipeline, rather than after. The original justification "the
> project already needs a TOML parser for Cargo metadata" was fabricated —
> Cargo parses `Cargo.toml` at build time; the binary needs a `toml` dependency
> that the Phase 3 table did not list. The coverage criterion contradicted this
> phase's own risk response and has been restated to something achievable.

## Requirements

- Functional: mapping tables in §2, §3.4, §5, §8.2, §8.3 and §10 exist as a
  structured data file the Rust build consumes.
- Functional: a doc-sync test fails when the data file and the Markdown tables
  disagree, so the reference cannot silently rot.
- Non-functional: adding a new construct mapping requires editing one place.

## Architecture

A single `mappings.toml` at the repo root, embedded at compile time via
`include_str!` and parsed into static tables. TOML over JSON/YAML because it
tolerates comments — each row can cite its reference-doc section. This requires
adding the `toml` crate to Phase 3's dependency table; it is not free.

**`mappings.toml` is a compile-time constant.** Correcting a mapping is a code
change plus a release across crates.io, npm, and the binary matrix — not, as the
original Phase 6 claimed, "a one-line change." Phases that depend on cheap
correction must be written accordingly.

```toml
[[directive]]
myst = "note"
quarto_class = "callout-note"
fidelity = "exact"
ref = "§2"

[[directive]]
myst = "danger"
quarto_class = "callout-important"
fidelity = "lossy"
note = "no Quarto danger callout; collapses to important"
ref = "§2"

[[label_prefix]]
myst = "tab:"
quarto = "tbl-"
ref = "§3.4"

[[role]]
myst = "cite"
legacy = true
quarto = "[@{}]"
ref = "§10"
```

Fidelity is a three-state enum — `exact` / `lossy` / `unmappable` — and is what
Phase 7 reads to decide whether to emit a diagnostic. Encoding it as data means
"which conversions warn?" is answerable by reading one file.

## Related Code Files

- Create: `mappings.toml` — the conversion contract
- Create: `crates/mystquarto-core/src/mappings.rs` — typed accessors over it
- Create: `crates/mystquarto-core/tests/doc-sync.rs` — table/data consistency test
- Create: `scripts/render-mapping-tables.rs` (or `.py`) — regenerates the
  Markdown tables from `mappings.toml`
- Modify: `docs/dialect-comparison.md` — generated tables marked with
  `<!-- generated: do not edit -->` sentinels

## Implementation Steps

0. **Run the abort gate first (30 minutes, not 2 days).** Count the rows that
   survive as pure `{from, to, fidelity}` name swaps after removing (a)
   structural transforms that need hand-written code anyway — figure, table,
   tabset, include, embed, static code, blockquote attribution, iframe, proof —
   and (b) §3.4's rows, which are worked examples of one five-step algorithm,
   i.e. unit-test cases rather than data. If under ~40 rows survive, take this
   phase's own documented response: collapse to plain `match` arms in Rust, keep
   the Markdown as documentation only, drop `doc-sync` and the generator, and
   skip to Phase 3. Record the count and the decision here either way.
1. Transcribe §2, §3.4, §5, §8.2, §8.3, §10 tables into `mappings.toml`.
   Keep §1, §6, §7, §9, §11, §12 as prose — they describe policy and rationale,
   not row-wise mappings, and forcing them into data would obscure them.
2. Define the Rust types: `Fidelity`, `DirectiveMapping`, `RoleMapping`,
   `LabelPrefixMapping`, `ConfigFieldMapping`.
3. Write `mappings.rs` with a lazily-initialized static table plus lookup by
   MyST name and by Quarto class. Both directions need the index.
4. Write the generator that renders `mappings.toml` back into the Markdown
   tables, bounded by sentinel comments.
5. Write `doc-sync.rs`: regenerate in memory, compare to the committed
   Markdown, fail with a diff on mismatch.
6. Wire the generator into a `just`/`make` target so updating the doc is one
   command.

## Success Criteria

- [x] Step 0's surviving-row count and the resulting go/no-go are recorded here.
      **Step 0 (controller, run before this phase's implementation work
      started): counted survivable rows (pure `{from, to, fidelity}` name-swaps,
      excluding structural transforms that need hand-written code — figure,
      table, list-table/csv-table, tabset, include, notebook-embed,
      blockquote-attribution, iframe, proof/theorem — and excluding §3.4, which
      is worked examples of one algorithm, not row data) across §2, §5, §8.2,
      §8.3 and §10 and got ~94 rows, comfortably over the ~40-row threshold,
      matching this phase's own "~90 rows" estimate in the Risk Assessment
      below. Decision: GO — build the `mappings.toml` pipeline as specified, do
      not take the match-arms fallback.**
- [x] **If the gate said go:** `mappings.toml` covers every row of §2, §3.4, §5,
      §8.2, §8.3, §10; `cargo test doc_sync` passes and demonstrably fails when a
      table is hand-edited; regenerating the doc produces no diff on a clean tree.
      Verified: `mappings.toml` at the repo root transcribes §2 (33 non-structural
      `[[directive]]` rows + 11 `[[structural]]` rows), §5 (11 `[[role]]` + 5
      `[[inline]]` rows), §8.2 (32 `[[config_field]]` rows), §8.3 (6
      `[[export_format]]` rows), §10 (9 `[[legacy_role]]` rows), and §3.3's
      prefix-rule table, which the phase spec's own worked example identified as
      the data half of §3.4 (22 `[[label_prefix]]` rows) — 129 rows total, 96 of
      them the "survivable name-swap" rows the gate counted. `cargo test -p
      mystquarto-core` passes 4/4 (2 unit tests in `mappings.rs`, 2 in
      `doc-sync.rs`); temporarily corrupting a `mappings.toml` row and rerunning
      made `mappings_toml_matches_committed_doc` fail as expected, then the
      corruption was reverted and the suite passed again. Running
      `uv run python scripts/render-mapping-tables.py` against the committed
      `mappings.toml` and `docs/dialect-comparison.md` produces zero diff.
- [ ] **If the gate said stop:** *(does not apply — the gate said GO; no
      match-arms fallback was taken and `docs/dialect-comparison.md` was
      intentionally restructured, not left unchanged, to host the generated
      tables)*
- [x] Every mapping row carries a `fidelity` value and a `ref` back-pointer
- [x] Every conversion rule is **either** a `mappings.toml` row **or** a named
      transform whose fidelity and diagnostic code are declared in `mappings.toml`
      — the original "no rule absent from `mappings.toml`" was unachievable by
      this phase's own design, since structural transforms stay hand-written.
      The 11 `[[structural]]` rows record exactly this: name + fidelity + `ref`
      for figure, figure (div form), table, list-table/csv-table, tab-set,
      blockquote+attribution, iframe, include, notebook-embed, proof/theorem and
      static code, with no `to` field, per the phase spec's own instruction.
- [x] `toml` is present in Phase 3's dependency table. `crates/mystquarto-core/Cargo.toml`
      pins `toml = "0.8.23"` (latest 0.8.x at the time this phase ran) plus
      `serde` with the `derive` feature; Phase 3 inherits this from the
      workspace member it will build out rather than re-adding it.

## Risk Assessment

**Risk: over-engineering.** A data-driven table is only worth it if the
mappings are numerous and change. They are (~90 rows) and they will (new MyST
directives ship regularly).
*Signal:* if fewer than ~40 rows survive transcription, the indirection is not
paying for itself.
*Response:* collapse to plain `match` arms in Rust and keep the Markdown as
documentation only, dropping `doc-sync`. Note the reversal here.

**Risk: some mappings are not expressible as a row.** Tab-sets, tables, and
figures need structural transformation, not a name swap.
*Signal:* a row needs more than `{from, to, fidelity, note}`.
*Response:* those stay as hand-written transforms in Phase 5; `mappings.toml`
records only their existence and fidelity so diagnostics still work. Do not
contort the schema to hold them.
