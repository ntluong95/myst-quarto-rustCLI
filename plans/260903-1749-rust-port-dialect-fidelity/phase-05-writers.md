---
phase: 5
title: "Writers: IR → MyST + Quarto, label normalization"
status: done
priority: P1
effort: "6d"
dependencies: [4]
---

# Phase 5: Writers — IR → MyST + Quarto, label normalization

## Overview

Emit both dialects from the IR, and implement the label registry that makes
Quarto cross-references actually resolve. This phase fixes the largest defect
class (D1, D2, D3, D11) and delivers the sidecar round-trip mechanism.

> **Red team RT-03, RT-08, RT-09 applied.** The original phase specified the
> registry as document-scoped while Phase 4 mandated run-scoped, used a flat
> sidecar map that can represent neither, read that sidecar from untrusted input
> with no validation, left "output root" undefined for `--in-place` and
> single-file runs, and chose a notebook-embed policy that makes `quarto render`
> produce no output at all. Effort raised 5d → 6d.

## Requirements

- Functional: `QuartoWriter` emits IDs satisfying Quarto's prefix rules —
  hyphen-separated, type-prefixed, lowercase, no colons, no underscores.
- Functional: `MystWriter` emits **modern mystmd v1 only**. No legacy roles, ever.
- Functional: the label registry is **run-scoped**, and the sidecar is keyed by
  source file so it can represent a multi-file conversion.
- Functional: the sidecar is validated as untrusted input on read.
- Functional: notebook cells referenced by embeds are **relabelled in place** so
  the emitted `{{< embed >}}` resolves (RD-3).
- Non-functional: output is deterministic — same inputs, same bytes, every run,
  and stable under changes to the *set* of files converted.

## Architecture

### LabelRegistry — run-scoped, one definition (RT-08)

Built in one pass over **every document in the conversion set** before any
writing. Run-scoped is the only lifetime that satisfies Phase 4's forward-
reference requirement and Quarto's project-global crossref namespace; the
original phase's document-scoped construction would emit duplicate `{#fig-x}`
IDs across files with no diagnostic.

```rust
pub struct LabelRegistry {
    forward: BTreeMap<(SourceFile, Label), QuartoId>,
    reverse: BTreeMap<(SourceFile, QuartoId), Label>,
    kinds:   BTreeMap<(SourceFile, Label), RefKind>,
}
```

Normalization (reference §3.4), applied in order:

1. Split on the first `:` — `fig:samples` → (`fig`, `samples`).
2. Map the kind token: `fig`→`fig`, **`tab`→`tbl`**, `eq`→`eq`, `sec`→`sec`,
   `nb`→`fig`.
3. If no kind token, **infer from `RefKind`** — possible only because the IR
   carries type with the label.
4. Lowercase; `_`→`-`; strip characters outside `[a-z0-9-]`.
5. On collision, append `-2`, `-3`, … **seeded from a stable sort of
   `(source_path, label)`**, not registry traversal order. Emit a warning.

Step 5's seeding matters: with traversal-order suffixing, adding an unrelated
file shifts which document keeps the bare `fig-samples`, silently rewriting IDs
inside a file the user did not touch. `BTreeMap` throughout, so serialization
order is stable — hash order is not.

### Sidecar — keyed, validated, merged (RT-08, RT-09)

Written to `.mystquarto/labels.json` under the **output root**, defined as:

| Invocation | Output root |
|---|---|
| `-o DIR` | `DIR` |
| default, directory input | the derived `<input>-quarto/` |
| `--in-place` | the input root — the sidecar lands in the source tree, hence `--no-label-map` |
| single-file input | the file's derived output directory |

```json
{ "version": 1,
  "generated_by": "mystquarto 0.2.0",
  "direction": "myst_to_quarto",
  "source_root": "article-template",
  "files": {
    "article.md": {
      "content_hash": "sha256:…",
      "labels": { "fig-samples": "fig:samples", "tbl-phenotypic-variation": "tab:phenotypic-variation" }
    }
  }
}
```

Keying by file is required: a flat `{id: label}` map cannot disambiguate two
files that both define `fig:samples`, so the reverse conversion would restore
the wrong original into one of them.

