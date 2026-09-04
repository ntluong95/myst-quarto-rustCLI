# Diagnostic codes

Every diagnostic `mystquarto` emits carries a severity, a stable code, a
file/line when one is known, and (for a preserved construct) a pointer into
`.mystquarto/preserved.json`. This page documents the cause and remedy for
each code currently emitted. See
[`plans/260903-1749-rust-port-dialect-fidelity/phase-07-diagnostics.md`](../plans/260903-1749-rust-port-dialect-fidelity/phase-07-diagnostics.md)
for the architecture (severity policy, `--strict` semantics, the
preservation sidecar).

## Severity

| Severity | Meaning | `--strict` | `--strict=all` |
|---|---|---|---|
| Error | A file could not be read/written, or a path-safety check refused an operation. Already fails a run today via a per-file failure, not one of the codes below. | fails | fails |
| Warning | Something a user would reasonably expect to keep working was lost or could not be resolved. | fails | fails |
| LossyExpected | A correct conversion that is inherently lossy — the two dialects differ, not a defect. | passes | fails |
| Info | Purely informational; never fails a run. | passes | passes |

Run a conversion with `--strict` to fail on Warning and above, or
`--strict=all` to additionally fail on LossyExpected — a "fully faithful
conversion or nothing" gate. A clean run always prints its counts, including
explicit zeros, so silence is a signal, not an assumption.

`.mystquarto/suppress.toml` baselines specific codes project-wide:

```toml
codes = ["MQ0410", "MQ0302"]
```

## Scope note

`mappings.toml` catalogs 57 `fidelity = "lossy"` rows and 13
`fidelity = "unmappable"` rows — more than the codes below. Most `lossy`
rows describe a fixed 1:1 syntax narrowing a writer performs unconditionally
(e.g. `:tags: [hide-input]` → `code-fold: true`), with no runtime branch
that currently distinguishes "this row's construct was converted" from
ordinary output. The codes below cover every place a diagnostic is *actually
emitted today* — every existing warning call site, plus the writer's
`Unmappable`/`Preserved` block handling. Extending per-row code coverage to
the rest of `mappings.toml` is tracked, not done, in
`phase-07-diagnostics.md`.

## MQ01xx — label / cross-reference / notebook-cell identity

| Code | Severity | Cause | Remedy |
|---|---|---|---|
| MQ0101 | Warning | Two labels in the conversion set normalized to the same Quarto id; the later one was suffixed (`-2`, `-3`, …). | Rename one of the source labels if the collision was unintentional; otherwise no action needed — both ids now resolve correctly. |
| MQ0102 | Warning | Two documents embedded the same notebook cell under different labels; only the first request's label was honored. | Give the notebook cell one label, or have each document define its own figure/embed around a differently-labeled cell. |
| MQ0103 | Info | The label sidecar (`.mystquarto/labels.json`) was refused outright — missing, oversized, malformed, wrong version, too many entries, or generated for the other conversion direction. | Normal on a first run or a natively-authored Quarto project. If unexpected, check the sidecar wasn't hand-edited or is from a stale prior run. |
| MQ0104 | Info | One malformed entry was dropped from an otherwise-valid label sidecar. | Usually safe to ignore; the label just won't restore its original MyST spelling on a reverse conversion. |
| MQ0105 | Warning | Notebook relabelling and/or the label sidecar write were skipped because the output writes into the input tree and `--force` was not passed. | Pass `--force` to write anyway, or point `-o` outside the input tree. |

## MQ02xx — block construct lossy or unmappable

| Code | Severity | Cause | Remedy |
|---|---|---|---|
| MQ0201 | LossyExpected | A construct has no equivalent in the target dialect (an unrecognized directive, an unsupported shortcode, …); its original source was preserved in `.mystquarto/preserved.json` and a single-line marker left in its place. | Expected for constructs the target dialect genuinely can't express. Converting back restores the original verbatim. |
| MQ0202 | LossyExpected | A preservation marker was read back and its sidecar entry restored, but the content didn't re-parse as one clean block in the current dialect — kept as an opaque preserved block rather than guessed at. | No action needed; the content round-trips through the marker mechanism. |
| MQ0203 | Warning | A preservation marker was read back but its id was not found in the sidecar (missing or stale `.mystquarto/preserved.json`) — the original content could not be restored. | Restore or regenerate `.mystquarto/preserved.json` from the run that created the marker; don't hand-edit or delete it between conversions. |
| MQ0204 | Warning | The block-content preservation sidecar was not written because the output writes into the input tree and `--force` was not passed. | Pass `--force` to write it anyway, or point `-o` outside the input tree. Note: under `--in-place`, this exact situation instead aborts the whole run rather than warning — see the note below. |

