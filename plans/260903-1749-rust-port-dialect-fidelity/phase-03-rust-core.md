---
phase: 3
title: "Rust core: workspace, IR, YAML, orchestration contract"
status: done
priority: P1
effort: "7d"
dependencies: [2]
---

# Phase 3: Rust core — workspace, IR, YAML, orchestration contract

## Overview

Stand up the Cargo workspace and the four foundations everything else builds on:
the document IR, a YAML strategy that actually preserves style, the **file
orchestration contract** (assets, path safety, atomicity, in-place), and the CLI.

> **Red team RT-01, RT-05, RT-06, RT-11, RT-16 applied.** The original phase
> justified `saphyr` with a false capability claim, ported `discover_files` and
> nothing else of the orchestration layer, left `--in-place` as a one-line table
> row over a destructive operation, had no `Preserved` IR variant, and named a
> published crate the workspace never creates. Effort raised 4d → 7d.

## Requirements

- Functional: `mystquarto`, `myst2quarto`, `quarto2myst` accept exactly the flags
  the Python CLI accepts, with **documented** semantics — parity by specification,
  not by reference to the old implementation.
- Functional: frontmatter and config YAML round-trip block scalars, key order,
  and comments (fixes D9) without depending on a general YAML round-trip crate.
- Functional: no conversion reads or writes outside its declared roots, follows a
  symlink out of the input tree, or recurses into its own output.
- Functional: assets reach the output tree, refresh when the source changes, and
  never nest.
- Functional: `--dry-run` writes zero bytes on every code path.
- Functional: the IR can represent every construct in reference §2, plus
  `Preserved`, without an untyped escape hatch.
- Non-functional: no `unsafe`; `#![forbid(unsafe_code)]` in both crates.
- Non-functional: MSRV pinned in `rust-toolchain.toml`.

## Architecture

### Workspace (RT-16)

```
Cargo.toml                      # workspace root
crates/mystquarto-core/         # library
crates/mystquarto/              # binaries — THE PUBLISHED PACKAGE
mappings.toml                   # from Phase 2, if the gate said go
rust-toolchain.toml
```

`cargo install X` resolves `X` as a **package name**, not a binary name. The
original layout published `mystquarto-core` + `mystquarto-cli`, making the
documented `cargo install mystquarto` fail. The CLI crate is named `mystquarto`
and depends on `mystquarto-core`.

### YAML strategy (RT-01, decision RD-1)

The original rationale — "saphyr's document-level API preserves block scalars
and comments" — is false. Verified against the vendored crate:

```
saphyr-0.0.12/src/emitter.rs:241: ScalarStyle::Literal => todo!(),
saphyr-0.0.12/src/emitter.rs:242: ScalarStyle::Folded  => todo!(),
```

It panics on exactly the `abstract: |` case it was selected to fix, discards
style at load, and has no comment type anywhere in its source.

**No general YAML round-trip is used.** Two narrow mechanisms instead:

| Path | Mechanism |
|---|---|
| **Frontmatter edit** (`.md`/`.qmd`, mostly-unchanged keys) | Parse for reading; apply changes as **targeted line surgery** on the original text. Untouched keys stay byte-identical, including block scalars and comments, because they are never re-serialized |
| **Config synthesis** (`_quarto.yml` from `myst.yml`) | A purpose-built deterministic emitter over the known key set from reference §8.2 — block scalars, key order, and comment emission, for ~40 keys, not arbitrary YAML |

The synthesis path needs a real emitter because there is no original text to
operate on — the gap the original Phase 3 fallback did not cover. Writing one for
a closed key set is bounded work; adopting a general YAML round-trip library is
not, because none in the ecosystem does it.

Reading uses a safe parser with no alias expansion (billion-laughs) and YAML 1.2
semantics, so `open_access: no` reads as the string `"no"`.

### Orchestration contract (RT-05, RT-06, RT-15)

The layer the original plan omitted. Every rule below is a requirement with a test.

**Path safety.** Canonicalize the input root, the output root, and every resolved
path. Before any read or write, assert the target is a descendant of its declared
root. Reject and diagnose (`MQ06xx`) otherwise. This covers document-controlled
paths: include targets, figure sources, toc entries, `exports[].article`.

