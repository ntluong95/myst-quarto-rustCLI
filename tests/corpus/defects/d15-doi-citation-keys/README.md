# D15 — DOI citation key corrupted (quarto_to_myst)

**Direction note.** D15 was originally catalogued as an M→Q defect citing the
`docs-quarto/` output artifact, where the DOI key is in fact left intact —
that reading would have produced a fixture that matches its own (unfixed)
baseline and passes before any fix exists. The corrected, verified direction
is **quarto_to_myst**; see docs/dialect-comparison.md §12's "A note on D15's
original entry."

**Defect statement.** Modern MyST and Quarto share identical Pandoc-style
citation syntax, including the DOI-as-key form `[@10.1038/nmeth.1974]` (§4,
all ✅ rows) — so the *correct* behavior in both directions is to leave it
completely unchanged. Instead, the Quarto→MyST single-citation regex is too
narrow to match a DOI key (it stops at `[\w-]+`, which excludes `.` and `/`),
so it silently fails to match the well-formed bracketed citation. A second,
broader "bare citation" regex then partially matches inside the unconsumed
text and mangles it.

**Reference-doc section.** docs/dialect-comparison.md §4 "Citations" — "DOI
as key" row (✅ identical syntax in both directions, only the resolution
mechanism differs) and the "Critical" note directly below the table: "The
DOI-as-key form (`@10.1038/nmeth.1974`) contains `.` and `/` and breaks naive
`[\w-]+` patterns." Catalogued as D15 in §12, which also records the
controller's original verification of the exact corrupted string.

**Root cause.** In `src/mystquarto/transforms/quarto_to_myst.py`:
```python
_SINGLE_CITE_RE = re.compile(r"\[@([\w-]+)\]")          # line 18
_BARE_CITE_RE = re.compile(r"(?<!\w)@((?!fig-|eq-|tbl-|sec-)[\w][\w-]*)(?!\w)")  # line 35
```
`_SINGLE_CITE_RE` requires `]` to immediately follow a run of `[\w-]`
characters — for `[@10.1038/nmeth.1974]`, the `.` after `10` breaks the run
before `]` is reached, so the whole bracketed form never matches and the
literal `[` and `]` are left in the output. `_BARE_CITE_RE` then runs next
(quarto_to_myst.py:172-173, inside `transform_quarto_inline`) and matches
just `@10` (its `(?!\w)` lookahead is satisfied because `.` is not a word
character), replacing only that fragment with `` {cite:t}`10` `` and leaving
`.1038/nmeth.1974]` dangling as literal text immediately after.

**Capture command.**

```bash
uv run python -c "
from mystquarto.transforms.quarto_to_myst import convert_quarto_to_myst
text = open('tests/corpus/defects/d15-doi-citation-keys/input.qmd').read()
open('tests/corpus/defects/d15-doi-citation-keys/python-actual.md', 'w').write(convert_quarto_to_myst(text))
"
```

**Verification.** `expected.md` built with `myst build expected.md --md
--force` in a scratch dir. Exit 0, no error-level output; myst's build log
confirms `🪄 Linked 1 DOI from doi.org` — the DOI citation is recognized and
resolved as a genuine citation by myst itself, corroborating that leaving it
unchanged is correct.

**Result vs. prediction.** Matched exactly, and matches the controller's
independently-verified evidence cited in docs/dialect-comparison.md §12:
`python-actual.md` is `` Cite [{cite:t}`10`.1038/nmeth.1974] here. `` — the
DOI key is corrupted into a truncated `{cite:t}` role plus dangling literal
text. **The correct fix is to widen the citation regex to accept `.` and
`/`, not to convert to the legacy `{cite:t}` form** — modern MyST and Quarto
citation syntax are identical here (§4), so `expected.md` is simply the
unchanged input.
