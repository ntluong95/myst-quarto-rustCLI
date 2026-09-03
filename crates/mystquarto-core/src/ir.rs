//! The document intermediate representation (IR) shared by every reader and
//! writer in this crate.
//!
//! `Document` / `Block` / `BlockKind` / `FigureSource` are transcribed
//! verbatim (field names, variant names) from the phase spec's Architecture
//! section — that sketch is a considered design, not a suggestion. Every
//! `BlockKind` variant below carries a doc comment citing the
//! `docs/dialect-comparison.md` §2/§2.1 (or, for the base constructs that
//! predate any directive vocabulary, §3.1) row it represents; see this
//! module's doc comment on [`BlockKind`] for the three variants added
//! beyond the sketch and why.
//!
//! Four properties this IR has that the Python implementation's ad hoc
//! dict/string manipulation lacked (phase spec, Architecture section):
//!
//! 1. `span` on every [`Block`] — diagnostics can say `article.md:55` (fixes D12).
//! 2. `label` on labelable constructs — the writer knows a label belongs to
//!    a *figure* and picks `fig-` (fixes D1, D2, D3).
//! 3. [`BlockKind::Preserved`] — a first-class, readable variant (RT-11).
//! 4. [`FigureSource::CellRef`]'s `notebook` field — `{{< embed >}}` needs a
//!    file path the MyST source alone does not carry (RT-03).

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::label::Label;
use crate::span::Span;
use crate::yaml::YamlValue;

/// A single source file being converted, and everything read out of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// The page's YAML frontmatter, if any — original text plus a parsed
    /// view (reference §8.4). `None` for files with no `---` block.
    pub frontmatter: Option<Frontmatter>,
    /// The file's content blocks, in source order.
    pub blocks: Vec<Block>,
    /// The path this document was read from.
    pub source: PathBuf,
    /// The execution engine in effect for this document's code cells, if
    /// any — recorded by Phase 4's reader from `kernelspec`/`engine:`
    /// (reference §6, §8.4).
    pub engine: Option<Engine>,
}

/// One content block: its parsed shape, where it came from, and how much
/// blank space preceded it in the source.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub kind: BlockKind,
    /// Line range in the source, 1-indexed (fixes D12).
    pub span: Span,
    /// Number of blank lines immediately before this block, capped at
    /// `u8::MAX`. Exists so same-dialect round-trip can be byte-exact —
    /// without it, separator conventions are unrepresentable (RT-13).
    pub blank_lines_before: u8,
}

/// The execution engine a document's code cells run under. MyST has only
/// Jupyter; Quarto has two (reference §6 "Execution model").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// R-native engine, no Jupyter kernel required for R.
    Knitr,
    /// The only engine MyST supports; also one of Quarto's two.
    Jupyter,
}

/// A page's YAML frontmatter: the original text (for [`crate::yaml::surgery`]
/// to edit byte-precisely) plus a parsed, order-preserving view (for
/// programmatic field access — e.g. Phase 6 reading `open_access`).
#[derive(Debug, Clone, PartialEq)]
pub struct Frontmatter {
    /// The exact original text between the `---` delimiters, byte for
    /// byte, including block scalars, comments, and key order.
    pub raw: String,
    /// The same content parsed into an order-preserving mapping. Has no
    /// memory of block-scalar style — that's exactly why edits go through
    /// `raw` via [`crate::yaml::surgery`], never through re-emitting this
    /// field (see the `yaml` module docs).
    pub parsed: YamlValue,
}

/// Arbitrary `{key="value"}`-style attributes, e.g. a figure's `width=`/
/// `align=`, or a static code block's `filename=`/`linenos=` (reference
/// §2.1 "Static code", "Figure"). A `BTreeMap` rather than a `Vec` keeps
/// lookups simple and output deterministic; these are small, unordered-by-
/// convention attribute bags, not something that needs to preserve
/// original key order the way frontmatter does.
pub type Attrs = BTreeMap<String, String>;