**`--in-place` and MQ0204.** `--in-place` deletes each source file immediately after its own output is written. If a run produced preserved block content and the sidecar that's the *only* record of it can't be written (this MQ0204 situation, without `--force`), continuing would delete the source with no way to recover that content. So under `--in-place` specifically, this refuses the entire run up front — before anything is written or deleted — with a plain error, not an MQ0204 warning. MQ0204 itself only fires in every other mode (`-o` elsewhere, even one that happens to alias the input root), where nothing gets deleted and a degraded round-trip is the actual, survivable outcome.

## MQ03xx — inline construct, citations, bibliography

| Code | Severity | Cause | Remedy |
|---|---|---|---|
| MQ0301 | Warning | A citation key is used somewhere in the conversion set but defined in no reachable `.bib` file (often a DOI key MyST resolved live). | Add the missing entry to the project's `.bib` file. |
| MQ0302 | Info | A bibliography setting or supplemental DOI bibliography was synthesized so Quarto can resolve citations locally. | No action — a helpful autofix, not a loss. |

## MQ04xx — config / frontmatter

| Code | Severity | Cause | Remedy |
|---|---|---|---|
| MQ0401 | LossyExpected | A `myst.yml` project field has no `_quarto.yml` equivalent (`abbreviations`, `open_access`, `venue`, `id`, or any unrecognized field); preserved as a comment and in `.mystquarto/preserved.json`. | No action — a reverse conversion restores it under its original key. |
| MQ0402 | LossyExpected | A manuscript's `project.toc` has entries beyond its article and notebooks (e.g. an appendix); Quarto's manuscript shape has no slot for them. | Preserved, not dropped — restored on a reverse conversion. |
| MQ0403 | LossyExpected | A `_quarto.yml` book's `part:`-grouped chapters were flattened into a plain myst.yml toc list. | The grouping label is lost; the chapters themselves are not. Re-group manually in myst.yml if the label matters. |
| MQ0404 | LossyExpected | A `_quarto.yml` book's `appendices` were appended to the myst.yml toc as regular entries. | myst.yml has no appendix/main-matter distinction; the files themselves are preserved. |
| MQ0405 | Warning | `_quarto.yml` `categories` had more than one entry; only the first was mapped back to myst.yml's single-valued `subject`. | The remaining categories are genuinely lost on this direction; add them back to `subject`/elsewhere by hand if needed. |
| MQ0406 | Warning | An export/format value (`meca`, or an unrecognized format name) has no equivalent in the target dialect and was dropped entirely. | No target exists; remove the export or accept the loss. |
| MQ0407 | Warning | An export's Quarto format was guessed from a non-portable `template:` name's suffix. | Verify the guessed format is correct; override with an explicit `format:` if not. |
| MQ0408 | Warning | A `_quarto.yml` `format:` key has no exact myst.yml export equivalent; passed through as a `format:` value of the same name. | Usually fine as-is; myst.yml's `format` field accepts arbitrary names. |
| MQ0409 | Warning | A root-level `_quarto.yml` key has no myst.yml equivalent at all (`execute:`, `csl:`, `theme:`, …); dropped. | No myst.yml target exists for this key; reproduce the setting via a different mechanism if needed. |
| MQ0410 | LossyExpected | A page-frontmatter field has no correct target in the other dialect (`label`, `math`); dropped rather than mismapped. | `label` isn't a cross-reference target in this tool's model, so nothing regresses. `math` (LaTeX macros) has no Quarto page-level equivalent. |
| MQ0411 | Warning | `myst.yml` set both `banner` and `thumbnail`; `_quarto.yml` has one `image:` slot, so `banner` was used. | Remove whichever of the two isn't wanted in the Quarto output. |

## MQ06xx — file, IO, path safety, discovery

| Code | Severity | Cause | Remedy |
|---|---|---|---|
| MQ0601 | Warning | A notebook in the conversion set could not be read. | Check the file exists and is readable; any embed referencing its cells will not resolve until fixed. |
| MQ0602 | Warning | A notebook was read but its cells could not be indexed (malformed JSON). | Fix the notebook's JSON structure. |
| MQ0603 | Warning | A notebook's output-tree copy could not be read back to relabel it, or relabelling itself failed. | Check the asset copy step completed and the notebook is valid JSON. |
| MQ0604 | Warning | An asset path was a symlink and was skipped without dereferencing its target. | Replace the symlink with a regular file inside the project if the asset should be copied. |
| MQ0605 | Warning | A path-safety check refused an include or embed target (escapes project root, include cycle, depth exceeded, or absolute path). | Ensure all include/embed targets reside within the project root and do not form circular references. |
