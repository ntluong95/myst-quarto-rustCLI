//! Core types for the `mystquarto` Rust port: currently just the
//! `mappings.toml` conversion contract (Phase 2). The IR, readers, writers
//! and orchestration contract are Phase 3's responsibility.

pub mod mappings;

pub use mappings::{
    directive_by_myst_name, directive_by_quarto_class, mappings, ConfigFieldMapping,
    DirectiveMapping, ExportFormatMapping, Fidelity, InlineMapping, LabelPrefixMapping,
    LegacyRoleMapping, Mappings, RoleMapping, StructuralMapping,
};