**Writes merge, never replace.** A single-file re-run (`myst2quarto ./paper/article.md`)
defaults to the same output directory as the project run, so a
replace-on-write sidecar would destroy every other file's mappings.

**Reads are untrusted.** The sidecar sits in the input tree, which per the Phase 1
threat model is attacker-influenced. On read:

- reject `version != 1`;
- cap file size and entry count at documented constants;
- reject a `direction` that does not match the run — **Warning**, not Info;
- validate every restored label against `^[A-Za-z0-9_:.\-]+$` and drop
  non-conforming entries with an `MQ01xx` warning. MyST imposes no constraint on
  label strings, so an unvalidated value containing a newline escapes the
  `(sec:x)=` construct the writer emits and injects arbitrary MyST;
- flag entries whose `content_hash` no longer matches as stale and skip them;
- on parse failure, fall back to the absent-sidecar path with a warning, not an error.

Absence remains Info — normal for a natively-authored Quarto project.

### Notebook relabelling (RD-3, RT-03)

The original policy — emit `{{< embed nb.ipynb#fig-x >}}` and leave the notebook
alone — was verified to fail:

```
quarto render embed.qmd  →  ERROR: The cell fig-analysis does not exist in notebook
                            NO HTML PRODUCED
```

That makes Goal 4 unreachable on the flagship fixture. Notebook relabelling is
therefore **in scope**:

1. Phase 4's notebook index gives `nb:analysis → analysis.ipynb`.
2. The writer normalizes the cell label the same way document labels are
   normalized (`nb:analysis` → `fig-analysis`).
3. It rewrites `#| label:` inside the copied notebook in the **output** tree —
   never the source — and records the mapping in the sidecar.
4. Notebooks outside the conversion set cannot be rewritten: emit `Preserved`
   plus a warning, not a dangling embed.

Scope boundary: notebook **cell content** is not converted. Only `#| label:`
lines are patched. Open question 1 in `plan.md` covers whether that suffices.

### MystWriter — modern output

| IR | Emitted |
|---|---|
| `Heading` + label | `(sec:x)=`, blank line, `## Heading` |
| `Figure { Path }` | `:::{figure} path` + `:label:` + `:width:` + caption + `:::` |
| `Figure { CellRef }` | `:::{figure} #nb:x` + `:label:` + caption + `:::` |
| `Table` | `:::{table}` + `:label:` + caption + rows + `:::` |
| `Math` + label | `` ```{math} `` + `:label:` + body |
| `Admonition` | `` ```{note} `` / `` ```{admonition} Title `` |
| `TabSet` | `::::{tab-set}` / `:::{tab-item} Label` |
| `Comment { Percent }` | `% text` |
| citation | `[@key]` / `@key` — **never** `` {cite}` `` |
| cross-ref | `@fig:x` — **never** `` {numref}` `` |
| `Preserved` | marker comment + sidecar entry (Phase 7), never inline source |

### QuartoWriter

| IR | Emitted |
|---|---|
| `Heading` + label | `## Heading {#sec-x}` |
| `Figure { Path }`, single-block caption | `![caption](path){#fig-x width="…"}` |
| `Figure { Path }`, multi-block caption | `::: {#fig-x}` … `:::` div form |
| `Figure { CellRef }` | `{{< embed analysis.ipynb#fig-x >}}` with the notebook relabelled |
| `Table` | rows + `: Caption {#tbl-x}` |
| `Math` + label | `$$` + body + `$$ {#eq-x}` |
| `Admonition` | `::: {.callout-note title="…" collapse="…"}` |
| `TabSet` | `::: {.panel-tabset}` + `## Label` per item |
| `Comment { Percent }` | `<!-- text -->` |
| `Include` | `{{< include _file.qmd >}}`, blank-line padded |
| `Preserved` | marker comment + sidecar entry (Phase 7) |

Admonition kinds outside Quarto's five collapse per reference §2 with a warning.
Collapse polarity inverts: MyST `:open: true` ≙ Quarto `collapse="false"`.

## Related Code Files

