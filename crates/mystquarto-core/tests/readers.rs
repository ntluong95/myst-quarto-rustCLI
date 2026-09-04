use std::fs;
use std::path::{Path, PathBuf};

use mystquarto_core::fs::path_guard::{canonicalize_root, IncludeChain, MAX_INCLUDE_DEPTH};
use mystquarto_core::{
    BlockKind, FigureSource, MystReader, NotebookCellIndex, QuartoReader, ReaderContext,
};

#[test]
fn article_template_parses_without_unmappable_blocks_and_resolves_notebook_cell() {
    let mut index = NotebookCellIndex::default();
    index
        .add_notebook_json(
            "analysis.ipynb",
            include_str!("../../../article-template/analysis.ipynb"),
        )
        .unwrap();
    let reader = MystReader::new(ReaderContext {
        notebook_index: index,
        ..ReaderContext::new("article-template/article.md")
    });
    let doc = reader
        .read_str(include_str!("../../../article-template/article.md"))
        .unwrap();

    assert!(
        doc.blocks
            .iter()
            .all(|b| !matches!(b.kind, BlockKind::Unmappable { .. })),
        "article-template/article.md should parse without unmappable blocks"
    );
    assert!(doc.blocks.iter().any(|block| {
        matches!(
            &block.kind,
            BlockKind::Figure {
                src: FigureSource::CellRef {
                    notebook: Some(path),
                    ..
                },
                ..
            } if path == &PathBuf::from("analysis.ipynb")
        )
    }));
}

#[test]
fn myst_include_targets_are_checked_by_path_guard() {
    let tmp = tempdir("include-escape");
    let input = tmp.join("input");
    let sub = input.join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(tmp.join("secret.md"), "outside").unwrap();
    let root = canonicalize_root(&input).unwrap();
    let source = sub.join("article.md");
    let reader = MystReader::new(ReaderContext::new(source).with_input_root(root));
    let doc = reader
        .read_str("```{include} ../../secret.md\n```\n")
        .unwrap();

    assert!(
        matches!(&doc.blocks[0].kind, BlockKind::Unmappable { reason, .. } if reason.contains("escapes")),
        "include traversal should become an unmappable guarded block"
    );
    cleanup(&tmp);
}

#[test]
fn myst_include_in_subdirectory_retains_relative_target() {
    let tmp = tempdir("include-subdir");
    let input = tmp.join("input");
    let sub = input.join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("snippet.md"), "content").unwrap();
    let root = canonicalize_root(&input).unwrap();
    let source = sub.join("article.md");
    let reader = MystReader::new(ReaderContext::new(source).with_input_root(root));
    let doc = reader.read_str("```{include} snippet.md\n```\n").unwrap();

    assert!(
        matches!(&doc.blocks[0].kind, BlockKind::Include { target, .. } if target == Path::new("snippet.md")),
        "include in subdirectory should retain target relative to document, not relative to project root"
    );
    cleanup(&tmp);
}

#[test]
fn unresolved_notebook_cell_ref_becomes_unmappable() {
    let reader = MystReader::new(ReaderContext::new("article.md"));
    let doc = reader.read_str(":::{figure} #nb:missing\n:::\n").unwrap();

    assert!(
        matches!(&doc.blocks[0].kind, BlockKind::Unmappable { reason, .. } if reason.contains("does not resolve")),
        "unresolved notebook cell refs must not become partial figures"
    );
}

#[test]
fn include_cycle_and_depth_are_checked_by_reader_context() {
    let mut chain = IncludeChain::new();
    chain.push(PathBuf::from("shared.md")).unwrap();
    let reader = MystReader::new(ReaderContext {
        include_chain: chain,
        ..ReaderContext::new("article.md")
    });
    let doc = reader.read_str("```{include} shared.md\n```\n").unwrap();
    assert!(
        matches!(&doc.blocks[0].kind, BlockKind::Unmappable { reason, .. } if reason.contains("cycle"))
    );

    let mut deep = IncludeChain::new();
    for i in 0..MAX_INCLUDE_DEPTH {
        deep.push(PathBuf::from(format!("{i}.md"))).unwrap();
    }
    let reader = MystReader::new(ReaderContext {
        include_chain: deep,
        ..ReaderContext::new("article.md")
    });
    let doc = reader.read_str("```{include} too-deep.md\n```\n").unwrap();
    assert!(
        matches!(&doc.blocks[0].kind, BlockKind::Unmappable { reason, .. } if reason.contains("depth"))
    );
}

#[test]
fn loose_preservation_sidecar_mentions_remain_paragraphs() {
    let reader = MystReader::new(ReaderContext::new("article.md"));
    let doc = reader
        .read_str("See .mystquarto/preserved.json#b7f3 for details.\n")
        .unwrap();
    assert!(matches!(&doc.blocks[0].kind, BlockKind::Paragraph { .. }));
}

