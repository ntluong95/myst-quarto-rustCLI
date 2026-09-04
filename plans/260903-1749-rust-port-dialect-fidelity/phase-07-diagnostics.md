---
phase: 7
title: "Diagnostics & lossy preservation"
status: done
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

- [x] `myst2quarto article-template/` emits a non-empty, accurate diagnostic list (D12)
- [x] Every diagnostic has file, line, code — `.reference` (the `docs/dialect-comparison.md`
      section pointer) is never set on any construction site; a known, disclosed gap (M6)
- [ ] Every lossy mapping row has a corpus case that triggers its code — deliberately
      narrowed scope: ~20 codes exist for every site that *actually emits* a diagnostic
      today, not one per `mappings.toml` row (57 lossy + 13 unmappable); see
      `docs/diagnostics.md`'s "Scope note" and this phase's Review Outcome below
- [x] `--strict` exits 1 when a genuine Warning-class loss occurs
- [x] `--strict=all` exits 1 on `article-template/` (expected-lossy present)
- [ ] `--strict` exits **0** on a correct `article-template/` conversion (RT-10) —
      **accepted deviation**, see Review Outcome's H3 note: the fixture genuinely has
      two DOI citation keys missing from `references.bib`, so `--strict` correctly
      exits 1 there; the fixture is not "correct" in the sense this criterion assumed
- [x] `suppress.toml` baselines a code and is honored — single-line `codes = [...]` form
      only; a multi-line array is silently ignored (M1, deferred, documented below)
- [x] Preserved source lives in `.mystquarto/preserved.json`, never inline — verified
      against a real `quarto render`, including the injection case
- [x] The injection case renders with no executable markup and no stray `-->` (RT-02) —
      verified against a real `quarto render`
- [x] Round-trip recovers preserved constructs via Phase 4's reader (RT-11) — verified
      byte-identical, including through a cross-dialect hop (see C2 in Review Outcome)
- [ ] Diagnostic order is deterministic across runs — sorts by `(file, line)` only;
      two diagnostics at the same file:line with different codes/messages have no
      guaranteed tiebreak (L2, deferred)
- [x] `docs/diagnostics.md` documents every emitted code
- [ ] A clean conversion prints explicit zero counts — true for Error/Warning/
      LossyExpected; `Info`-severity diagnostics are never counted at all (M5, deferred)
- [x] `--no-preserve` does not exist

## Review Outcome (pre-completion code review)

An adversarial review (mirroring Phases 5 and 6) found **3 Critical + 3 High + 7
Medium + 8 Low** issues the initial implementation's 295 green tests did not catch.

**Fixed this pass (Critical + High, user-scoped):**
- Critical: `--in-place` could permanently destroy a preserved construct.
  Moving unmappable content out of the document (into the sidecar) meant the
  H1 "output writes into input tree ⇒ refuse without `--force`" gate, which
  only blocked the *sidecar write*, no longer prevented the *source deletion*
  `--in-place` performs immediately after — the content's only remaining copy
  could vanish, exit 0. Fixed:
  `orchestrate::refuse_if_in_place_would_lose_preserved_content` now refuses
  the whole run, before anything is written or deleted, whenever this
  combination is possible.
- Critical: a preserved construct's content could be silently reinterpreted
  as a different construct — and, if its body itself contained a code fence,
  let trailing lines escape as live, unescaped markup — when restored during
  a reverse conversion and reparsed through the *wrong* dialect's parser
  (e.g. a backtick-fenced MyST directive reparsing as a Quarto executable
  code cell). This is the injection class RT-02 exists to close, reopened by
  a path the original regression test didn't cover. Fixed by recording which
  dialect each sidecar entry was captured in
  (`preserve::Dialect`/`PreservedEntry::dialect`) and refusing to ever
  reparse foreign-dialect content — it now always round-trips byte-identical
  instead.
- Critical: a crafted directive/shortcode name could hijack which sidecar
  entry an unrelated marker resolved to (the id-lookup needle,
  `.mystquarto/preserved.json#`, wasn't excluded from the source-derived
  `kind` text embedded in the marker, and resolution used the *first*
  occurrence). Fixed: the needle is neutralized in `preserve::marker`, and
  `reader::preservation_marker_id` additionally resolves from the *last*
  occurrence as defense in depth.
- High: `--strict <input>` (the flag before the positional — how the
  previous `bool`-typed flag always worked) was a hard CLI parse error.
  Fixed with `require_equals = true`.
- High: the single most common preservation disposition (an unmappable
  `myst.yml` config field) never actually emitted a diagnostic —
  `--strict=all` passed "by accident" via unrelated citation warnings on
  `article-template/`. Fixed: `myst_to_quarto::convert` now emits
  `MQ0401`/`LossyExpected` whenever `preserved_fields` is non-empty.
- High (H3): `--strict` exits 1 on `article-template/` itself, contradicting
  this file's own stated criterion. Investigated and accepted as documented
  above — the fixture's two DOI citation keys really are absent from
  `references.bib`; the diagnostic is correct, the criterion's assumption
  about the fixture was not.

**Deferred (Medium + Low, explicit user decision — not fixed this pass):**
`suppress.toml`'s hand-rolled parser only supports a single-line
`codes = [...]` array; `suppress.toml` is looked up relative to a *file*
path (not its parent directory) in single-file mode, so it's silently
inert there; `MQ0202` was unreachable (the disposition it names — a
restored, matching-dialect entry that doesn't collapse to one clean block —
never actually emitted it; now moot in the sense that this path is rare by
construction, but still worth a real diagnostic); `PreservedEntry.file`
stores an absolute, machine-local path rather than one relative to the
input root (a portability/reproducibility issue for a committed sidecar
file); `Info`-severity diagnostics are never printed or counted, so a
corrupted label sidecar currently produces zero observable output;
`.reference` is unset on every diagnostic; the print-order sort has no
tiebreaker; a stale preservation-sidecar entry is never pruned on a run
that no longer produces any (config-field preservation is written
unconditionally for exactly this reason; block-content preservation is
not); one diagnostic code (`MQ0105`, before this pass's fix) was reused
across three unrelated dispositions. None of these lose data or reintroduce
injection on their own (unlike the three Criticals); each is a real but
bounded gap, most already noted inline in `docs/diagnostics.md` or this
file's Success Criteria above.

Verified end-to-end after the fix: `cargo test --workspace` (299 pass, 0
fail, 0 ignored), `cargo clippy --workspace --all-targets` clean, `cargo fmt
--all -- --check` clean, plus direct reproductions of all three Criticals'
exact failure scenarios (`--in-place` refusing rather than deleting;
byte-identical restoration through a Quarto hop for a backtick-fenced
directive whose body contains a fence and a `<script>`-adjacent live HTML
tag, verified against a real `quarto render` producing no executable
markup; two constructs whose marker `kind` collides with the sidecar
needle restoring distinct, uncorrupted content) and a real `myst2quarto
article-template/` → `quarto render --to html` run.

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