- Create: `crates/mystquarto-core/src/writer/mod.rs`, `myst.rs`, `quarto.rs`
- Create: `crates/mystquarto-core/src/label/registry.rs`, `normalize.rs`, `sidecar.rs`
- Create: `crates/mystquarto-core/src/notebook/relabel.rs`
- Read: `docs/dialect-comparison.md` §2, §3, §5, §10
- Read: `src/mystquarto/transforms/*.py` (behavior to match where still correct)

## Implementation Steps

1. Implement `normalize.rs` with the five ordered rules; unit-test every
   reference §3.4 row including `tab:`→`tbl-`.
2. Build `LabelRegistry` as a run-scoped pre-write pass over the whole
   conversion set. Detect collisions here; seed suffixes from the stable sort.
3. Implement `sidecar.rs`: file-keyed schema, merge-on-write, and the full
   untrusted-read validation list. Test hostile, stale, corrupt, wrong-direction,
   wrong-version, and oversized sidecars.
4. Implement `notebook/relabel.rs` operating on the **output** copy only.
5. `QuartoWriter` construct by construct, defect tests first: D1, D2, D3, D11.
6. `MystWriter`, with a grep-based guard asserting the output contains no
   `{cite}`, `{numref}`, `{ref}`, `{eq}`, `{doc}`.
7. Wire `run_conversion`: read → notebook index → registry → write → relabel → sidecar.
8. Round-trip test: MyST → Quarto → MyST with sidecar present, byte-identical on
   the Stable class.
9. **Determinism test that varies the file set**, not just repeats the command —
   converting a subset must not renumber labels in files outside the subset.

## Success Criteria

- [x] Every reference §3.4 normalization row has a passing unit test
- [x] `tab:x` → `tbl-x` (not `tab-x`)
- [x] Unprefixed labels get the prefix inferred from block type
- [x] Collisions are disambiguated from a stable `(source_path, label)` sort and warned
- [x] Converting a subset of files does not renumber labels in other files (RT-08)
- [x] No Quarto output contains `:` or `_` in any `{#id}`
- [x] No MyST output contains any legacy role
- [x] `article-template/` → Quarto: every `@ref` has a matching `{#id}` (D1)
- [x] Figure labels survive (D2); table captions and labels survive (D3)
- [x] `:::{figure} #nb:analysis` → `{{< embed analysis.ipynb#fig-analysis >}}`
      **and the notebook copy is relabelled**, so `quarto render` produces HTML (D11)
- [x] A notebook outside the conversion set yields `Preserved` + warning, never a
      dangling embed
- [x] Sidecar: hostile, stale, corrupt, wrong-direction, wrong-version and
      oversized inputs each produce the documented outcome (RT-09)
- [x] A newline-bearing sidecar label cannot escape the `(sec:x)=` construct
- [x] Single-file re-run merges into the existing sidecar rather than truncating it
- [x] MyST→Quarto→MyST byte-identical on the Stable class with sidecar present
- [x] Two identical runs produce identical bytes

## Risk Assessment

**Risk: notebook relabelling mutates a file the user considers an input.** The
mitigation is that only the output copy is touched, but a user running
`--in-place` has no separate output copy.
*Signal:* a source notebook's `#| label:` changes on disk.
*Response:* refuse notebook relabelling under `--in-place` without `--force`, and
emit `Preserved` + warning instead. Add this to Phase 3's in-place contract tests.

**Risk: multi-block figure captions force the div form and churn output.**
*Signal:* the same figure renders both ways across runs.
*Response:* the rule is deterministic — div form iff the caption exceeds one
block. Encode it once; test both branches.

**Risk: the sidecar becomes a hidden coupling users do not expect.** It now lives
under `.mystquarto/` and carries content hashes.
*Signal:* user confusion, or stale-hash warnings on legitimate edits.
*Response:* stale entries are skipped with an Info diagnostic, never an error;
`--no-label-map` remains the opt-out, with its one-way lossiness documented.

**Risk: relabelling breaks a notebook that other tools reference by label.**
A MyST project may reference `nb:analysis` from elsewhere.
*Signal:* MyST build warnings after a Quarto→MyST→Quarto cycle.
*Response:* the sidecar records the original cell label, so the reverse
conversion restores it. Test the full cycle including the notebook.
