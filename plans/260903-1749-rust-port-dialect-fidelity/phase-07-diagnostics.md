---
phase: 7
title: "Diagnostics & lossy preservation"
status: pending
priority: P1
effort: "4d"
dependencies: [5, 6]
---

# Phase 7: Diagnostics & lossy preservation

## Overview

Make every lossy decision visible. Today the tool converts `article-template`
into broken Quarto and reports `Converted 4 file(s).` with no warnings — defect
D12, and the reason all the other defects went unnoticed. This phase makes
silence mean correctness.

> **Red team RT-02, RT-10, RT-11 applied.** The original phase preserved
> unmappable source inside HTML comments, which injects live markup into
> rendered output; specified a `--strict` gate that is permanently red on
> correct output; and asserted a round-trip criterion with no reader behind it.
> Effort raised 3d → 4d.

## Requirements

- Functional: every `lossy` or `unmappable` conversion emits a diagnostic with
  file, line, severity, code, and the reference-doc section explaining it.
- Functional: unmappable source is preserved **losslessly and inertly** — it can
  never become markup in the rendered output.
- Functional: `--strict` fails on genuine information loss and **passes** on a
  correct conversion of a normal MyST project.
- Non-functional: diagnostic order is deterministic (file, then line).

## Architecture

### Preservation: sidecar, not HTML comments (RT-02, decision RD-2)

The original format wrapped arbitrary source in `<!-- … -->` and mitigated only
`-->` inside it. That mitigation addresses the wrong mechanism. **Pandoc ends a
raw-HTML block at the first blank line**, not at `-->`, and unmappable
constructs — `{glossary}`, `{epigraph}`, `{pull-quote}`, `{list-table}` — routinely
contain blank lines. Reproduced end to end with `quarto 1.9.36`:

```
render exit: 0
73:<script>alert(1)</script>
74:-->
```

A live script tag in the published HTML and a stray `-->` visible to readers,
with the render reporting success. The plan's own tripwire — "*Signal:*
`quarto render` errors" — never fires, and Phase 8's `?@` grep does not look for
this. Four-space indenting inside the comment, the original fallback, does not
change comment termination at all. `--!>` is also a valid HTML5 comment
terminator that the `-->` rule misses.

Preservation therefore moves out of the document:

```markdown
<!-- mystquarto MQ0203: {glossary} preserved — see .mystquarto/preserved.json#b7f3 -->
```

A single-line, content-free marker. The original source lives in
`.mystquarto/preserved.json`, keyed by a content hash:

```json
{ "version": 1,
  "entries": { "b7f3": { "file": "article.md", "line": 88, "code": "MQ0203",
                         "kind": "glossary", "original": ["```{glossary}", "term", "  definition", "```"] } } }
```

This satisfies "nothing is ever destroyed" better than the comment form did —
the content is intact, structured, and cannot be reinterpreted as markup. Phase 4
reads the marker plus the sidecar to restore the construct (RT-11); the original
plan emitted preservation with no reader, so its round-trip criterion could not
have been met by anyone.

`--no-preserve` is **dropped**. It contradicted the accepted "nothing is ever
destroyed" decision and had no success criterion. With preservation out of the
document, the reason it existed — output pollution — no longer applies.

### Diagnostic

```rust
pub struct Diagnostic {
    pub severity: Severity,        // Error | Warning | LossyExpected | Info
    pub code: &'static str,        // "MQ0104"
    pub message: String,
    pub file: PathBuf,
    pub span: Span,
    pub reference: Option<&'static str>,  // "§3.4"
    pub preserved: Option<PreservedId>,
}
```

Stable codes let users suppress a class in CI and let corpus tests assert on
codes rather than message text.

| Range | Class |
|---|---|
| `MQ01xx` | Label / cross-reference |
| `MQ02xx` | Block construct lossy or unmappable |
| `MQ03xx` | Inline construct, citations, bibliography |
| `MQ04xx` | Config / frontmatter |
| `MQ05xx` | Execution engine |
| `MQ06xx` | File, IO, path safety, discovery |

### Severity policy — four classes (RT-10, decision RD-4)

The original three-class policy made `--strict` unusable. It assigned **Warning**
to "config field preserved as comment", and Phase 6 preserves `abbreviations`,
`open_access`, `venue`, and `id` for every MyST project that has them —
`article-template/myst.yml` has all four plus `banner` and `downloads`. Phase 8
then stated the consequence plainly: "`--strict` exits 1 on `article-template/`".
A gate that is red on a *correct* conversion is not a gate; users delete it, and
D12-class silent loss returns with the tool's blessing.

| Situation | Severity | In `--strict`? |
|---|---|---|
| Unreadable/unwritable file, path-safety refusal | **Error** | always fails |
| Label collision disambiguated | **Warning** | yes |
| Engine mismatch (knitr → MyST) | **Warning** | yes |
| Citation key absent from every reachable `.bib` | **Warning** | yes |
| Notebook outside conversion set, embed unresolvable | **Warning** | yes |
| Construct approximated (`danger`→`important`) | **LossyExpected** | opt-in |
| Config field with no target equivalent, preserved | **LossyExpected** | opt-in |
| Construct with no equivalent, preserved | **LossyExpected** | opt-in |
| Label normalized (`fig:x`→`fig-x`) | **Info** | never |
| Sidecar absent on reverse conversion | **Info** | never |

