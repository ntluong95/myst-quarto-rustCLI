//! Core types for the `mystquarto` Rust port: the `mappings.toml`
//! conversion contract, the document IR, leaf types (`Span`, `Label`), YAML
//! strategy, MyST/Quarto readers, IR->text writers, label normalization
//! (`registry`), notebook cell relabelling, batch conversion (`pipeline`),
//! and project/page config mapping (`config`, `frontmatter`).
#![forbid(unsafe_code)]

pub mod config;
pub mod diagnostics;
pub mod frontmatter;
pub mod fs;
pub mod ir;
pub mod label;
pub mod mappings;
pub mod notebook;
pub mod pipeline;
pub mod preserve;
pub mod reader;
pub mod registry;
pub mod span;
pub mod writer;
pub mod yaml;

pub use diagnostics::{Diagnostic, Severity};
pub use ir::{
    AdmonitionKind, Attrs, Block, BlockKind, CellOptions, CommentStyle, Document, EmbedTarget,
    Engine, FigureSource, Frontmatter, IncludeOpts, TabItem,
};
pub use label::Label;
pub use mappings::{
    directive_by_myst_name, directive_by_quarto_class, mappings, ConfigFieldMapping,
    DirectiveMapping, ExportFormatMapping, Fidelity, InlineMapping, LabelPrefixMapping,
    LegacyRoleMapping, Mappings, RoleMapping, StructuralMapping,
};
pub use reader::{
    MystReader, NotebookCellIndex, PreservationStore, QuartoReader, ReaderContext, ReaderError,
};
pub use registry::{LabelRegistry, RefKind};
pub use span::Span;
pub use yaml::YamlValue;
