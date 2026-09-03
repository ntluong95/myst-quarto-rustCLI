# D12 — Zero warnings ever emitted (both directions; fixture built myst_to_quarto)

**No `expected.*` / render-verify step applies to this fixture.** Per the
phase-01 spec, D12 is bucket-B-shaped (behavioral/CLI defect) even though it
lives under `tests/corpus/defects/`: the correct behavior is described in
this README, not encoded as a renderable file pair.

**Defect statement.** `WarningCollector` (`src/mystquarto/warnings.py`)
exists, has a `.warn()` method, and `cli.py` wires it up to be printed and to
gate `--strict`'s exit code — but nothing in the entire codebase ever calls
`.warn()`. Every lossy, dropped, or ambiguous conversion decision (all of
D1-D11's defects, plus every ⚠️/❌ row in docs/dialect-comparison.md)
happens completely silently. `--strict` is a no-op because there are never
any warnings to promote to errors.

**Reference-doc section.** docs/dialect-comparison.md §12 row D12: "`Zero
warnings emitted for any of the above` | `warnings.py:31` is the only
`.append`; no transform calls it; `cli.py:63` prints success unconditionally
| `WarningCollector` exists but is never populated". Also relevant: §11
"Unmappable inventory" policy table, which specifies `--strict` should fail
on Warning-class diagnostics and `--strict=all` on the expected-lossy class —
none of which can happen while nothing ever calls `.warn()`.

**Root cause.** Grep confirms it: `warnings.py`'s `WarningCollector.warn()`
(warnings.py:19-31) is the only place `self.warnings.append(...)` happens.
`grep -rn "\.warn(" src/mystquarto/transforms src/mystquarto/config.py
src/mystquarto/frontmatter.py src/mystquarto/convert.py` returns nothing —
none of the transform, config, or frontmatter modules import or construct a
`WarningCollector`, or return any per-field warning through
`ConversionResult.warnings` (`convert.py:35`, always left `[]`). `cli.py`'s
`_run_conversion` (cli.py:13-69) constructs a `WarningCollector` and would
correctly report/gate on it if anything populated it — the collector and CLI
wiring are correct; the producer side simply doesn't exist.

**Reused fixture.** This fixture reuses D10's `input_dir/myst.yml` verbatim
(copied, not referenced across directories, per the spec) because it has
exactly 8 silently-dropped/overwritten fields — a correct implementation
must warn on every one of them, and `--strict` must then make the run fail.

**Capture command.**

```bash
uv run myst2quarto tests/corpus/defects/d12-silent-warnings/input_dir \
  -o /tmp/d12-out --strict
```

Output captured verbatim (stdout+stderr, plus exit code) in
`python-actual.txt`.

**Result vs. prediction.** Matched exactly — exit code 0, output is just
`Converted 1 file(s).` with no warning text of any kind about the 8
dropped/overwritten fields, despite `--strict` being passed. A correct
implementation must instead: emit >= 8 distinct warnings (one per
dropped/overwritten field: `subtitle`, `short_title`, `id`, `open_access`,
`venue`, `banner`, `abbreviations`, `site.template`, plus a warning that
`description` was overwritten by `subject`), and under `--strict`, exit
non-zero.
