# D16 — Output directory nested inside input is re-converted and re-copied each run (both directions; fixture built myst_to_quarto)

**No `expected.*` / render-verify step applies to this fixture.** Per the
phase-01 spec, D16 is structural (a directory-tree defect, not a text
transform), so `expected.md` states the correct behavior in prose and
`python-actual.txt` is a frozen `find | sort` tree listing rather than a
renderable file pair.

**Defect statement.** When the output directory is (deliberately or
accidentally) chosen to be a subdirectory of the input directory — as
happened for the real `article-template/docs-quarto/` fixture, per §12's
evidence — files inside the output directory get swept up by the *next*
asset-copy pass as if they were part of the input, and copied again one
level deeper. Every re-run compounds the nesting.

**Reference-doc section.** docs/dialect-comparison.md §12 row D16: "Output
directory nested inside input is re-converted and re-copied each run" |
evidence `article-template/docs-quarto/docs-quarto/` (8 duplicated files
incl. a 895 KB `banner.png`) | root cause: `discover_files` (convert.py:69-82)
and `_copy_assets` (convert.py:337-350) keep separate skip-sets, neither
excluding the output dir. Also relevant: phase-01's own threat model (RD-5,
"input repositories are untrusted... document-controlled paths... are
attacker-influenced surfaces") — path-safety handling of the output
directory is a Phase 3 concern that this fixture proves is currently absent.

**Root cause (see `python-actual.txt` for the full trace).** `_copy_assets`
in `src/mystquarto/convert.py:335-368` walks `input_dir` with `os.walk`, and
`docs_dir` skip-sets (`skip_dirs`, convert.py:337-350 and, separately,
`discover_files`'s own copy at convert.py:69-82) list only fixed directory
*names* (`.git`, `node_modules`, `_build`, etc.) — never the actual resolved
output directory path. When that output directory is nested inside the input
tree, `os.walk` descends into it too, and any asset already copied there gets
copied again to `output_dir/<relpath-that-now-includes-output_dir's-own-name>`.
Because `os.makedirs(effective_output_dir)` runs *before* `_copy_assets`
walks the tree (`convert_directory`, convert.py:222-299), this can even
happen within a *single* run — see `python-actual.txt`'s "Discrepancy vs.
prediction" section for the observed (worse-than-predicted) behavior.

**Capture commands (used exactly as shown; this doubles as the reproduction
recipe for `scripts/snapshot-baseline.sh`).**

```bash
find tests/corpus/defects/d16-output-recursion/input_dir -type f | sort

uv run myst2quarto tests/corpus/defects/d16-output-recursion/input_dir \
  -o tests/corpus/defects/d16-output-recursion/input_dir/docs-quarto

find tests/corpus/defects/d16-output-recursion/input_dir -type f | sort

uv run myst2quarto tests/corpus/defects/d16-output-recursion/input_dir \
  -o tests/corpus/defects/d16-output-recursion/input_dir/docs-quarto

find tests/corpus/defects/d16-output-recursion/input_dir -type f | sort
```

The committed `input_dir/` contains only the minimal source
(`myst.yml`, `article.md`, `images/banner.png`); the `docs-quarto/` scratch
output produced by the two runs above was deleted after capturing
`python-actual.txt` so it is not committed as pollution.

**Result vs. prediction — IMPORTANT DISCREPANCY.** The spec predicted clean
output after run 1 and one level of `docs-quarto/docs-quarto/` nesting after
run 2. What was actually observed is worse: **run 1 alone** already produces
`docs-quarto/docs-quarto/images/banner.png`, because the output directory
exists as a live subdirectory of the input tree for the remainder of the
*same* run, and Python's `os.walk` directory-iteration order (not guaranteed
alphabetical) determined that `images/` was visited (and copied into
`docs-quarto/images/`) before `docs-quarto/` itself was walked and its
now-present `images/` subdirectory copied again. Run 2 adds one further
level (`docs-quarto/docs-quarto/docs-quarto/`). Full before/after trees are
recorded in `python-actual.txt`.