`LossyExpected` names conversions that are correct *and* lossy — an inherent
consequence of the formats differing, not a defect. `--strict` fails on Error and
Warning. `--strict=all` additionally fails on `LossyExpected`, for users who want
a fully faithful conversion or nothing. A `.mystquarto/suppress.toml` can baseline
specific codes.

Phase 8 now requires `--strict` to **exit 0** on a correct `article-template/`
conversion, reversing the original criterion.

### Human output

```
article.md:55:1: warning[MQ0201]: notebook `other.ipynb` is outside the conversion
    set, so its cell label cannot be rewritten; the embed will not resolve
    see docs/dialect-comparison.md §7

article.md:88:1: lossy[MQ0203]: {glossary} has no Quarto equivalent; preserved
    in .mystquarto/preserved.json#b7f3
    see docs/dialect-comparison.md §11

Converted 4 files: 0 errors, 1 warning, 6 expected-lossy.
Run with --strict to fail on warnings, --strict=all to fail on expected-lossy too.
```

Counts always print, including zeros — a visible `0 warnings` is the signal that
silence was earned.

## Related Code Files

- Create: `crates/mystquarto-core/src/diagnostics/mod.rs`, `codes.rs`, `render.rs`
- Create: `crates/mystquarto-core/src/preserve.rs` — sidecar write; read is Phase 4
- Create: `docs/diagnostics.md` — code reference with cause and remedy
- Modify: `crates/mystquarto/src/main.rs` — reporting, `--strict[=all]`, exit codes
- Modify: writers and config modules — emit at every lossy branch
- Read: `docs/dialect-comparison.md` §11

## Implementation Steps

1. Define `Diagnostic`, the four-class `Severity`, and the code enum; thread a
   collector through the conversion context.
2. Assign a code to every `fidelity = "lossy"` / `"unmappable"` mapping row.
3. Emit at every lossy branch in writers and config. Assert by test that every
   lossy mapping row has at least one corpus case producing its code.
4. Implement `preserve.rs`: content-hash keying, marker emission, sidecar write.
   Verify the marker is a single line with no user content in it.
5. Implement `--strict` and `--strict=all` promotion, `suppress.toml`, and exit
   codes: 0 clean, 1 on promoted severities.
6. Implement the human renderer with per-class counts; sort by (file, line, column).
7. Write `docs/diagnostics.md`.
8. Run against `article-template/` and confirm: every legitimate lossy case
   produces a diagnostic, and `--strict` exits **0**.
9. **Injection regression test:** a corpus case whose unmappable source contains
   a blank line followed by `<script>`, asserting the rendered HTML contains no
   executable markup and no stray comment terminator.

## Success Criteria

- [ ] `myst2quarto article-template/` emits a non-empty, accurate diagnostic list (D12)
- [ ] Every diagnostic has file, line, code, and a reference section
- [ ] Every lossy mapping row has a corpus case that triggers its code
- [ ] `--strict` exits **0** on a correct `article-template/` conversion (RT-10)
- [ ] `--strict` exits 1 when a genuine Warning-class loss occurs
- [ ] `--strict=all` exits 1 on `article-template/` (expected-lossy present)
- [ ] `suppress.toml` baselines a code and is honored
- [ ] Preserved source lives in `.mystquarto/preserved.json`, never inline
- [ ] The injection case renders with no executable markup and no stray `-->` (RT-02)
- [ ] Round-trip recovers preserved constructs via Phase 4's reader (RT-11)
- [ ] Diagnostic order is deterministic across runs
- [ ] `docs/diagnostics.md` documents every emitted code
- [ ] A clean conversion prints explicit zero counts
- [ ] `--no-preserve` does not exist

## Risk Assessment

**Risk: the sidecar makes preserved content easy to ignore.** Out of the
document is out of sight; a user may never look at `preserved.json`.
*Signal:* users report surprise at missing content.
*Response:* the marker comment stays in the document at the original location and
names the code and entry, so the loss is visible where it happened. The
end-of-run summary counts expected-lossy separately for the same reason.

**Risk: `LossyExpected` becomes a dumping ground.** Anything inconvenient gets
filed there to keep `--strict` green.
*Signal:* the expected-lossy count grows without new constructs.
*Response:* class assignment lives in `mappings.toml`, so it is reviewable in one
place, and step 3's test requires a corpus case per code. Moving a code between
classes is a visible diff.

**Risk: stable codes calcify too early.**
*Signal:* codes needing renumbering during Phase 8.
*Response:* codes derive from the completed mapping set, so the space is known.
Never reuse a retired code; mark it deprecated in `docs/diagnostics.md`.
