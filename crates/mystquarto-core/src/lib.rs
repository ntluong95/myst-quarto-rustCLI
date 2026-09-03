//! Core types for the `mystquarto` Rust port: the `mappings.toml`
//! conversion contract (Phase 2), and the document IR, leaf types
//! (`Span`, `Label`), and YAML strategy (Phase 3). Readers, writers, and
//! the file orchestration contract are later phases' responsibility.
#![forbid(unsafe_code)]

pub mod ir;
pub mod label;
pub mod mappings;
pub mod span;
pub mod yaml;

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
pub use span::Span;
pub use yaml::YamlValue;