#[test]
fn final_multiline_paragraph_span_reaches_last_line() {
    let reader = MystReader::new(ReaderContext::new("article.md"));
    let doc = reader.read_str("first\nsecond\n").unwrap();
    assert_eq!(doc.blocks[0].span.start_line, 1);
    assert_eq!(doc.blocks[0].span.end_line, 2);
}

#[test]
fn myst_static_fences_and_tab_sets_parse_to_typed_blocks() {
    let reader = MystReader::new(ReaderContext::new("article.md"));
    let doc = reader
        .read_str("```python\nprint(1)\n```\n\n::::{tab-set}\n:::{tab-item} A\nAlpha\n:::\n::::\n")
        .unwrap();
    assert!(
        matches!(&doc.blocks[0].kind, BlockKind::StaticCode { lang: Some(lang), .. } if lang == "python")
    );
    assert!(
        matches!(&doc.blocks[1].kind, BlockKind::TabSet { items } if items.len() == 1 && items[0].label == "A")
    );
}

#[test]
fn myst_pending_targets_are_not_silently_dropped() {
    let reader = MystReader::new(ReaderContext::new("article.md"));
    let doc = reader
        .read_str("(a)=\n(b)=\n## Heading\n\n(c)=\n:::{figure} img.png\n:label: fig:x\n:::\n")
        .unwrap();
    assert!(matches!(&doc.blocks[0].kind, BlockKind::Target { label } if label.raw == "a"));
    assert!(
        matches!(&doc.blocks[1].kind, BlockKind::Heading { label: Some(label), .. } if label.raw == "b")
    );
    assert!(matches!(&doc.blocks[2].kind, BlockKind::Target { label } if label.raw == "c"));
    assert!(
        matches!(&doc.blocks[3].kind, BlockKind::Figure { label: Some(label), .. } if label.raw == "fig:x")
    );
}

#[test]
fn quarto_nested_divs_respect_outer_fence_count() {
    let reader = QuartoReader::new(ReaderContext::new("article.qmd"));
    let doc = reader
        .read_str(":::: {.callout-note}\nText\n::: {.column-margin}\nNested\n:::\n::::\n")
        .unwrap();
    assert_eq!(doc.blocks.len(), 1);
    assert!(matches!(&doc.blocks[0].kind, BlockKind::Admonition { body, .. } if body.len() == 2));
}

#[test]
fn quarto_inline_or_unknown_shortcodes_are_unmappable() {
    let reader = QuartoReader::new(ReaderContext::new("article.qmd"));
    let doc = reader
        .read_str("- {{< include _part.qmd >}}\n\n{{< doesnotexist x >}}\n")
        .unwrap();
    assert!(matches!(&doc.blocks[0].kind, BlockKind::Unmappable { .. }));
    assert!(matches!(&doc.blocks[1].kind, BlockKind::Unmappable { .. }));
}

#[test]
fn myst_dollar_math_parses_to_math_block() {
    let reader = MystReader::new(ReaderContext::new("article.md"));
    let doc = reader.read_str("$$\nx + y\n$$\n").unwrap();
    assert!(
        matches!(&doc.blocks[0].kind, BlockKind::Math { body, label: None } if body == &vec!["x + y".to_string()])
    );
}

#[test]
fn quarto_raw_fence_is_not_executable_code_cell() {
    let reader = QuartoReader::new(ReaderContext::new("article.qmd"));
    let doc = reader.read_str("```{=latex}\n\\newpage\n```\n").unwrap();
    assert_eq!(doc.engine, None);
    assert!(
        matches!(&doc.blocks[0].kind, BlockKind::Raw { format, body } if format == "latex" && body == &vec!["\\newpage".to_string()])
    );
}

#[test]
fn quarto_embed_notebook_paths_are_guarded() {
    let tmp = tempdir("embed-escape");
    let input = tmp.join("input");
    fs::create_dir_all(&input).unwrap();
    fs::write(tmp.join("analysis.ipynb"), "{}").unwrap();
    let root = canonicalize_root(&input).unwrap();
    let reader =
        QuartoReader::new(ReaderContext::new(input.join("article.qmd")).with_input_root(root));
    let doc = reader
        .read_str("{{< embed ../analysis.ipynb#fig-analysis >}}\n")
        .unwrap();
    assert!(
        matches!(&doc.blocks[0].kind, BlockKind::Unmappable { reason, .. } if reason.contains("escapes")),
        "embed notebook traversal should be refused before writers consume it"
    );
    cleanup(&tmp);
}

fn tempdir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mystquarto-reader-test-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn cleanup(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}
