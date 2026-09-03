# Defect corpus

Direction-aware, reproducing fixtures for the 16 defects (17 directories,
since D13 spans both directions) catalogued in
`docs/dialect-comparison.md` §12 "Verified defects in the Python
implementation". Built for Phase 1 of
`plans/260903-1749-rust-port-dialect-fidelity/phase-01-baseline-audit.md`.

## Layout

```
tests/corpus/defects/d01-<slug>/
  direction            # exactly "myst_to_quarto" or "quarto_to_myst"
  input.md  (or .qmd)  # matches the READ side of the direction
  expected.md (or .qmd)   # hand-written, then renderer-verified
  python-actual.md (or .qmd)  # captured by actually running the Python code
  README.md            # defect statement, root cause, capture command,
                        # reference-doc section, and result-vs-prediction note
```

`myst_to_quarto` direction → `input.md` + `expected.qmd` +
`python-actual.qmd`. `quarto_to_myst` direction → `input.qmd` +
`expected.md` + `python-actual.md`.

**Exceptions to the file-extension convention**, each stated again in its own
README:

- **D06, D07, D08, D10** (config-level defects in `myst.yml` ↔ `_quarto.yml`)
  use `.yml` throughout — `input.yml` / `expected.yml` / `python-actual.yml`
  — plus a full `input_dir/myst.yml` that drives the real conversion. There
  is no MyST/Quarto document render check for these; instead `expected.yml`
  is confirmed to be valid YAML and to produce a non-erroring `quarto
  render` when used as `_quarto.yml` for a trivial one-page project.
- **D12** (silent warnings) and **D16** (output-directory recursion) are
  behavioral/structural defects, not text transforms. Neither has a
  render-verified `expected.*`; each README states the correct behavior in
  prose, and `python-actual.txt` holds the captured CLI output (D12) or
  `find | sort` directory tree (D16) instead of a matching-extension pair.
- **D13** spans both directions (docs/dialect-comparison.md §12's note on
  D13/D14/D15), so it is split into `d13a-include-myst-to-quarto/` and
  `d13b-include-quarto-to-myst/`, each a normal single-direction fixture,
  both referencing "D13" in their READMEs.

## Directory index