**Symlinks.** Never dereference. The current `shutil.copy2` follows them —
reproduced: a symlink to a secrets file was materialized into the output as a
regular file containing the secret. Skip symlinks with a diagnostic, or recreate
them as symlinks; never copy target content.

**Output containment.** If the output root is a descendant of the input root,
exclude it from **both** the discovery walk and the asset walk. Without this each
run copies the previous output one level deeper — already on disk as
`article-template/docs-quarto/docs-quarto/`, complete with a duplicated 895 KB
`banner.png`.

**Assets.** Copy non-Markdown, non-config files to the output. Refresh when the
source differs by mtime-or-hash; the current `if not os.path.exists(dst)` means a
user who fixes `references.bib` and re-converts silently keeps the stale copy.
Assets are what make `quarto render` work at all, so Phase 8's headline criterion
depends on this section existing.

**Atomicity.** Write each output to a temp file in the destination directory and
`rename` it. On any error, abort **before** deleting any source. The current
implementation writes files one by one, appends errors, continues, and exits 1
only after everything is already on disk.

**`--in-place`.** Fully specified, because it deletes data:
- Refuse to delete a source unless its output was written and renamed successfully.
- Never overwrite a hand-authored `myst.yml`/`_quarto.yml` without `--force`;
  write alongside and diff instead.
- Require a clean VCS state or `--force`.
- Reproduced hazard: `myst2quarto --in-place` then `quarto2myst --in-place` on
  the fixture shrank `myst.yml` from 1308 to 532 bytes, losing eleven keys, and
  deleted and regenerated the sources — exit 0, zero warnings.

**`--dry-run`.** Writes zero bytes on every path, including the label and
preservation sidecars. Asserted by hashing the tree before and after, for every
flag combination.

**Includes.** Resolve against the canonicalized project root; reject escaping or
absolute targets. Cap depth at a documented constant and detect cycles by
canonical-path set.

### Dependencies

| Crate | Purpose | Rationale |
|---|---|---|
| `clap` (derive) | CLI | Declarative flag surface |
| `saphyr` / `saphyr-parser` | YAML **reading** only | Safe, pure-Rust, no alias expansion. Its emitter is never used |
| `toml` | `mappings.toml` | Required by Phase 2 if its gate said go — not free, and absent from the original table |
| `serde` / `serde_json` | Sidecars | `.mystquarto/labels.json`, `preserved.json` |
| `regex` | Inline patterns | Structure is handled by the IR; inline text still uses regex |
| `walkdir` | Discovery | With symlink following **disabled** |
| `anyhow` / `thiserror` | Errors | `thiserror` in core, `anyhow` in cli |
| `similar`, `insta` (dev) | Test diffs, snapshots | Readable corpus failures |

**Not used: `pulldown-cmark` / `comrak` / `markdown-rs`.** They parse CommonMark;
MyST directives and Quarto divs/shortcodes are not CommonMark, and none can
round-trip source faithfully. A purpose-built block scanner over the ~30
constructs in reference §2 is smaller and exact.

### The IR

```rust
pub struct Document {
    pub frontmatter: Option<Frontmatter>,  // original text + parsed view
    pub blocks: Vec<Block>,
    pub source: PathBuf,
    pub engine: Option<Engine>,            // knitr | jupyter — Phase 4 records it
}

pub struct Block {
    pub kind: BlockKind,
    pub span: Span,                        // line range in the source
    pub blank_lines_before: u8,            // RT-13: separator fidelity
}

pub enum BlockKind {
    Heading { level: u8, text: String, label: Option<Label> },
    Paragraph { lines: Vec<String> },
    CodeCell { lang: String, options: CellOptions, body: Vec<String>, label: Option<Label> },
    StaticCode { lang: Option<String>, body: Vec<String>, attrs: Attrs },
    Figure { src: FigureSource, caption: Vec<String>, label: Option<Label>, attrs: Attrs },
    Table { caption: Vec<String>, rows: Vec<String>, label: Option<Label> },
    Math { body: Vec<String>, label: Option<Label> },
    Admonition { kind: AdmonitionKind, title: Option<String>, body: Vec<Block>, collapse: Option<bool> },
    TabSet { items: Vec<TabItem> },
    Margin { body: Vec<Block> },
    Include { target: PathBuf, opts: IncludeOpts },
    Embed { target: EmbedTarget, label: Option<Label> },
    Comment { text: String, style: CommentStyle },
    Target { label: Label },
    Raw { format: String, body: Vec<String> },
    BlockBreak,
    Preserved { original: Vec<String>, code: &'static str },   // RT-11
    Unmappable { original: Vec<String>, reason: String },
}

pub enum FigureSource {
    Path(PathBuf),
    CellRef { label: Label, notebook: Option<PathBuf> },       // RT-03
}
```

