# mystquarto

Bidirectional **modern MyST (mystmd v1)** ↔ Quarto converter. Transforms
directives, roles, config files, and frontmatter between the two formats
through a typed document IR — not a regex line-scanner — so label
normalization, cross-references, and citations survive the round trip.

> **Coming from the Python version?** `pip install mystquarto` / `uvx
> myst2quarto` are discontinued as of 0.2.0. See
> [`docs/migration-from-python.md`](docs/migration-from-python.md).

## Installation

```bash
cargo install mystquarto
```

Or without a local install:

```bash
npx mystquarto to-quarto docs/
```

Or grab a prebuilt binary for macOS (arm64/x64), Linux (x64/arm64/musl), or
Windows (x64) from the [Releases page](https://github.com/ntluong95/myst-quarto-rustCLI/releases).

## Usage

```bash
# Convert MyST → Quarto
myst2quarto docs/ -o docs-quarto/

# Convert Quarto → MyST
quarto2myst docs/ -o docs-myst/

# Unified CLI
mystquarto to-quarto docs/
mystquarto to-myst docs/
```

### Options

| Flag | Description |
|---|---|
| `-o DIR` / `--output DIR` | Output directory (default: `<input>-quarto/` or `<input>-myst/`) |
| `--in-place` | Modify files in-place (requires `--force`, refuses on a dirty VCS state) |
| `--force` | Bypass the `--in-place` overwrite and clean-VCS-state gates |
| `--config-only` | Only convert config files (`myst.yml` ↔ `_quarto.yml`) |
| `--no-config` | Skip config file conversion |
| `--dry-run` | Show what would change; writes zero bytes |
| `--strict[=warn\|all]` | `--strict` fails the run on Warning+ diagnostics; `--strict=all` also fails on expected-lossy conversions |

Every lossy or ambiguous conversion prints a `file:line` diagnostic with a
stable code — see [`docs/diagnostics.md`](docs/diagnostics.md). Nothing is
silently dropped: unmappable constructs are preserved verbatim in
`.mystquarto/preserved.json` and restored on the reverse conversion; label
renames are tracked in `.mystquarto/labels.json` so `fig:samples` ↔
`fig-samples` round-trips.

## What it converts

### Block directives

| MyST | Quarto |
|---|---|
| `` ```{code-cell} python `` | `` ```{python} `` |
| `:tags: [remove-input]` | `#\| echo: false` |
| `:tags: [remove-output]` | `#\| output: false` |
| `:tags: [remove-cell]` | `#\| include: false` |
| `:tags: [hide-input]` | `#\| code-fold: true` |
| `` ```{figure} path `` | `![caption](path){#fig-id width=X}` |
| `` ```{math} `` + `:label:` | `$$ ... $$ {#eq-id}` |
| `` ```{note} `` | `::: {.callout-note}` |
| `` ```{warning} `` | `::: {.callout-warning}` |
| `` ```{tip} `` | `::: {.callout-tip}` |
| `` ```{important} `` | `::: {.callout-important}` |
| `` ```{admonition} Title `` | `::: {.callout-note title="Title"}` |
| `::::{tab-set}` / `:::{tab-item}` | `::: {.panel-tabset}` / `## Label` |
| `` ```{margin} `` | `::: {.column-margin}` |
| `` ```{image} url `` | `![alt](url){width=X}` |
| `` ```{table} Caption `` | Markdown table + `: Caption {#tbl-id}` |
| `` ```{bibliography} `` | Removed (Quarto handles via config) |
| `` ```{tableofcontents} `` | Removed (Quarto handles via config) |
| `` ```{mermaid} `` | Pass through (both support it) |

### Inline roles

| MyST | Quarto |
|---|---|
| `` {eval}`expr` `` | `` `{python} expr` `` |
| `` {cite}`key` `` | `[@key]` |
| `` {cite:t}`key` `` | `@key` |
| `` {cite:p}`key` `` | `[@key]` |
| `` {cite}`a,b,c` `` | `[@a; @b; @c]` |
| `` {numref}`fig-id` `` | `@fig-id` |
| `` {ref}`label` `` | `@label` |
| `` {eq}`label` `` | `@eq-label` |
| `` {doc}`path` `` | `[path](path.qmd)` |

### Config files (`myst.yml` ↔ `_quarto.yml`)

| `myst.yml` | `_quarto.yml` |
|---|---|
| `project.title` | `title:` or `book.title:` |
| `project.authors` | `author:` |
| `project.bibliography` | `bibliography:` |
| `project.toc` | `book.chapters:` |
| `site.template: book-theme` | `project.type: book` |
| `project.exports[format: pdf]` | `format.pdf:` |

### Frontmatter (per-file YAML)

| MyST | Quarto |
|---|---|
| `kernelspec: {name: python3}` | `jupyter: python3` |
| `label:` | `id:` |
| `exports:` | `format:` |

## Architecture

```
                    ┌──────────────────┐
   .md (MyST)  ───► │   MystReader     │ ─┐
                    └──────────────────┘  │
                                          ├──►  ┌─────────┐  ──┬─► MystWriter  ──► .md
                    ┌──────────────────┐  │     │ DocIR   │    │
  .qmd (Quarto) ──► │  QuartoReader    │ ─┘     │ + spans │    └─► QuartoWriter ──► .qmd
                    └──────────────────┘        └─────────┘
                                                     │
                                    ┌────────────────┼────────────────┐
                                    │                │                │
                            ┌───────────────┐ ┌─────────────┐ ┌──────────────┐
                            │ LabelRegistry │ │ Diagnostics │ │ PathGuard    │
                            │ (run-scoped)  │ │ file:line   │ │ containment  │
                            └───────────────┘ └─────────────┘ └──────────────┘
```

Both readers parse into a typed `Document` IR that knows what kind of
construct each block and label is (a figure needs `fig-`, a table needs
`tbl-`) before either writer runs. `PathGuard` refuses symlink escapes,
`..` include traversal, include cycles, and output-inside-input recursion.
See [`docs/dialect-comparison.md`](docs/dialect-comparison.md) for the full
MyST/Quarto construct reference this implementation targets.

Crate layout (Cargo workspace):

| Crate | Responsibility |
|---|---|
| `mystquarto-core` | IR, readers, writers, label registry, diagnostics, config/frontmatter, path guard |
| `mystquarto` | `clap` CLI, file discovery, orchestration, reporting — the published binary crate |

## Development

```bash
git clone https://github.com/ntluong95/myst-quarto-rustCLI
cd myst-quarto-rustCLI
cargo test --workspace                                       # unit + corpus + round-trip
cargo test -p mystquarto --features renderer-tests --test renderer  # needs quarto + myst installed
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## License

MIT-0