/// Per-cell rendering options for a [`BlockKind::CodeCell`]: MyST's
/// `:tags:` list (reference §2 rows `:tags: [remove-input]` /
/// `[remove-output]` / `[remove-cell]` / `[hide-input]`) and Quarto's
/// `#| key: value` cell comments it corresponds to, plus the caption
/// option (`:caption:` / `#| fig-cap:`, reference §2). Anything not
/// otherwise modeled lands in `extra` rather than being dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CellOptions {
    pub tags: Vec<String>,
    pub caption: Option<String>,
    pub extra: BTreeMap<String, String>,
}

/// The ten admonition kinds a reader may see. The five overlapping ones
/// (`Note`..`Caution`) have a native Quarto `callout-*` target; the five
/// MyST-only ones (`Danger`..`Attention`) collapse onto the nearest of
/// those five on write (reference §2 rows `danger`..`attention`) but are
/// kept distinct here so the reader can round-trip what it actually saw —
/// same-dialect (MyST→MyST) round-trip needs to know `hint` was `hint`,
/// not `note`, even though Quarto can't tell the difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmonitionKind {
    /// Reference §2: `note` → `callout-note`.
    Note,
    /// Reference §2: `warning` → `callout-warning`.
    Warning,
    /// Reference §2: `tip` → `callout-tip`.
    Tip,
    /// Reference §2: `important` → `callout-important`.
    Important,
    /// Reference §2: `caution` → `callout-caution`.
    Caution,
    /// Reference §2: `danger` → `callout-important` (lossy, collapses).
    Danger,
    /// Reference §2: `error` → `callout-important` (lossy, collapses).
    Error,
    /// Reference §2: `hint` → `callout-note` (lossy, collapses).
    Hint,
    /// Reference §2: `seealso` → `callout-note` (lossy, collapses).
    SeeAlso,
    /// Reference §2: `attention` → `callout-note` (lossy, collapses).
    Attention,
}

/// One tab of a [`BlockKind::TabSet`]. Reference §2.1 "Tabs":
/// `:::{tab-item} Label` ↔ a Quarto `## Label` heading inside
/// `::: {.panel-tabset}`.
#[derive(Debug, Clone, PartialEq)]
pub struct TabItem {
    pub label: String,
    pub body: Vec<Block>,
}

/// Options for a [`BlockKind::Include`]. Reference §7 "Composition:
/// includes and embeds": `:literal:`/`:lang:` (render the target as a code
/// block rather than inlining it), and `:start-line:`/`:end-line:`/
/// `:lines:` (partial-file includes) map to Quarto's `start-line=`/
/// `end-line=` shortcode parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IncludeOpts {
    /// `:literal:` — include as a fenced code block rather than parsed content.
    pub literal: bool,
    /// `:lang:` — language for a literal include.
    pub lang: Option<String>,
    /// `:start-line:` / Quarto `start-line=`.
    pub start_line: Option<u32>,
    /// `:end-line:` / Quarto `end-line=`.
    pub end_line: Option<u32>,
    /// `:lines:` — a range spec (e.g. `"1-10,15"`) with no single-field
    /// Quarto equivalent; kept as the original spec string.
    pub lines: Option<String>,
}

/// What a [`BlockKind::Embed`] points at. Reference §7 "Embed notebook
/// output": MyST's `nb:` convention needs the source notebook's path
/// because the MyST text alone (`#nb:cell-label`) does not carry it
/// (RT-03) — the same gap [`FigureSource::CellRef`] fixes for figures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedTarget {
    /// A plain label reference within the same document.
    Label(Label),
    /// A cross-file reference to a labeled cell in another notebook.
    NotebookCell {
        notebook: PathBuf,
        cell_label: Label,
    },
}

/// How a [`BlockKind::Comment`] was written. Reference §9 "Comments and
/// structural syntax": MyST's `%`-prefixed line comment has no Quarto
/// equivalent and becomes `<!-- comment -->`; an HTML comment is already
/// identical in both dialects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentStyle {
    /// `% comment` — MyST-only, line-start only (reference §9).
    Percent,
    /// `<!-- comment -->` — identical in both dialects.
    Html,
}

