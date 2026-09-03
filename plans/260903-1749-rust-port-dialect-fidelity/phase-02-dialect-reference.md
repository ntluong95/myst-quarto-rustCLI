---
phase: 2
title: "Dialect reference & conversion contract"
status: pending
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

- [ ] Step 0's surviving-row count and the resulting go/no-go are recorded here
- [ ] **If the gate said go:** `mappings.toml` covers every row of §2, §3.4, §5,
      §8.2, §8.3, §10; `cargo test doc_sync` passes and demonstrably fails when a
      table is hand-edited; regenerating the doc produces no diff on a clean tree
- [ ] **If the gate said stop:** `match` arms in Rust, `docs/dialect-comparison.md`
      unchanged, and Phases 6/7 amended to drop their `mappings.toml` dependency
- [ ] Every mapping row carries a `fidelity` value and a `ref` back-pointer
- [ ] Every conversion rule is **either** a `mappings.toml` row **or** a named
      transform whose fidelity and diagnostic code are declared in `mappings.toml`
      — the original "no rule absent from `mappings.toml`" was unachievable by
      this phase's own design, since structural transforms stay hand-written
- [ ] `toml` is present in Phase 3's dependency table

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
