# MyST ↔ Quarto Dialect Comparison

Authoritative mapping reference for the `mystquarto` converter. Every conversion
rule the implementation applies must trace to a row in this document.

**Target dialects**

| Side | Dialect | Version verified | Renderer |
|---|---|---|---|
| MyST | mystmd v1.x (**modern**) | `myst v1.10.1` | `myst build` |
| Quarto | Quarto 1.4+ | `quarto 1.9.36` | `quarto render` |

> **Legacy MyST is read-only.** Sphinx / Jupyter Book v1 constructs
> (`` {cite}`key` ``, `` {numref}`fig-x` ``, `:name:`) are accepted when *reading*
> MyST, because mystmd still parses them, but the writer never emits them.
> See [Legacy read-only surface](#legacy-read-only-surface).

Legend: **✅** direct equivalent · **⚠️** lossy or approximate · **❌** no
equivalent (warn + preserve original as comment) · **➖** not applicable.

---

## 1. Core model differences

The two formats disagree at a level deeper than syntax. These four differences
generate most of the mapping complexity downstream.

| Dimension | MyST (mystmd v1) | Quarto | Consequence for the converter |
|---|---|---|---|
| Extension mechanism | **Directives + roles** — an open, named vocabulary (`` ```{figure} ``, `` {abbr}`X` ``) inherited from reStructuredText | **Pandoc divs + spans + shortcodes** — `::: {.class}`, `[x]{.class}`, `{{< shortcode >}}` | Directive names map to div classes only where a class exists; otherwise ❌ |
| Label namespace | Free-form. Any string. Convention is `kind:name` (`fig:samples`) | **Constrained.** ID *must* begin with a registered type prefix, hyphen-separated (`fig-`, `tbl-`, `eq-`, `sec-`) | Colon→hyphen normalization is mandatory in one direction and unrecoverable without a sidecar map. See §3 |
| Execution engine | **Jupyter kernels only** (via `kernelspec`) | **Two engines**: knitr (R-native) and Jupyter | knitr-only syntax has no MyST equivalent. See §6 |
| Config unit | `myst.yml` — one file, `project:` + `site:`, publication-metadata rich (`venue`, `open_access`, `doi`, `abbreviations`) | `_quarto.yml` — `project.type` selects a whole rendering pipeline (`default`/`website`/`book`/`manuscript`) | Project-type inference is a judgement call, not a field copy. See §8 |

### Fence styles

Both dialects accept two fence characters, but assign them different meanings.

| Form | MyST | Quarto |
|---|---|---|
| `` ```{name} `` … `` ``` `` | Directive (any) | **Executable code cell only** (`{python}`, `{r}`, `{ojs}`, …) |
| `:::{name}` … `:::` | Directive (any) — identical semantics to backtick form | ➖ (Quarto uses `::: {.class}`, not `:::{name}`) |
| `::: {.class}` … `:::` | Pandoc div — passed through, class ignored unless MyST knows it | Div — the primary block-extension mechanism |

Nesting in both: the outer fence must use **more** fence characters than the
inner one (`::::` wraps `:::`). MyST directive options are `:key: value` lines
immediately after the opening fence; Quarto uses inline `{.class key="value"}`
attributes or `#|` cell comments.

---

## 2. Block constructs

| Construct | MyST | Quarto | Fidelity | Notes |
|---|---|---|---|---|
| Executable cell | ```` ```{code-cell} python ```` | ```` ```{python} ```` | ✅ | Language moves from argument to fence label |
| Cell — hide input | `:tags: [remove-input]` | `#\| echo: false` | ✅ | |
| Cell — hide output | `:tags: [remove-output]` | `#\| output: false` | ✅ | |
| Cell — hide both | `:tags: [remove-cell]` | `#\| include: false` | ✅ | |
| Cell — fold input | `:tags: [hide-input]` | `#\| code-fold: true` | ⚠️ | MyST hides; Quarto collapses. Not identical UX |
| Cell — caption | `:caption: text` | `#\| fig-cap: "text"` | ✅ | |
| Cell — label | `:label: fig:x` | `#\| label: fig-x` | ✅ | Subject to §3 normalization |
| Static code | ```` ```{code} python ```` + `:filename:` `:linenos:` | ```` ```python ```` + `{filename="…"}` | ⚠️ | `:linenos:`/`:emphasize-lines:` have partial Quarto analogues (`code-line-numbers`) |
| Figure | `` :::{figure} path `` + `:label:` `:width:` `:alt:` `:align:` | `![caption](path){#fig-id width="X"}` | ✅ | Multi-paragraph captions require Quarto's div form |
| Figure (div form) | ➖ | `::: {#fig-id}` … `:::` | ⚠️ | Required when caption is multi-block; reader must accept it |
| Image (no caption) | `` ```{image} url `` + `:alt:` `:width:` | `![alt](url){width="X"}` | ✅ | |
| Table + caption | `` :::{table} `` + `:label:` + caption paragraph + pipe table | pipe table + `: Caption {#tbl-id}` | ✅ | Caption is the directive **body** in MyST, a trailing line in Quarto |
| `list-table` / `csv-table` | `` ```{list-table} ``, `` ```{csv-table} `` | ➖ | ❌ | No Quarto equivalent. Render to a pipe table + warn |
| Math (labelled) | `` ```{math} `` + `:label: eq:x` | `$$ … $$ {#eq-x}` | ✅ | |
| Math (unlabelled) | `$$ … $$` | `$$ … $$` | ✅ | Identical |
| Inline math | `$x$` | `$x$` | ✅ | Identical |
| Admonition (typed) | `` ```{note} ``, `{warning}`, `{tip}`, `{important}`, `{caution}` | `::: {.callout-note}` … | ✅ | Only these five overlap |
| Admonition (MyST-only) | `{danger}`, `{error}`, `{hint}`, `{seealso}`, `{attention}` | ➖ | ⚠️ | Collapse to nearest of the five: `danger`/`error`→`important`, `hint`/`seealso`/`attention`→`note`. **Lossy — warn** |
| Admonition (custom title) | `` ```{admonition} My Title `` | `::: {.callout-note title="My Title"}` | ✅ | |
| Admonition (collapsible) | `:class: dropdown` + `:open:` | `collapse="true"` / `collapse="false"` | ⚠️ | Inverted polarity: MyST `:open:` true ≙ Quarto `collapse="false"` |
| Tabs | `::::{tab-set}` / `:::{tab-item} Label` | `::: {.panel-tabset}` / `## Label` | ⚠️ | Quarto uses **headings** as tab labels — round-trip collides with real headings at the same level |
| Margin content | `` ```{margin} `` or `` ```{aside} `` | `::: {.column-margin}` | ✅ | MyST `{aside}` and `{margin}` both map here; reverse picks `{aside}` |
| Blockquote + attribution | `` ```{blockquote} `` + `-- Author` | `> quote` + `> — Author` | ⚠️ | Quarto has no first-class attribution node |
| Epigraph / pull-quote | `` ```{epigraph} ``, `` ```{pull-quote} `` | ➖ | ❌ | Warn + preserve |
| Mermaid | `` ```{mermaid} `` | `` ```{mermaid} `` | ✅ | Identical fence |
| iframe | `` ```{iframe} url `` | raw HTML | ⚠️ | Emit raw `<iframe>` + warn |
| Include | `` ```{include} file.md `` | `{{< include _file.qmd >}}` | ⚠️ | See §7 — placement rules differ |
| Notebook output embed | `:::{figure} #nb:cell-label` | `{{< embed nb.ipynb#fig-cell >}}` | ⚠️ | See §7 |
| Bibliography placement | `` ```{bibliography} `` | ➖ (implicit, config-driven) | ⚠️ | Drop the directive; ensure `bibliography:` is set in config |
| TOC placement | `` ```{tableofcontents} `` | ➖ (implicit, config-driven) | ⚠️ | Drop the directive |
| Glossary | `` ```{glossary} `` | ➖ | ❌ | Warn + preserve |
| Grid / card | `` ::::{grid} ``, `` :::{card} `` | `::: {.grid}` / `::: {.card}` | ⚠️ | Quarto classes are Bootstrap, not semantic. Approximate |
| Proof / theorem | `` ```{prf:theorem} `` | `::: {#thm-x .theorem}` | ⚠️ | Requires Quarto `crossref` theorem config |

---

## 3. Cross-references and labels — the critical divergence

This section drives the single largest class of defects. **Every cross-reference
in the `article-template` fixture is currently emitted broken.**

### 3.1 Defining a label

| Target | MyST | Quarto |
|---|---|---|
| Heading | `(sec:data-analysis)=` on the line **before** the heading | `## Heading {#sec-data-analysis}` |
| Figure | `:label: fig:samples` directive option | `{#fig-samples}` attribute |
| Table | `:label: tab:results` directive option | `{#tbl-results}` in the caption line |
| Equation | `:label: eq:chi-squared` directive option | `{#eq-chi-squared}` after closing `$$` |
| Code cell | `#\| label: nb:analysis` | `#\| label: fig-analysis` |
| Arbitrary block | `(my-para)=` before any block | ➖ (no general mechanism) |

### 3.2 Referencing a label

| Intent | MyST | Quarto |
|---|---|---|
| Auto-typed reference | `@fig:samples` | `@fig-samples` |
| Link form | `[](#fig:samples)` | ➖ (use `@`) |
| Custom text | `[see here](#fig:samples)` | `[see here](#fig-samples)` |
| Number only | `[Fig. %s](#fig:samples)` | `[-@fig-samples]` |
| Custom prefix | `` {numref}`Figure %s <fig:x>` `` *(legacy)* | `[Fig @fig-x]` |
| Grouped | ➖ | `[@fig-a; @fig-b]` |

### 3.3 Prefix rules — the incompatibility

**Quarto requires** the identifier to begin with a registered type prefix,
hyphen-separated, and rejects colons:

`fig-` `tbl-` `eq-` `sec-` `lst-` `thm-` `lem-` `cor-` `prp-` `cnj-` `def-`
`exm-` `exr-` `sol-` `rem-` `alg-` `tip-` `nte-` `wrn-` `imp-` `cau-`

Quarto also warns against `_` in IDs (breaks LaTeX/PDF).
**MyST imposes no constraint** — `fig:samples`, `my-figure`, and `x1` are all valid.

### 3.4 Normalization rules (MyST → Quarto)

| MyST label | Quarto ID | Rule |
|---|---|---|
| `fig:samples` | `fig-samples` | `tab:`→`tbl-`, others: `:`→`-` |
| `tab:phenotypic-variation` | `tbl-phenotypic-variation` | **`tab:` → `tbl-`** (not `tab-`) |
| `eq:chi-squared` | `eq-chi-squared` | |
| `sec:data-analysis` | `sec-data-analysis` | |
| `nb:analysis` | `fig-analysis` | Notebook cell → figure prefix, per target usage |
| `my_label` | `my-label` | `_`→`-`, PDF safety |
| `samples` (figure, no prefix) | `fig-samples` | Prefix **injected** from the construct's type |
| `Fig:Samples` | `fig-samples` | Lowercased |

Collisions after normalization (`fig:a-b` and `fig-a-b` both → `fig-a-b`) must be
disambiguated with a numeric suffix and reported as a warning.

### 3.5 Round-trip preservation

Normalization is not injective, so Quarto → MyST cannot recover `fig:samples`
from `fig-samples` by rule alone. The converter writes a sidecar map:

```json
{ "version": 1,
  "direction": "myst_to_quarto",
  "source_root": "article-template",
  "files": {
    "article.md": {
      "content_hash": "sha256:…",
      "labels": { "fig-samples": "fig:samples", "tbl-phenotypic-variation": "tab:phenotypic-variation" }
    }
  }
}
```

Written to `.mystquarto/labels.json` in the output root. When present on the
reverse conversion, original spellings are restored exactly; when absent, the
normalized form is kept and a note is emitted.

Keying by source file is required: a flat `{id: label}` map cannot disambiguate
two files that both define `fig:samples`, so the reverse conversion would restore
the wrong original into one of them. The sidecar sits in the input tree on the
reverse pass, which makes it untrusted input — it is validated for version, size,
entry count, direction, and label character set before use.

---

## 4. Citations

Modern MyST and Quarto both use Pandoc-style citation syntax, so this area is
**mostly identical** — a fact the current Python implementation does not exploit.

| Intent | MyST (v1) | Quarto | Fidelity |
|---|---|---|---|
| Parenthetical | `[@smith2020]` | `[@smith2020]` | ✅ identical |
| Narrative | `@smith2020` | `@smith2020` | ✅ identical |
| Multiple | `[@a; @b]` | `[@a; @b]` | ✅ identical |
| With locator | `[@a, p. 33]` | `[@a, p. 33]` | ✅ identical |
| Suppress author | `[-@a]` | `[-@a]` | ✅ identical |
| DOI as key | `[@10.1038/nmeth.1974]` | `[@10.1038/nmeth.1974]` | ⚠️ MyST resolves DOIs live; Quarto needs the entry in `.bib` |
| Legacy role | `` {cite}`key` `` | ➖ | read-only → emit `[@key]` |
| Legacy narrative | `` {cite:t}`key` `` | ➖ | read-only → emit `@key` |
| Legacy parenthetical | `` {cite:p}`key` `` | ➖ | read-only → emit `[@key]` |

**Critical:** because `@key` and `@fig-x` share a prefix, citation and
cross-reference transforms must be applied in one pass with type-aware
resolution, not as sequential regex substitutions. The DOI-as-key form
(`@10.1038/nmeth.1974`) contains `.` and `/` and breaks naive `[\w-]+` patterns.

---

## 5. Inline constructs

| Construct | MyST | Quarto | Fidelity |
|---|---|---|---|
| Inline eval (Python) | `` {eval}`expr` `` | `` `{python} expr` `` | ✅ |
| Inline eval (R, Jupyter) | `` {eval}`expr` `` | `` `{r} expr` `` | ✅ |
| Inline eval (R, knitr) | ➖ | `` `r expr` `` | ❌ **knitr-only.** See §6 |
| Abbreviation | `` {abbr}`CRISPR (Clustered…)` `` or project `abbreviations:` | ➖ | ❌ Emit raw `<abbr>` + warn |
| Strikethrough | `` {del}`x` `` or `~~x~~` | `~~x~~` | ✅ |
| Underline | `` {u}`x` `` | `[x]{.underline}` | ✅ |
| Small caps | `` {sc}`x` `` | `[x]{.smallcaps}` | ✅ |
| Subscript | `` {sub}`x` `` or `~x~` | `~x~` | ✅ |
| Superscript | `` {sup}`x` `` or `^x^` | `^x^` | ✅ |
| Keyboard | `` {kbd}`Ctrl-C` `` | `[Ctrl-C]{.kbd}` | ⚠️ No Quarto semantic |
| Document link | `[text](./other.md)` | `[text](./other.qmd)` | ✅ Extension rewrite |
| Legacy doc role | `` {doc}`path` `` | ➖ | read-only → `[path](path.qmd)` |
| File link (empty text) | `[](./references.bib)` | `[references.bib](./references.bib)` | ⚠️ MyST auto-fills text; Quarto renders empty |
| Footnote | `[^label]` + `[^label]: text` | `[^label]` + `[^label]: text` | ✅ identical |
| Link ref definition | `[Label]: https://url` | `[Label]: https://url` | ✅ identical |
| Line break | `\` at end of line | `\` at end of line | ✅ identical |

---

## 6. Execution model

The deepest incompatibility. MyST runs **Jupyter kernels only**; Quarto runs
either **knitr** (R-native, `.Rmd` lineage) or **Jupyter**.

| Aspect | MyST | Quarto |
|---|---|---|
| Engines | Jupyter | knitr, Jupyter |
| Engine selection | `kernelspec` in frontmatter | `engine:` key, or inferred from cell languages |
| Python | Jupyter `ipykernel` | Jupyter `ipykernel` |
| R | Jupyter `IRkernel` — **must be installed** | knitr (default for R) — no Jupyter kernel needed |
| Inline R | `` {eval}`x` `` (needs IRkernel) | `` `r x` `` (knitr) or `` `{r} x` `` (Jupyter) |
| Cell options | `:tags:`, `#\| key: value` | `#\| key: value` |

### Conversion consequences

1. **Quarto (knitr, R) → MyST is not executable by default.** The MyST output
   requires `IRkernel` in the environment. The converter must emit a warning and
   add `kernelspec: {name: ir, display_name: R}` to the frontmatter.
2. **`` `r expr` `` inline (knitr) has no MyST form.** Convert to
   `` {eval}`expr` `` **and warn** that IRkernel is required — the semantics are
   equivalent only if a Jupyter R kernel exists.
3. **Escape hatch — pre-render.** When the source project cannot supply the
   target's engine, render to a fully-resolved static Markdown first
   (`quarto render --to gfm`), then convert the static output. This trades
   executability for correctness and is the documented remedy for the
   R-kernel-missing case.
4. **Mixed-engine includes are illegal in Quarto** — all cells in an included
   file must share one engine.

---

## 7. Composition: includes and embeds

| Feature | MyST | Quarto | Fidelity |
|---|---|---|---|
| Include a file | `` ```{include} file.md `` | `{{< include _file.qmd >}}` | ⚠️ |
| Include literal | `:literal:` + `:lang:` | ```` ```{.python include="f.py"} ```` | ⚠️ |
| Include line range | `:start-line:` / `:end-line:` / `:lines:` | `start-line=` / `end-line=` | ✅ |
| Embed notebook output | `:::{figure} #nb:cell` / `` ```{embed} #label `` | `{{< embed nb.ipynb#fig-cell >}}` | ⚠️ |
| Embed shorthand | `![](#label)` | `{{< embed … >}}` | ⚠️ |

**Placement constraints differ and this breaks naive conversion.** Quarto's
`{{< include >}}`:

- must be alone on its line, surrounded by blank lines;
- cannot appear inside markdown syntax (list items, table cells, block quotes);
- resolves relative paths against the **including** file's directory, not its own;
- conventionally names its target with a leading `_` so `quarto render` skips it.

MyST's `{include}` directive has none of these restrictions. Therefore
MyST → Quarto must:

1. rename the target `file.md` → `_file.qmd`;
2. verify the directive is at block level — if it is nested inside a list or
   quote, ❌ warn and preserve the original as a comment;
3. rewrite relative paths inside the included file to be root-relative.

**Embeds** require the source notebook cell to carry a `fig-`-prefixed label
in Quarto (`#| label: fig-analysis`), whereas MyST uses the `nb:` convention
(`#| label: nb:analysis`). Converting `:::{figure} #nb:analysis` therefore
requires **also rewriting the label inside the referenced notebook** — a
cross-file edit. When the notebook is outside the conversion set, warn and
preserve.

---

## 8. Project configuration: `myst.yml` ↔ `_quarto.yml`

### 8.1 Project type inference

MyST has no `type` field. Quarto's `project.type` selects an entire pipeline.
Infer as follows (first match wins):

| MyST signal | Quarto `project.type` |
|---|---|
| `site.template: book-theme` | `book` |
| `project.exports[].template` present, or `site.template: article-theme` | `manuscript` |
| `project.toc` present with ≥2 entries and no article template | `book` |
| otherwise | `default` |

> The current Python implementation treats **any** `project.toc` as a book. The
> `article-template` fixture — an article with `site.template: article-theme` and
> a `lapreprint-typst` export — is therefore mis-typed as `book`. Correct target
> is `manuscript`.

### 8.2 Field mapping

| `myst.yml` (`project.*`) | `_quarto.yml` | Fidelity | Notes |
|---|---|---|---|
| `title` | `title` / `book.title` | ✅ | Location depends on project type |
| `subtitle` | `subtitle` | ✅ | **Currently dropped** |
| `short_title` | `\| short-title` (metadata) | ⚠️ | **Currently dropped** |
| `description` | `description` | ✅ | **Currently dropped** — and `subject` wrongly overwrites it |
| `subject` | `categories` | ⚠️ | Currently mapped to `description`, which is wrong |
| `keywords` | `keywords` | ✅ | |
| `authors[]` | `author[]` | ✅ | |
| `authors[].name` | `author[].name` | ✅ | |
| `authors[].orcid` | `author[].orcid` | ✅ | |
| `authors[].email` | `author[].email` | ✅ | |
| `authors[].affiliation` | `author[].affiliations[].name` | ⚠️ | MyST allows a bare string; Quarto wants a list of objects |
| `authors[].roles` | `author[].roles` | ✅ | Both use CRediT |
| `authors[].corresponding` | `author[].corresponding` | ✅ | |
| `date` | `date` | ✅ | |
| `license` | `license` | ✅ | |
| `doi` | `doi` | ✅ | |
| `github` | `repo-url` | ✅ | |
| `bibliography` | `bibliography` | ✅ | |
| `toc[].file` | `book.chapters[]` | ⚠️ | Extension rewrite must be **type-aware**: `.md`→`.qmd`, `.ipynb`→`.ipynb` (**unchanged**) |
| `exports[]` | `format` | ⚠️ | See §8.3 |
| `downloads[]` | `downloads` | ⚠️ | Partial analogue |
| `banner` | `image` | ⚠️ | |
| `thumbnail` | `image` | ⚠️ | Collides with `banner` — prefer `banner`, warn on both |
| `abbreviations` | ➖ | ❌ | No Quarto feature. Preserve as comment |
| `open_access` | ➖ | ❌ | Preserve as comment |
| `venue` | ➖ | ❌ | Preserve as comment (journal templates may consume it) |
| `funding` | `funding` | ⚠️ | Shapes differ |
| `id` | ➖ | ❌ | Preserve as comment |
| `math` (macros) | `include-in-header` LaTeX macros | ⚠️ | |
| `numbering` | `number-sections` + `crossref` | ⚠️ | |
| `site.template` | `format.html.theme` | ⚠️ | Theme names do not correspond |
| ➖ | `manuscript.article` | ➖ | Derived from `exports[].article` |

### 8.3 Exports ↔ formats

MyST `exports` is a list of `{format\|template, …}`; Quarto `format` is a map.

| MyST export | Quarto format |
|---|---|
| `- format: pdf` | `format: {pdf: {}}` |
| `- format: docx` | `format: {docx: {}}` |
| `- format: tex` | `format: {latex: {}}` |
| `- format: jats` | `format: {jats: {}}` |
| `- format: meca` | ➖ ❌ |
| `- template: lapreprint-typst` | `format: {typst: {}}` ⚠️ template not portable |

An export entry with only a `template:` and no `format:` **currently produces
`format: {}`**, which is invalid Quarto. The template must be inspected to infer
the format, and the template name preserved as a comment.

### 8.4 Page-level frontmatter

| MyST | Quarto | Fidelity |
|---|---|---|
| `title` | `title` | ✅ |
| `abstract: \|` | `abstract: \|` | ✅ — **block scalar style must be preserved** |
| `acknowledgments` | `\| acknowledgments` | ⚠️ Non-standard in Quarto; keep as metadata |
| `label` | `\|` heading `{#sec-…}` | ⚠️ Currently mapped to `id`, which Quarto ignores |
| `kernelspec: {name: python3}` | `jupyter: python3` | ✅ |
| `kernelspec: {name: ir}` | `engine: knitr` **or** `jupyter: ir` | ⚠️ Engine choice — see §6 |
| `jupytext` | ➖ | ⚠️ Drop |
| `exports` | `format` | ⚠️ As §8.3 |
| `numbering.equation.template` | `crossref.eq-prefix` | ⚠️ |
| `math` | ➖ | ❌ |
| `parts.abstract` | `abstract` | ✅ |

**YAML style preservation is a hard requirement.** Serializing `abstract: |`
through a style-discarding YAML round-trip produces a single-quoted folded
scalar — technically valid, unreadable, and a large spurious diff. The
implementation must use a document-level YAML API that retains block scalars,
key order, and comments.

---

## 9. Comments and structural syntax

| Construct | MyST | Quarto | Fidelity |
|---|---|---|---|
| Line comment | `% comment` at line start | ➖ | ⚠️ → `<!-- comment -->` |
| HTML comment | `<!-- x -->` | `<!-- x -->` | ✅ |
| Block break | `+++` | ➖ | ❌ Preserve as comment |
| Cell delimiter (percent) | `# %%` in `.py` | `# %%` | ✅ |
| Raw block | `` ```{raw} latex `` | ```` ```{=latex} ```` | ⚠️ |

`%` is a comment **only at the start of a line** in MyST — `50% of users` is
literal text mid-line and must not be touched. A `%` comment placed inside a
paragraph splits it into two paragraphs; the HTML-comment equivalent does not,
so the conversion is not perfectly faithful in that position.

---

## 10. Legacy read-only surface

Accepted when reading MyST; never emitted.

| Legacy construct | Modern MyST equivalent | Emitted as (Quarto) |
|---|---|---|
| `` {cite}`key` `` | `[@key]` | `[@key]` |
| `` {cite:t}`key` `` | `@key` | `@key` |
| `` {cite:p}`key` `` | `[@key]` | `[@key]` |
| `` {numref}`fig-x` `` | `@fig-x` | `@fig-x` |
| `` {numref}`Figure %s <fig-x>` `` | `[Figure %s](#fig-x)` | `[Fig @fig-x]` |
| `` {ref}`label` `` | `@label` / `[](#label)` | `@label` |
| `` {eq}`label` `` | `@eq-label` | `@eq-label` |
| `` {doc}`path` `` | `[path](path.md)` | `[path](path.qmd)` |
| `:name:` directive option | `:label:` | `{#id}` |

---

## 11. Unmappable inventory

Constructs with no target equivalent. Policy: **best-effort map, emit a
`file:line` diagnostic, and preserve the original source verbatim in the
`.mystquarto/preserved.json` sidecar**, leaving a single-line marker comment at
the original location.

> Preservation deliberately does **not** inline the original inside an HTML
> comment. Pandoc terminates a raw-HTML block at the first blank line, not at
> `-->`, and unmappable constructs routinely contain blank lines — so inlined
> source escapes the comment and is rendered as live markup. Reproduced with
> `quarto 1.9.36`: a preserved block containing a blank line and a `<script>` tag
> produced an executable script element in the rendered HTML with the render
> exiting 0. Escaping `-->` does not address this; the sidecar does.

`--strict` fails on Warning-class diagnostics; `--strict=all` additionally fails
on the expected-lossy class, which covers most rows in this section.

| Direction | Construct | Handling |
|---|---|---|
| MyST → Quarto | `abbreviations` config | `<abbr>` tags + comment |
| MyST → Quarto | `open_access`, `venue`, `id` | Comment in `_quarto.yml` |
| MyST → Quarto | `{glossary}`, `{epigraph}`, `{pull-quote}` | Comment |
| MyST → Quarto | `{list-table}`, `{csv-table}` | Render to pipe table + warn |
| MyST → Quarto | `+++` block breaks | Comment |
| MyST → Quarto | `{danger}`/`{error}`/`{hint}`/`{seealso}`/`{attention}` | Collapse to nearest callout + warn |
| MyST → Quarto | `:::{figure} #nb:x` with notebook outside conversion set | Comment + warn |
| Quarto → MyST | `` `r expr` `` knitr inline | `` {eval}`expr` `` + IRkernel warning |
| Quarto → MyST | knitr-engine R cells | `{code-cell} r` + IRkernel warning |
| Quarto → MyST | `{{< include >}}` inside a list/quote | Comment + warn |
| Quarto → MyST | `{{< video >}}`, `{{< pagebreak >}}`, `{{< meta >}}`, `{{< var >}}` | Comment + warn |
| Quarto → MyST | `.panel-tabset` whose `##` labels collide with real headings | Warn |
| Quarto → MyST | Bootstrap layout classes (`.column-screen`, `.grid`) | Comment + warn |
| Both | PDF export | Environment concern, not conversion. Both need LaTeX (or Typst for MyST) |

---

## 12. Verified defects in the Python implementation

Reproduced against `article-template/` → `article-template/docs-quarto/`, and
against the direction-aware, renderer-verified fixtures now frozen under
`tests/corpus/defects/d01-*` … `d16-*` (Phase 1 of
`plans/260903-1749-rust-port-dialect-fidelity/`). Each is a failing test case
before the Rust port begins; each fixture's `README.md` documents the exact
capture command and, where applicable, the render-verification command.

**Direction matters.** A fixture built in the wrong direction passes against the
unfixed tool and proves nothing. `M→Q` = MyST→Quarto, `Q→M` = Quarto→MyST.

| # | Dir | Defect | Evidence | Root cause |
|---|---|---|---|---|
| D1 | M→Q | Colon labels emitted unchanged into Quarto — every cross-ref dead | `@fig:samples`, `{#eq:chi-squared}` in `docs-quarto/article.qmd:69,126`; reproduced in `tests/corpus/defects/d01-colon-labels-unnormalized/` | No label normalization layer (§3.4) |
| D2 | M→Q | `:::{figure}` label dropped entirely | `docs-quarto/article.qmd:75` — no `{#fig-samples}`; reproduced in `tests/corpus/defects/d02-figure-label-dropped/` | Reads `:name:`, not `:label:` (§10) |
| D3 | M→Q | `:::{table}` caption and label both lost | `docs-quarto/article.qmd:106-113`; reproduced in `tests/corpus/defects/d03-table-caption-label-lost/` | Caption comes from directive **body** in MyST; code reads `argument` |
| D4 | M→Q | `% comments` emitted as literal visible text | `docs-quarto/article.qmd:40,79,161`; reproduced in `tests/corpus/defects/d04-percent-comments-literal/` | `%` comments unhandled (§9) |
| D5 | M→Q | `(sec:data-analysis)=` emitted as literal visible text | `docs-quarto/article.qmd:81`; reproduced in `tests/corpus/defects/d05-heading-target-literal/` | Target syntax unhandled (§3.1) |
| D6 | M→Q | `format: {}` — invalid Quarto | `docs-quarto/_quarto.yml:15`; reproduced in `tests/corpus/defects/d06-export-template-only/` | Export entry has `template:` but no `format:` (§8.3) |
| D7 | M→Q | `analysis.ipynb.qmd` — nonexistent chapter file | `docs-quarto/_quarto.yml:14`; reproduced in `tests/corpus/defects/d07-notebook-chapter-extension/` | Extension rewrite strips only `.md` (§8.2) |
| D8 | M→Q | Article mis-typed as `book` | `docs-quarto/_quarto.yml:2`; cause at `config.py:10-22`; reproduced in `tests/corpus/defects/d08-article-mistyped-book/` | `toc` presence treated as book signal, ignoring `site.template: article-theme` (§8.1) |
| D9 | M→Q | Block scalars mangled into folded/quoted strings | `docs-quarto/article.qmd:3-19`; reproduced in `tests/corpus/defects/d09-block-scalar-mangled/` (page frontmatter path, `frontmatter.py`'s `replace_frontmatter`) | Style-discarding YAML round-trip — stock `yaml.dump` (§8.4) |
| D10 | M→Q | 8 config fields dropped, 1 overwritten, no warning for any | Dropped: `id`, `subtitle`, `short_title`, `open_access`, `venue`, `banner`, `abbreviations`, `site.template`. **Overwritten:** `description` holds `subject`'s value (`_quarto.yml:21`). `downloads` was `[]`, so its loss is vacuous. Reproduced in `tests/corpus/defects/d10-config-fields-dropped/` | Unmapped keys ignored without warning (§8.2) |
| D11 | M→Q | `:::{figure} #nb:analysis` → broken image link | `docs-quarto/article.qmd:139`; reproduced in `tests/corpus/defects/d11-notebook-embed-broken-link/` using the real syntax from `article-template/article.md:127` | Notebook embed unhandled (§7) |
| D12 | both | **Zero warnings emitted for any of the above** | `warnings.py:31` is the only `.append`; no transform calls it; `cli.py:63` prints success unconditionally, even under `--strict`. Reproduced in `tests/corpus/defects/d12-silent-warnings/` (CLI run against D10's fixture, `--strict`, exit 0, zero warning text) | `WarningCollector` exists but is never populated |
| D13 | both | `{{< include >}}` / `{include}` unhandled | `grep "{{<" src/mystquarto/transforms/` → no matches. Split by direction: `tests/corpus/defects/d13a-include-myst-to-quarto/` (`` ```{include} `` falls to the generic unknown-directive passthrough) and `tests/corpus/defects/d13b-include-quarto-to-myst/` (`{{< include >}}` copied through verbatim, invalid MyST) | Shortcodes not parsed (§7) |
| D14 | Q→M | knitr `` `r expr` `` unhandled | `quarto_to_myst.py:12` matches only `` `{python} ``; reproduced in `tests/corpus/defects/d14-knitr-inline-unhandled/` | Inline knitr form unrecognized (§5) |
| D15 | **Q→M** | DOI citation keys corrupted | `quarto_to_myst.py:18,35`. Verified: `convert_quarto_to_myst("[@10.1038/nmeth.1974]")` → `` [{cite:t}`10`.1038/nmeth.1974] ``, while the M→Q direction leaves it **intact**. Reproduced in `tests/corpus/defects/d15-doi-citation-keys/` — expected output is the **unchanged** input, since modern MyST and Quarto citation syntax are identical (§4) and the fix is a wider regex, not a legacy-form conversion | `_SINGLE_CITE_RE` `[\w-]+` fails to match `.`/`/`, then `_BARE_CITE_RE` mangles the key mid-string (§4) |
| D16 | both | Output directory nested inside input is re-converted and re-copied — **worse than originally measured** | `article-template/docs-quarto/docs-quarto/` — 8 duplicated files incl. a 895 KB `banner.png`. Reproduced in `tests/corpus/defects/d16-output-recursion/`: nesting (`docs-quarto/docs-quarto/`) appears after a **single run**, not after a second run as originally assumed — `os.makedirs(effective_output_dir)` creates the output dir inside the input tree *before* `_copy_assets` walks it, and `os.walk`'s non-alphabetical directory order can revisit the just-created output subtree within the same pass. A second run adds a further nesting level | `discover_files` (`convert.py:69-82`) and `_copy_assets` (`convert.py:337-350`) keep separate skip-sets, neither excluding the effective output dir |

Two further hazards found during red-team review, tracked as requirements rather
than conversion defects because they are orchestration-layer, not dialect-layer:
`_copy_assets` dereferences symlinks (`convert.py:368` `shutil.copy2`), and
`--in-place` deletes sources (`convert.py:291-293`) with no atomicity.

**A note on D15's original entry.** It was first catalogued as an M→Q defect
citing the `docs-quarto/` artifact, where the DOI key is in fact untouched. A
fixture built from that reading would have matched its own baseline and passed
before any fix existed. Direction is now recorded per row for this reason.

**A note on D16's severity.** The original catalogue entry (and Phase 1's own
plan text) assumed the recursion required two runs to manifest. Phase 1's
renderer-backed reproduction found it manifests on the **first** run whenever
the output directory is placed inside the input tree, which is precisely how
`article-template/docs-quarto/` was produced. This raises D16 from a
repeated-run hazard to a single-run one.

---

## Sources

- [MyST cross-references](https://mystmd.org/guide/cross-references)
- [MyST directives](https://mystmd.org/guide/directives)
- [MyST admonitions](https://mystmd.org/guide/admonitions)
- [MyST frontmatter](https://mystmd.org/guide/frontmatter)
- [MyST blocks and comments](https://mystmd.org/guide/blocks)
- [MyST typography](https://mystmd.org/guide/typography)
- [MyST reuse Jupyter outputs](https://mystmd.org/guide/reuse-jupyter-outputs)
- [Quarto cross-references](https://quarto.org/docs/authoring/cross-references.html)
- [Quarto callouts](https://quarto.org/docs/authoring/callouts.html)
- [Quarto includes](https://quarto.org/docs/authoring/includes.html)
- [Quarto inline code](https://quarto.org/docs/computations/inline-code.html)
- [Quarto manuscripts](https://quarto.org/docs/manuscripts/)