/// Where a figure's image comes from. Reference §2.1 "Figure" (a plain
/// path) vs. §7 "Embed notebook output" / RT-03 (a reference to a notebook
/// cell's output, which needs the notebook's path to become
/// `{{< embed nb.ipynb#fig-cell >}}` — the MyST source alone,
/// `#nb:cell-label`, doesn't carry it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FigureSource {
    /// A plain image path, e.g. `:::{figure} path/to/img.png`.
    Path(PathBuf),
    /// A reference to a labeled notebook cell's output.
    CellRef {
        label: Label,
        notebook: Option<PathBuf>,
    },
}

/// The shape of one [`Block`].
///
/// Every variant traces to a reference §2/§2.1 row (cited on the variant
/// itself), to §3.1 for the two base constructs that predate any directive
/// vocabulary (`Heading`, `Paragraph` — see their doc comments), or is one
/// of the explicitly-allowed non-content variants (`BlockBreak`, `Target`,
/// `Comment`, `Raw`, `Preserved`, `Unmappable`).
///
/// **Three variants beyond the phase spec's sketch**, added because §2.1's
/// own framing paragraph lists eleven structural constructs needing
/// "type-aware, hand-written transform code," and the sketch gave eight of
/// them a dedicated variant (`StaticCode`, `Figure` for two rows, `Table`,
/// `Tabs`→`TabSet`, `Include`, `Notebook output embed`→`Embed`) but not
/// the remaining three:
///
/// - [`BlockKind::Blockquote`] — §2.1 "Blockquote + attribution": no
///   existing variant models a quote body plus an optional `-- Author`
///   attribution line.
/// - [`BlockKind::Theorem`] — §2.1 "Proof / theorem": not one of
///   [`AdmonitionKind`]'s ten kinds (the phase spec scopes that enum to
///   exactly the five overlapping + five MyST-only admonition kinds), and
///   it carries a label for cross-referencing that a generic `Admonition`
///   does not want on every callout.
/// - [`BlockKind::Directive`] — a generic catch-all for §2 rows that are
///   "pure directive/attribute name swaps" (the doc's own §2 framing) with
///   nested block content but no dedicated fields, concretely `grid`/`card`
///   (Bootstrap-class containers) and the argument-only `bibliography`/
///   `tableofcontents` directives (§2: "Drop the directive" — dropping is a
///   writer decision, so the reader still needs somewhere to park the
///   parsed directive rather than special-casing "some directives don't
///   parse to a block at all"). Using [`BlockKind::Unmappable`] for these
///   was considered and rejected: `Unmappable`'s own policy (reference
///   §11) is "warn + preserve original as comment," which is the ❌-fidelity
///   disposition, not the ⚠️-fidelity "lossy but mapped" disposition these
///   rows actually have — using it here would have been a Phase-4-facing
///   modeling error (the phase spec's Risk Assessment calls out exactly
///   this: "the IR is wrong and Phase 4 needs an IR change more than
///   twice").
#[derive(Debug, Clone, PartialEq)]
pub enum BlockKind {
    /// A base CommonMark construct that predates any directive vocabulary
    /// — not itself a §2/§2.1 row, but its *label* association is: §3.1's
    /// "Heading" row, `(sec:data-analysis)=` on the line before the
    /// heading ↔ `## Heading {#sec-data-analysis}`. `text` is the raw
    /// heading text (inline markup untouched; Phase 5's job).
    Heading {
        level: u8,
        text: String,
        label: Option<Label>,
    },

    /// The default prose container — not itself a §2/§2.1 row (§2/§2.1
    /// catalog directives and structural constructs, not plain paragraphs)
    /// but required as the fallback holder for ordinary text, since almost
    /// every other variant's `Vec<String>` fields (captions, admonition
    /// bodies via nested `Block`s, …) ultimately bottom out in
    /// paragraph-shaped text. `lines` holds raw source lines; inline
    /// syntax (citations, cross-refs, emphasis) is untouched here —
    /// Phase 5's job.
    Paragraph { lines: Vec<String> },

    /// Reference §2 "code-cell | {python} | ✅" plus its option rows
    /// (`:tags:`, `:caption:`, `:label:`): `` ```{code-cell} lang `` ↔
    /// `` ```{python} ``/`` ```{r} ``.
    CodeCell {
        lang: String,
        options: CellOptions,
        body: Vec<String>,
        label: Option<Label>,
    },