Four properties the Python design lacks:

1. **`span` on every block** — diagnostics can say `article.md:55` (fixes D12).
2. **`label` on labelable constructs** — the writer knows a label belongs to a
   *figure* and picks `fig-` (fixes D1, D2, D3).
3. **`Preserved`** — a first-class variant with a reader (RT-11). The original
   plan emitted preservation but never parsed it back, making its own
   "preserved originals round-trip" criterion unreachable.
4. **`CellRef.notebook`** — `{{< embed >}}` needs a file path that the MyST
   source does not contain; the original `CellRef(Label)` could not carry it.

`blank_lines_before` exists so same-dialect round-trip can be byte-exact; without
it, separator conventions are unrepresentable (RT-13).

### CLI surface

Unchanged from Python: `-o/--output`, `--in-place`, `--config-only`,
`--no-config`, `--dry-run`, `--strict`.

Added, each with a success criterion: `--force` (required by the in-place
contract above). **Dropped from the original plan:** `--no-preserve`, which
contradicted the accepted "nothing is ever destroyed" decision, and
`--format json`, which duplicated `--strict`'s CI role. `--no-label-map` is
retained only because the sidecar is written by default into the user's tree.

## Related Code Files

- Create: `Cargo.toml`, `rust-toolchain.toml`, `.cargo/config.toml`
- Create: `crates/mystquarto-core/src/lib.rs`, `ir.rs`, `span.rs`, `label.rs`
- Create: `crates/mystquarto-core/src/yaml/mod.rs`, `surgery.rs`, `emit.rs`
- Create: `crates/mystquarto-core/src/fs/path_guard.rs`, `assets.rs`, `atomic.rs`
- Create: `crates/mystquarto/src/main.rs`, `args.rs`, `discover.rs`
- Create: `crates/mystquarto/src/bin/myst2quarto.rs`, `quarto2myst.rs`
- Create: `crates/mystquarto/tests/cli.rs` — **Phase 1 bucket B CLI tests land here**
- Modify: `.github/workflows/ci.yml` — Rust job; `permissions: contents: read`;
  pin third-party actions to commit SHAs
- Modify: `.gitignore` — `/target`, `.mystquarto/`
- Read: `src/mystquarto/convert.py` (the orchestration behavior being specified)

## Implementation Steps

1. Scaffold the workspace with the corrected crate names; pin MSRV;
   `#![forbid(unsafe_code)]`.
2. Define leaf types: `Span`, `Label`, `Attrs`, `CellOptions`, `Engine`.
3. Define `Block` / `BlockKind` per the sketch. Every variant must trace to a
   reference §2 row, or be `Preserved`/`Unmappable`.
4. **YAML, in this order:** write the block-scalar and comment tests first, then
   build line surgery for the frontmatter path, then the bounded emitter for the
   synthesis path. Cover `|`, `|-`, `>`, key order, comments, and
   `open_access: no` reading as `"no"`.
5. Build `path_guard.rs` and its test suite: symlink escape, `..` traversal,
   absolute target, output-inside-input, include cycle, include depth cap.
6. Build `assets.rs` (no symlink following, output excluded, mtime/hash refresh)
   and `atomic.rs` (temp + rename, abort before source deletion).
7. Port discovery with `walkdir`, symlink following disabled, output root excluded.
8. Build the `clap` arg structs and the `--in-place` / `--force` / `--dry-run`
   contract. Port Phase 1 bucket B CLI tests here.
