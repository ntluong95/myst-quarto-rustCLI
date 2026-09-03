# D09 — Block scalar style destroyed (both directions; fixture built myst_to_quarto)

**Defect statement.** MyST/Quarto page frontmatter allows a multi-line
`abstract: |` using YAML's literal block-scalar style, which preserves hard
line breaks. §8.4 states this is a **hard requirement**: "YAML style
preservation is a hard requirement. Serializing `abstract: |` through a
style-discarding YAML round-trip produces a single-quoted folded scalar —
technically valid, unreadable, and a large spurious diff." The converter
round-trips frontmatter through plain `yaml.safe_load` + `yaml.dump`, which
has no memory of the original scalar style, so the block-literal style is
destroyed.

**Reference-doc section.** docs/dialect-comparison.md §8.4 "Page-level
frontmatter" — `abstract: |` → `abstract: |`, "block scalar style must be
preserved". Catalogued as D9 in §12.

**Root cause.** `replace_frontmatter` in `src/mystquarto/frontmatter.py:61-79`
(reached via `convert_file` in `src/mystquarto/convert.py:102-169`) calls
`yaml.dump(new_fm, default_flow_style=False, sort_keys=False)`. PyYAML's
stock `Dumper` has no concept of "the original scalar was a block literal" —
it independently decides how to represent the string based on its content,
and for a plain multi-line Python string it chooses a **single-quoted, folded
flow scalar** rather than reproducing `|` block-literal style.

**Capture command.**

```bash
uv run python -c "
from mystquarto.frontmatter import extract_frontmatter, myst_to_quarto_frontmatter
from mystquarto.transforms.myst_to_quarto import convert_myst_to_quarto
import yaml
text = open('tests/corpus/defects/d09-block-scalar-mangled/input.md').read()
fm, body = extract_frontmatter(text)
new_fm = myst_to_quarto_frontmatter(fm)
transformed_body = convert_myst_to_quarto(body)
fm_yaml = yaml.dump(new_fm, default_flow_style=False, sort_keys=False)
output_text = '---\n' + fm_yaml + '---\n' + transformed_body
open('tests/corpus/defects/d09-block-scalar-mangled/python-actual.qmd', 'w').write(output_text)
"
```
(This is exactly the sequence `convert_file` in `convert.py` performs; it was
inlined here rather than calling `convert_file` directly so the fixture
doesn't need to touch the filesystem beyond the two files it owns.)

**Verification.** `expected.qmd` rendered with `quarto render --to html` in a
scratch dir (minimal `_quarto.yml`). Exit 0, no `ERROR` — Quarto's debug
metadata dump confirms the `abstract` block-literal parses back into the
identical three-line string.

**Result vs. prediction.** Matched the *shape* of the prediction ("folded or
quoted scalar") exactly. The precise observed output is a single-quoted flow
scalar with blank-line-separated folding:
```yaml
abstract: 'This is a multi-line abstract

  with a hard line break preserved

  by the block literal style.'
```
Note per the plan: this fixture only needs to prove the Python defect exists.
RT-01/RD-1 already establish the Rust fix direction — a purpose-built,
style-preserving YAML emitter for the known frontmatter key set, not a
general-purpose round-trip YAML library — so no fix approach is prescribed
here.