    /// Reference §2.1 "Static code": `` ```{code} python `` + `:filename:`
    /// `:linenos:` ↔ `` ```python `` + `{filename="…"}`. Also covers
    /// reference §2's `mermaid` row (an identical fenced block in both
    /// dialects — `lang: Some("mermaid")`, no directive-specific fields
    /// needed).
    StaticCode {
        lang: Option<String>,
        body: Vec<String>,
        attrs: Attrs,
    },

    /// Reference §2.1 "Figure" and "Figure (div form)": `:::{figure} path`
    /// plus `:label:` `:width:` `:alt:` `:align:`, mapping to
    /// `![caption](path){#fig-id width="X"}`, or the div form for
    /// multi-paragraph captions.
    Figure {
        src: FigureSource,
        caption: Vec<String>,
        label: Option<Label>,
        attrs: Attrs,
    },

    /// Reference §2.1 "Table + caption": `:::{table}` + `:label:` +
    /// caption paragraph + pipe table ↔ pipe table + `: Caption {#tbl-id}`.
    /// Also the target of §2.1's `list-table`/`csv-table` rendering ("no
    /// Quarto equivalent... render to a pipe table + warn") once a reader
    /// performs that rendering.
    Table {
        caption: Vec<String>,
        rows: Vec<String>,
        label: Option<Label>,
    },

    /// Reference §2 rows `{math} + :label: eq:x` (↔ `$$ … $$ {#eq-x}`) and
    /// block-level `$$ … $$` (↔ `$$ … $$`, identical).
    Math {
        body: Vec<String>,
        label: Option<Label>,
    },

    /// Reference §2's ten callout/admonition rows (`note`..`attention`),
    /// "admonition (custom title)" (`title`), and `:class: dropdown` +
    /// `:open:` (`collapse` — inverted polarity: MyST `:open: true` ≙
    /// Quarto `collapse="false"`, a Phase 4/5 concern, not this field's).
    Admonition {
        kind: AdmonitionKind,
        title: Option<String>,
        body: Vec<Block>,
        collapse: Option<bool>,
    },

    /// Reference §2.1 "Tabs": `::::{tab-set}` / `:::{tab-item} Label` ↔
    /// `::: {.panel-tabset}` / `## Label`.
    TabSet { items: Vec<TabItem> },

    /// Reference §2 rows `margin` and `aside` (both ↔ `column-margin`;
    /// the reverse direction picks `{aside}` per the note on those rows).
    Margin { body: Vec<Block> },

    /// Reference §2.1 "Include": `` ```{include} file.md `` ↔
    /// `{{< include _file.qmd >}}`. Placement/path rules are in §7, a
    /// reader/writer concern, not this variant's.
    Include { target: PathBuf, opts: IncludeOpts },

    /// Reference §2.1 "Notebook output embed" / §7 "Embed notebook
    /// output" and "Embed shorthand": `:::{figure} #nb:cell-label` /
    /// `` ```{embed} #label `` / `![](#label)` ↔
    /// `{{< embed nb.ipynb#fig-cell >}}`.
    Embed {
        target: EmbedTarget,
        label: Option<Label>,
    },

    /// Reference §2.1 "Blockquote + attribution": `` ```{blockquote} `` +
    /// `-- Author` ↔ `> quote` + `> — Author`. `attribution`, when
    /// present, holds the author line(s) separately from `body` since
    /// Quarto has no first-class attribution node to round-trip through —
    /// the writer decides how to render it. Added beyond the phase spec's
    /// sketch; see this enum's doc comment.
    Blockquote {
        body: Vec<Block>,
        attribution: Option<Vec<String>>,
    },

    /// Reference §2.1 "Proof / theorem": `` ```{prf:theorem} `` ↔
    /// `::: {#thm-x .theorem}` (requires Quarto `crossref` theorem
    /// config, a writer/config concern). `thm_type` holds the MyST
    /// `prf:` subtype (`theorem`, `lemma`, `proof`, …) verbatim; Phase 5/6
    /// map it to a crossref class. Added beyond the phase spec's sketch;
    /// see this enum's doc comment.
    Theorem {
        thm_type: String,
        label: Option<Label>,
        body: Vec<Block>,
    },

