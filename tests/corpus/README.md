# tests/corpus/

Data-driven test fixtures extracted from the Python test suite (`tests/test_*.py`)
as part of Phase 1 (baseline audit) of the Rust port. See
`plans/260903-1749-rust-port-dialect-fidelity/phase-01-baseline-audit.md`.

## Layout

- `classification.md` — one row per Python test function (225 total), bucketed:
  - **A — text pair**: full-text `input == expected` equality. Extracted here.
  - **B — behavioral**: CLI exit codes, filesystem effects, dict/object
    assertions. Stays as a Rust integration test, not extracted as data.
  - **C — substring/phantom**: fragment (`"x" in result`) or too-weak
    assertions. Ported as hand-written Rust assertions, or flagged for
    deletion if phantom.
- `parity/` — one directory per extracted bucket-A case.
- `defects/` — direction-aware reproductions of the 16 cataloged conversion
  defects (D1–D16), built separately under phase-01 steps 6–9.

## `parity/<file-stem>__<test-name>/` convention

Each bucket-A test becomes `parity/<file-stem>__<test-name>/`:

- `<file-stem>` — source file without extension, e.g. `test_inline`.
- `<test-name>` — the test function's own name, unqualified (no class
  prefix), e.g. `test_basic_eval`.
- Example: `test_inline.py::TestEvalRole::test_basic_eval` becomes
  `parity/test_inline__test_basic_eval/`.

This traces every fixture back to the exact test it was extracted from.

## Input/expected extension convention (direction-matched)

| Direction | Input | Expected |
|---|---|---|
| MyST → Quarto | `input.md` | `expected.qmd` |
| Quarto → MyST | `input.qmd` | `expected.md` |

Content is copied **verbatim** from the Python test's literal string (or
fixture file) — no reformatting, no added trailing newline.

Some bucket-A tests pointed at a real fixture file under `tests/fixtures/`
instead of an inline string; those are **not** duplicated here.
`classification.md` notes the fixture path directly instead (see
`test_directives.py::test_simple_fixture` and
`test_roundtrip.py::test_quarto_fixture`).

## Adding a new case

1. Pick the direction and use the matching extension pair above.
2. Create `parity/<file-stem>__<test-name>/` (or a descriptive kebab-case
   name if there is no single source test).
3. Write `input.*` and `expected.*` verbatim — verify the expectation
   actually renders/builds before committing it.
4. Add a row to `classification.md` if it traces back to a Python test.
