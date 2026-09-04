# Migrating from the Python implementation

`mystquarto` 0.2.0 is a full Rust rewrite. The Python distribution
(`pip install mystquarto`, `uvx myst2quarto`) is discontinued. This page
covers what changed and how to move over.

## Install

| Before | Now |
|---|---|
| `pip install mystquarto` | `cargo install mystquarto` |
| `uvx myst2quarto docs/` | `npx mystquarto to-quarto docs/` |
| — | Prebuilt binary from the [Releases page](https://github.com/ntluong95/myst-quarto-rustCLI/releases) |

The CLI surface is unchanged: `myst2quarto`, `quarto2myst`, and
`mystquarto to-quarto`/`to-myst` all still exist with the same flags
(`-o`, `--in-place`, `--config-only`, `--no-config`, `--dry-run`,
`--strict`). One addition: `--force`, now required alongside `--in-place`
(see below).

## Behavior changes

### Target dialect is modern mystmd v1

The Python tool targeted legacy Sphinx-role MyST (`` {cite}`key` ``,
`` {numref}` ``, `:name:`). The Rust tool still **reads** that syntax, but
**writes** only modern mystmd v1 syntax (`@fig:samples`, `:label:`,
`(sec:x)=`, `%` comments). If your source documents already use modern
syntax, output is unaffected. If they use legacy syntax, a round trip
normalizes it — this is intentional, not a regression: legacy MyST is what
made every cross-reference in a real mystmd v1 project (like the
`article-template/` fixture this project was built against) resolve as
dead links.

### Labels normalize, and now restore

`fig:samples` becomes `fig-samples` on write (Quarto requires the
`fig-`/`tbl-`/`eq-`/`sec-` prefix convention to resolve cross-references).
The Python tool did this inconsistently; the Rust tool does it uniformly
and records the mapping in `.mystquarto/labels.json`, so converting back
restores your original label spelling. Delete that file (or pass
`--no-label-map`, currently a no-op reserved for a future release) if you
don't want it.

### Nothing is silently dropped

Any construct with no equivalent in the target dialect is preserved
verbatim in `.mystquarto/preserved.json` and a single-line marker is left
in its place; converting back restores the original. The Python tool
sometimes dropped these constructs outright. A diagnostic is always printed
when this happens — see [`diagnostics.md`](diagnostics.md).

### New diagnostics and `--strict` semantics

Every lossy or ambiguous conversion now prints a `file:line` diagnostic
with a stable code (`MQ0xxx`) and one of four severities: Error, Warning,
LossyExpected, Info.

- `--strict` fails the run on Warning and above.
- `--strict=all` additionally fails on LossyExpected — for a
  fully-faithful-conversion-or-nothing gate.
- A correct conversion of a real project exits 0 under `--strict`, because
  dialect differences that are inherently lossy (not defects) are classed
  `LossyExpected`, not `Warning`.

See [`diagnostics.md`](diagnostics.md) for the full code list, and
`.mystquarto/suppress.toml` to baseline specific codes project-wide.

### `--in-place` now requires `--force`

The Python tool would overwrite files in place unconditionally. The Rust
tool refuses unless `--force` is passed and the working tree is under
version control with a clean state — an unconditional in-place overwrite of
hand-authored config was judged too easy to trigger accidentally.

### Path safety

Symlinks that escape the input tree, `..` include traversal, include
cycles, include depth over the limit, and writing output inside the input
tree are all refused with a diagnostic. The Python tool followed all of
these silently.

### DOI citations resolve without network access at convert time

If a citation key isn't in your `.bib` file, the Rust tool synthesizes a
fallback bibliography entry from a CSL-JSON cache rather than leaving the
citation as a literal `@key` in the output.

## What didn't change

- The block-directive and inline-role mapping tables (see
  [`README.md`](../README.md#what-it-converts)) are the same conversions,
  just implemented against a typed IR instead of a regex scanner.
- Config file (`myst.yml` ↔ `_quarto.yml`) and frontmatter mapping keys are
  unchanged.
- `.ipynb` notebooks are still copied with labels patched, not converted
  cell-by-cell.

## If you relied on undocumented Python behavior

The Python source tree is preserved at the `pre-rust-port` git tag for
reference. The 225-case legacy pytest suite and the corpus's recorded
`python-actual.*` files document what the old implementation actually did,
including the 16 defects this rewrite fixes (see
[`dialect-comparison.md` §12](dialect-comparison.md)). If you find behavior
that changed and shouldn't have, open an issue with a minimal reproduction.