9. Wire the three binaries to a shared `run_conversion` returning "not
   implemented"; Phases 4/5 fill it in.
10. Add the Rust CI job: `cargo fmt --check`, `cargo clippy -- -D warnings`,
    `cargo test`.

## Success Criteria

- [x] `cargo build --release` produces all three binaries from the `mystquarto` crate
- [x] `cargo install --path crates/mystquarto` yields a working `mystquarto`
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [x] YAML: `abstract: |` survives read→write byte-identically (tested against D9's real fixture)
- [x] YAML: key order and comments preserved on the frontmatter path
- [x] YAML: the synthesis emitter produces block scalars and comments deterministically
- [x] YAML: `open_access: no` reads as the string `"no"`
- [x] Path guard refuses: symlink escaping the input root; `..` include traversal;
      absolute include target; include cycle; include beyond the depth cap
- [x] Symlinked assets are never dereferenced — the reproduced secret-exfiltration
      case is refused with a diagnostic
- [x] Converting into an output directory nested inside the input is excluded from
      discovery and asset walks at the primitive level (`effective_output_excluded_from_walk`)
      — full conversion-level proof against the D16 fixture is Phase 8's job
- [x] Assets refresh when the source changes
- [x] A mid-run write failure leaves no source deleted and no truncated output
- [x] `--in-place` refuses to clobber a hand-authored config without `--force`
- [x] `--dry-run` writes zero bytes for every flag combination, verified by a
      recursive tree hash (sidecars don't exist as real features until Phase 5/7;
      no eager sidecar creation exists yet for dry-run to suppress)
- [ ] `discover_files` matches a **frozen expected-list fixture** generated in
      Phase 1 from a clean checkout — **gap**: Phase 1 did not produce a
      discovery-specific frozen fixture (confirmed by grep over `tests/corpus/`).
      Substituted a synthetic in-test directory tree instead. Revisit if a real
      fixture is needed later — does not block Phase 4.
- [x] All Phase 1 bucket B CLI tests pass — 43/43 `test_cli.py` rows accounted
      for: 15 ported as real assertions, 28 `#[ignore]`'d with reasons pending
      Phase 4/5/6/7 (none silently dropped)
- [x] Every `BlockKind` variant maps to a reference §2/§2.1 row, or is
      `Preserved`/`Unmappable`/one of the other pre-allowed non-content variants
      (documented per-variant in `ir.rs`; three variants — `Blockquote`,
      `Theorem`, `Directive` — added beyond the sketch, each citing its §2.1/§2 row)
- [x] `ci.yml` declares `permissions: contents: read` and pins `actions/checkout`
      and `astral-sh/setup-uv` to commit SHAs (both verified against GitHub's tag
      API by the controller before commit)

## Risk Assessment

**Risk: the bounded YAML emitter grows into a general one.** The synthesis path
needs block scalars, comments, key order, nested maps, and sequences.
*Signal:* the emitter accepts inputs outside the reference §8.2 key set.
*Response:* keep it typed against a `QuartoConfig` struct rather than a generic
value tree, so unsupported shapes are a compile error rather than a feature
request. If the key set genuinely needs to be open, revisit — but do not slide
there by accretion.

**Risk: line surgery is fragile on unusual frontmatter.** Multi-document YAML,
tabs, CRLF, or unusual indentation could defeat targeted edits.
*Signal:* surgery tests fail on a corpus frontmatter.
*Response:* fall back to full re-emission through the bounded emitter for that
file, accepting style loss, and warn. Never silently corrupt.

**Risk: path canonicalization differs across platforms.** macOS case-insensitive
filesystems and Windows UNC paths complicate descendant checks.
*Signal:* path-guard tests fail on Windows CI.
*Response:* compare canonicalized paths component-wise rather than by string
prefix, and add Windows to the CI matrix at this phase rather than at Phase 9.

**Risk: the IR is wrong and Phases 4/5 fight it.**
*Signal:* Phase 4 needs an IR change more than twice.
*Response:* accept one revision as normal; on the third, review the design
against reference §2 rather than accreting variants.