| Directory | Direction | Defect (one line) | Capture method |
|---|---|---|---|
| `d01-colon-labels-unnormalized` | myst_to_quarto | `@fig:samples` / `{#eq:chi-squared}` labels pass through with colons, unnormalized | `convert_myst_to_quarto()` direct call |
| `d02-figure-label-dropped` | myst_to_quarto | `:::{figure}` reads `:name:` not `:label:`, so the id is dropped entirely | `convert_myst_to_quarto()` direct call |
| `d03-table-caption-label-lost` | myst_to_quarto | `:::{table}` caption (directive body) and label (`:label:`, read as `:name:`) both lost | `convert_myst_to_quarto()` direct call |
| `d04-percent-comments-literal` | myst_to_quarto | `% comment` lines emitted as literal visible text, not `<!-- -->` | `convert_myst_to_quarto()` direct call |
| `d05-heading-target-literal` | myst_to_quarto | `(sec:x)=` heading target emitted as literal visible text | `convert_myst_to_quarto()` direct call |
| `d06-export-template-only` | myst_to_quarto | Export with only `template:` (no `format:`) produces invalid `format: {}` | `convert_myst_config()` direct call (`.yml` exception) |
| `d07-notebook-chapter-extension` | myst_to_quarto | `.ipynb` toc entries get `.qmd` appended onto them (`analysis.ipynb.qmd`) | `convert_myst_config()` direct call (`.yml` exception) |
| `d08-article-mistyped-book` | myst_to_quarto | Any `project.toc` presence is treated as `book`, ignoring `site.template: article-theme` | `convert_myst_config()` direct call (`.yml` exception) |
| `d09-block-scalar-mangled` | myst_to_quarto | `abstract: \|` block-literal YAML style destroyed by a style-discarding `yaml.dump` round-trip | Inlined `extract_frontmatter` + `myst_to_quarto_frontmatter` + `yaml.dump`, matching `convert_file`'s exact sequence |
| `d10-config-fields-dropped` | myst_to_quarto | 8 config fields silently dropped; `description` silently overwritten by `subject`'s value | `convert_myst_config()` direct call (`.yml` exception) |
| `d11-notebook-embed-broken-link` | myst_to_quarto | `:::{figure} #nb:analysis` treated as a literal image path instead of an embed shortcode | `convert_myst_to_quarto()` direct call |
| `d12-silent-warnings` | myst_to_quarto | `WarningCollector.warn()` is never called anywhere; `--strict` is a no-op | Full CLI: `uv run myst2quarto ... --strict`, stdout+stderr+exit code captured verbatim (no `expected.*`; behavioral) |
| `d13a-include-myst-to-quarto` | myst_to_quarto | `` ```{include} file.md ``` `` unhandled, falls to the generic unknown-directive passthrough | `convert_myst_to_quarto()` direct call |
| `d13b-include-quarto-to-myst` | quarto_to_myst | `{{< include _file.qmd >}}` shortcode has no matching regex anywhere, passes through unchanged | `convert_quarto_to_myst()` direct call |
| `d14-knitr-inline-unhandled` | quarto_to_myst | `` `r expr` `` (knitr inline) unrecognized — only `` `{python} expr} `` matches | `convert_quarto_to_myst()` direct call |
| `d15-doi-citation-keys` | quarto_to_myst | `[@10.1038/nmeth.1974]` corrupted into `` [{cite:t}`10`.1038/nmeth.1974] `` by a `[\w-]+`-only citation regex | `convert_quarto_to_myst()` direct call |
| `d16-output-recursion` | myst_to_quarto | Output dir nested inside input dir gets swept up and re-copied by `_copy_assets` on every run (even the first) | Full CLI run twice with `-o` pointing inside `input_dir`; `find \| sort` tree captured before/after each run (no `expected.*`; structural) |

## Reproducing `python-actual.*` (spec for `scripts/snapshot-baseline.sh`)

**Body-only fixtures** (all except D06-D12, D16) call the transform function
directly — the same oracle the existing `tests/test_*.py` suite uses:

```bash
# myst_to_quarto direction
uv run python -c "
from mystquarto.transforms.myst_to_quarto import convert_myst_to_quarto
text = open('tests/corpus/defects/<dir>/input.md').read()
open('tests/corpus/defects/<dir>/python-actual.qmd', 'w').write(convert_myst_to_quarto(text))
"

# quarto_to_myst direction
uv run python -c "
from mystquarto.transforms.quarto_to_myst import convert_quarto_to_myst
text = open('tests/corpus/defects/<dir>/input.qmd').read()
open('tests/corpus/defects/<dir>/python-actual.md', 'w').write(convert_quarto_to_myst(text))
"
```

**D09** additionally exercises frontmatter, inlining `convert_file`'s exact
sequence (`extract_frontmatter` → `myst_to_quarto_frontmatter` →
`convert_myst_to_quarto` on the body → `yaml.dump` → reassemble) so the
fixture doesn't need to touch the filesystem beyond its own two files. See
`d09-block-scalar-mangled/README.md` for the literal command.

**D06, D07, D08, D10** (config) call `convert_myst_config` directly:

```bash
uv run python -c "
from mystquarto.config import convert_myst_config
convert_myst_config('tests/corpus/defects/<dir>/input_dir/myst.yml', '/tmp/<dir>-capture')
"
# then copy /tmp/<dir>-capture/_quarto.yml -> tests/corpus/defects/<dir>/python-actual.yml
```

**D12** runs the real CLI end-to-end (it needs the full `WarningCollector` +
`--strict` wiring exercised, not just the transform function):

```bash
uv run myst2quarto tests/corpus/defects/d12-silent-warnings/input_dir \
  -o /tmp/d12-out --strict