    /// A generic directive with nested block content, for reference §2
    /// rows that are pure name/attribute swaps with no dedicated fields:
    /// concretely `grid`, `card` (Bootstrap-class containers — "Quarto
    /// classes are Bootstrap, not semantic. Approximate"), and the
    /// argument-only `bibliography`/`tableofcontents` directives ("Drop
    /// the directive"). `name` is the MyST directive name (or Quarto div
    /// class) as read, unchanged. Added beyond the phase spec's sketch;
    /// see this enum's doc comment.
    Directive {
        name: String,
        attrs: Attrs,
        body: Vec<Block>,
        label: Option<Label>,
    },

    /// Reference §9 "Line comment" (`%`, MyST-only, line-start only) and
    /// "HTML comment" (`<!-- x -->`, identical in both dialects).
    Comment { text: String, style: CommentStyle },

    /// Reference §3.1 "Arbitrary block": `(my-para)=` before any block —
    /// a standalone label target with no attached content of its own.
    Target { label: Label },

    /// Reference §9 "Raw block": `` ```{raw} latex `` ↔
    /// `` ```{=latex} ``. Also where §2.1 "iframe" lands after
    /// conversion: "Emit raw `<iframe>` + warn" — `format: "html"`,
    /// `body` holding the `<iframe>` tag.
    Raw { format: String, body: Vec<String> },

    /// Reference §9 "Block break": MyST `+++` ↔ ➖, "Preserve as comment".
    BlockBreak,

    /// A construct the reader recognized but chose not to transform,
    /// preserved verbatim so a later pass (or the same-dialect round-trip
    /// path) can read it back — RT-11: the original plan emitted
    /// preservation sidecars but never parsed them back, making its own
    /// round-trip success criterion unreachable. `code` is a stable
    /// preservation-reason code, not a plan/phase label (kept short and
    /// diagnosable, e.g. `"D13-nested-include"`).
    Preserved {
        original: Vec<String>,
        code: &'static str,
    },