```
(stdout+stderr and exit code captured into `python-actual.txt`; `/tmp/d12-out`
is scratch and not committed.)

**D16** runs the real CLI twice with an explicit `-o` pointing inside the
input directory, capturing `find <input_dir> -type f | sort` before and
after each run:

```bash
find tests/corpus/defects/d16-output-recursion/input_dir -type f | sort
uv run myst2quarto tests/corpus/defects/d16-output-recursion/input_dir \
  -o tests/corpus/defects/d16-output-recursion/input_dir/docs-quarto
find tests/corpus/defects/d16-output-recursion/input_dir -type f | sort
uv run myst2quarto tests/corpus/defects/d16-output-recursion/input_dir \
  -o tests/corpus/defects/d16-output-recursion/input_dir/docs-quarto
find tests/corpus/defects/d16-output-recursion/input_dir -type f | sort
```
(the resulting `docs-quarto/` scratch output is deleted after capture; only
`python-actual.txt` and the minimal `input_dir/` source are committed.)

## Reproducing the `expected.*` render checks

**`expected.qmd` (Quarto side):** scratch dir with a minimal `_quarto.yml`
(`project: {type: default}`) alongside a copy of `expected.qmd`, then
`quarto render expected.qmd --to html`. Confirm exit 0 and no `ERROR` in
output (a `WARNING` about an unresolved crossref, as in D01, is expected and
acceptable — the fixture is a single-paragraph fragment with no actual
figure defined).

**`expected.md` (MyST side):** scratch dir with `version: 1\nproject: {}`
in `myst.yml` alongside a copy of `expected.md`, then
`myst build expected.md --md --force`. Confirm exit 0. (`myst build
expected.md` alone, without an export flag, reports "No file exports found"
and does nothing — `--md --force` is required to actually exercise the
build; `version: 1` is required in `myst.yml` or the CLI refuses to load the
config at all.)

**D06/D07/D08/D10 (config `.yml` fixtures):** `yaml.safe_load` for validity,
then use `expected.yml` as `_quarto.yml` for a trivial one-page project
(`index.qmd`, plus whatever placeholder assets the config references — e.g.
D10's `banner.png`) and `quarto render`. Confirm exit 0, no `ERROR`. D07's
book-type config additionally needed a wrapper `index.qmd` prepended to the
committed `chapters:` list *for the render check's scratch copy only* — a
Quarto book project requires an explicit home page — the committed
`expected.yml`'s `chapters:` stays exactly `[intro.qmd, analysis.ipynb]`.

**D11 and D13a** needed extra wrapper files that exist only in the scratch
verify directory, never in the committed fixture: D11's embed shortcode
needs an `analysis.ipynb` with a cell carrying `"label":
"fig-analysis-output"` metadata *and* a pre-populated `outputs` array (Quarto
embeds pre-existing notebook output; it does not execute notebooks for this
check). D13a's `{{< include _intro.qmd >}}` needs a placeholder `_intro.qmd`
to exist alongside it, or the include target itself becomes a render error.
D13b's mirror-image `` ```{include} intro.md ``` `` similarly needed a
placeholder `intro.md` for the `myst build` check.

**D12 and D16** have no render-verify step — see each README for why.

## Known deviations from the controller's predictions

All 17 fixtures matched the controller's predicted `python-actual.*` output
exactly, **except D16**, where actual behavior was worse than predicted: the
spec predicted the `docs-quarto/docs-quarto/` nesting would only appear
after a *second* run; in fact it already appears after the *first* run,
because the output directory exists as a live subdirectory of the input tree
for the remainder of the same run in which it's created, and `os.walk`'s
directory-iteration order (not guaranteed alphabetical) determines how much
nesting a single asset-copy pass produces. See
`d16-output-recursion/README.md` and `d16-output-recursion/python-actual.txt`
for the full trace.