    /// Reference §11 "Unmappable inventory": constructs with no target
    /// equivalent at all (❌ fidelity) — `abbreviations`, `{glossary}`,
    /// `{epigraph}`, `{pull-quote}`, and the Quarto→MyST-only rows in that
    /// section. Policy: "best-effort map, emit a `file:line` diagnostic,
    /// and preserve the original source verbatim" — `reason` is a
    /// human-readable diagnostic message, not a code (contrast
    /// `Preserved::code`, which is for the ⚠️-but-declined-to-transform
    /// case, a different disposition than ❌-no-target-exists).
    Unmappable {
        original: Vec<String>,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span {
        Span::single(1)
    }

    fn block(kind: BlockKind) -> Block {
        Block {
            kind,
            span: span(),
            blank_lines_before: 0,
        }
    }

    /// One instance of every `BlockKind` variant, each with a basic
    /// field-access assertion. This is a compile-time-shape proof more
    /// than a behavior test — it's what would have caught the phase
    /// spec's sketch missing a construct early (see this module's docs).
    #[test]
    fn every_block_kind_variant_constructs_and_is_accessible() {
        let heading = block(BlockKind::Heading {
            level: 2,
            text: "Data analysis".to_string(),
            label: Some(Label::new("sec:data-analysis")),
        });
        let BlockKind::Heading { level, text, label } = &heading.kind else {
            panic!()
        };
        assert_eq!(*level, 2);
        assert_eq!(text, "Data analysis");
        assert_eq!(label.as_ref().unwrap().raw, "sec:data-analysis");

        let paragraph = block(BlockKind::Paragraph {
            lines: vec!["Some prose.".to_string()],
        });
        let BlockKind::Paragraph { lines } = &paragraph.kind else {
            panic!()
        };
        assert_eq!(lines.len(), 1);

        let code_cell = block(BlockKind::CodeCell {
            lang: "python".to_string(),
            options: CellOptions {
                tags: vec!["remove-input".to_string()],
                caption: Some("A cell".to_string()),
                extra: BTreeMap::new(),
            },
            body: vec!["print('hi')".to_string()],
            label: Some(Label::new("nb:analysis")),
        });
        let BlockKind::CodeCell {
            lang,
            options,
            body,
            label,
        } = &code_cell.kind
        else {
            panic!()
        };
        assert_eq!(lang, "python");
        assert_eq!(options.tags, vec!["remove-input".to_string()]);
        assert_eq!(body.len(), 1);
        assert!(label.is_some());

        let static_code = block(BlockKind::StaticCode {
            lang: Some("mermaid".to_string()),
            body: vec!["graph TD; A-->B;".to_string()],
            attrs: Attrs::new(),
        });
        let BlockKind::StaticCode { lang, body, .. } = &static_code.kind else {
            panic!()
        };
        assert_eq!(lang.as_deref(), Some("mermaid"));
        assert_eq!(body.len(), 1);

        let figure = block(BlockKind::Figure {
            src: FigureSource::Path(PathBuf::from("img/samples.png")),
            caption: vec!["Sample distribution.".to_string()],
            label: Some(Label::new("fig:samples")),
            attrs: Attrs::new(),
        });
        let BlockKind::Figure { src, caption, .. } = &figure.kind else {
            panic!()
        };
        assert!(matches!(src, FigureSource::Path(_)));
        assert_eq!(caption.len(), 1);

        let figure_cellref = block(BlockKind::Figure {
            src: FigureSource::CellRef {
                label: Label::new("nb:analysis-cell"),
                notebook: Some(PathBuf::from("analysis.ipynb")),
            },
            caption: vec![],
            label: None,
            attrs: Attrs::new(),
        });
        let BlockKind::Figure {
            src: FigureSource::CellRef { notebook, .. },
            ..
        } = &figure_cellref.kind
        else {
            panic!()
        };
        assert!(notebook.is_some());

        let table = block(BlockKind::Table {
            caption: vec!["Results.".to_string()],
            rows: vec!["| a | b |".to_string(), "|---|---|".to_string()],
            label: Some(Label::new("tab:results")),
        });
        let BlockKind::Table { rows, .. } = &table.kind else {
            panic!()
        };
        assert_eq!(rows.len(), 2);

        let math = block(BlockKind::Math {
            body: vec!["x^2 + y^2 = z^2".to_string()],
            label: Some(Label::new("eq:chi-squared")),
        });
        let BlockKind::Math { body, .. } = &math.kind else {
            panic!()
        };
        assert_eq!(body.len(), 1);

        let admonition = block(BlockKind::Admonition {
            kind: AdmonitionKind::Hint,
            title: Some("Custom Title".to_string()),
            body: vec![block(BlockKind::Paragraph {
                lines: vec!["Nested.".to_string()],
            })],
            collapse: Some(false),
        });
        let BlockKind::Admonition {
            kind,
            body,
            collapse,
            ..
        } = &admonition.kind
        else {
            panic!()
        };
        assert_eq!(*kind, AdmonitionKind::Hint);
        assert_eq!(body.len(), 1);
        assert_eq!(*collapse, Some(false));

        let tabset = block(BlockKind::TabSet {
            items: vec![TabItem {
                label: "Tab A".to_string(),
                body: vec![],
            }],
        });
        let BlockKind::TabSet { items } = &tabset.kind else {
            panic!()
        };
        assert_eq!(items.len(), 1);

        let margin = block(BlockKind::Margin {
            body: vec![block(BlockKind::Paragraph {
                lines: vec!["Aside.".to_string()],
            })],
        });
        let BlockKind::Margin { body } = &margin.kind else {
            panic!()
        };
        assert_eq!(body.len(), 1);

        let include = block(BlockKind::Include {
            target: PathBuf::from("_shared.qmd"),
            opts: IncludeOpts {
                start_line: Some(1),
                end_line: Some(10),
                ..Default::default()
            },
        });
        let BlockKind::Include { opts, .. } = &include.kind else {
            panic!()
        };
        assert_eq!(opts.start_line, Some(1));

        let embed = block(BlockKind::Embed {
            target: EmbedTarget::NotebookCell {
                notebook: PathBuf::from("analysis.ipynb"),
                cell_label: Label::new("nb:analysis"),
            },
            label: Some(Label::new("fig-analysis")),
        });
        let BlockKind::Embed { target, .. } = &embed.kind else {
            panic!()
        };
        assert!(matches!(target, EmbedTarget::NotebookCell { .. }));

        let blockquote = block(BlockKind::Blockquote {
            body: vec![block(BlockKind::Paragraph {
                lines: vec!["A quote.".to_string()],
            })],
            attribution: Some(vec!["Jane Doe".to_string()]),
        });
        let BlockKind::Blockquote { body, attribution } = &blockquote.kind else {
            panic!()
        };
        assert_eq!(body.len(), 1);
        assert_eq!(attribution.as_ref().unwrap(), &vec!["Jane Doe".to_string()]);

        let theorem = block(BlockKind::Theorem {
            thm_type: "theorem".to_string(),
            label: Some(Label::new("thm:main")),
            body: vec![block(BlockKind::Paragraph {
                lines: vec!["Statement.".to_string()],
            })],
        });
        let BlockKind::Theorem { thm_type, body, .. } = &theorem.kind else {
            panic!()
        };
        assert_eq!(thm_type, "theorem");
        assert_eq!(body.len(), 1);

        let directive = block(BlockKind::Directive {
            name: "grid".to_string(),
            attrs: Attrs::new(),
            body: vec![block(BlockKind::Directive {
                name: "card".to_string(),
                attrs: Attrs::new(),
                body: vec![],
                label: None,
            })],
            label: None,
        });
        let BlockKind::Directive { name, body, .. } = &directive.kind else {
            panic!()
        };
        assert_eq!(name, "grid");
        assert_eq!(body.len(), 1);

        let comment = block(BlockKind::Comment {
            text: "a note".to_string(),
            style: CommentStyle::Percent,
        });
        let BlockKind::Comment { style, .. } = &comment.kind else {
            panic!()
        };
        assert_eq!(*style, CommentStyle::Percent);

        let target = block(BlockKind::Target {
            label: Label::new("my-para"),
        });
        let BlockKind::Target { label } = &target.kind else {
            panic!()
        };
        assert_eq!(label.raw, "my-para");

        let raw = block(BlockKind::Raw {
            format: "html".to_string(),
            body: vec!["<iframe src=\"x\"></iframe>".to_string()],
        });
        let BlockKind::Raw { format, .. } = &raw.kind else {
            panic!()
        };
        assert_eq!(format, "html");

        let block_break = block(BlockKind::BlockBreak);
        assert!(matches!(block_break.kind, BlockKind::BlockBreak));

        let preserved = block(BlockKind::Preserved {
            original: vec![
                "```{glossary}".to_string(),
                "term: def".to_string(),
                "```".to_string(),
            ],
            code: "D13-nested-include",
        });
        let BlockKind::Preserved { code, original } = &preserved.kind else {
            panic!()
        };
        assert_eq!(*code, "D13-nested-include");
        assert_eq!(original.len(), 3);

        let unmappable = block(BlockKind::Unmappable {
            original: vec!["```{epigraph}".to_string(), "```".to_string()],
            reason: "no Quarto target; warn + preserve (reference §11)".to_string(),
        });
        let BlockKind::Unmappable { reason, .. } = &unmappable.kind else {
            panic!()
        };
        assert!(reason.contains("no Quarto target"));
    }

    #[test]
    fn document_holds_frontmatter_blocks_source_and_engine() {
        let doc = Document {
            frontmatter: Some(Frontmatter {
                raw: "title: Sample\n".to_string(),
                parsed: YamlValue::Mapping(vec![(
                    "title".to_string(),
                    YamlValue::String("Sample".to_string()),
                )]),
            }),
            blocks: vec![block(BlockKind::Paragraph {
                lines: vec!["Body.".to_string()],
            })],
            source: PathBuf::from("article.md"),
            engine: Some(Engine::Jupyter),
        };
        assert!(doc.frontmatter.is_some());
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.source, PathBuf::from("article.md"));
        assert_eq!(doc.engine, Some(Engine::Jupyter));
    }

    #[test]
    fn block_span_and_blank_lines_before_are_accessible() {
        let b = Block {
            kind: BlockKind::BlockBreak,
            span: Span::new(10, 12),
            blank_lines_before: 2,
        };
        assert_eq!(b.span.start_line, 10);
        assert_eq!(b.span.end_line, 12);
        assert_eq!(b.blank_lines_before, 2);
    }
}
