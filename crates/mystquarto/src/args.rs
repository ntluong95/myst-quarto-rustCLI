//! `clap` argument structs for all three binaries.
//!
//! The flag surface is unchanged from the Python CLI (`-o/--output`,
//! `--in-place`, `--config-only`, `--no-config`, `--dry-run`, `--strict`),
//! plus this phase's `--force` (required by the `--in-place` safety
//! contract — see `crate::orchestrate`). `--no-preserve` and
//! `--format json` are deliberately **not** present: the phase spec drops
//! both (they contradicted later-phase decisions). `--no-label-map` is
//! present but inert this phase — see its field doc.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Flags shared by all three binaries for a single conversion invocation.
/// The direction (MyST->Quarto vs. Quarto->MyST) is fixed by which binary,
/// or which `mystquarto` subcommand, is running — it is not a flag on this
/// struct.
#[derive(Args, Debug, Clone)]
pub struct ConvertArgs {
    /// Input file or directory to convert.
    pub input: PathBuf,

    /// Output directory. Defaults to `<input>-quarto` / `<input>-myst`
    /// (matching the Python CLI). Ignored when `--in-place` is set — see
    /// that field's doc.
    #[arg(short = 'o', long = "output", value_name = "DIR")]
    pub output: Option<PathBuf>,

    /// Overwrite source files in place instead of writing to a separate
    /// output directory. When set, `--output` is ignored (parity with the
    /// Python CLI, whose `convert_directory` computes
    /// `effective_output_dir = input_dir` before even looking at
    /// `output_dir`). Deletes each source file only after its own output
    /// has been written and renamed successfully — see
    /// `crate::orchestrate`'s module docs for the full contract this flag
    /// implies (also gated by `--force` and a clean VCS state).
    #[arg(long = "in-place")]
    pub in_place: bool,

    /// Convert only the config file (`myst.yml` / `_quarto.yml`); skip
    /// content files.
    #[arg(long = "config-only")]
    pub config_only: bool,

    /// Skip config file conversion; convert content files only.
    #[arg(long = "no-config")]
    pub no_config: bool,

    /// Report what would happen; write nothing. Verified to write zero
    /// bytes on every flag combination — see `tests/cli.rs`.
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Treat warnings as errors. Accepted and stored this phase; there is
    /// no warning collector yet to promote (Phase 7 owns diagnostics), so
    /// it currently has no observable effect. Kept in the flag surface now
    /// so Phase 7 is additive rather than a CLI-breaking change.
    #[arg(long = "strict")]
    pub strict: bool,

    /// Required alongside `--in-place` to bypass the hand-authored-config
    /// overwrite gate and the clean-VCS-state gate. See
    /// `crate::orchestrate::check_in_place_preconditions`.
    #[arg(long = "force")]
    pub force: bool,

    /// Retained for CLI compatibility with the Python tool. The label-map
    /// sidecar this flag would suppress does not exist until a later
    /// phase's sidecar work; this phase accepts and stores the flag but
    /// wires no behavior to it.
    #[arg(long = "no-label-map")]
    pub no_label_map: bool,
}

/// `myst2quarto <args>` — the standalone MyST->Quarto binary.
#[derive(Parser, Debug)]
#[command(
    name = "myst2quarto",
    about = "Convert MyST markdown files to Quarto format"
)]
pub struct Myst2QuartoCli {
    #[command(flatten)]
    pub args: ConvertArgs,
}

/// `quarto2myst <args>` — the standalone Quarto->MyST binary.
#[derive(Parser, Debug)]
#[command(
    name = "quarto2myst",
    about = "Convert Quarto markdown files to MyST format"
)]
pub struct Quarto2MystCli {
    #[command(flatten)]
    pub args: ConvertArgs,
}

/// `mystquarto [to-quarto|to-myst] <args>` — the multi-purpose dispatcher
/// binary. With no subcommand it prints help and exits `0` (matching the
/// Python `click.Group(invoke_without_command=True)` behavior:
/// `mystquarto` alone is not an error).
#[derive(Parser, Debug)]
#[command(name = "mystquarto", about = "Bidirectional MyST <-> Quarto converter")]
pub struct MystquartoCli {
    #[command(subcommand)]
    pub command: Option<MystquartoCommand>,
}

#[derive(Subcommand, Debug)]
pub enum MystquartoCommand {
    /// Convert MyST markdown files to Quarto format.
    #[command(name = "to-quarto")]
    ToQuarto(ConvertArgs),
    /// Convert Quarto markdown files to MyST format.
    #[command(name = "to-myst")]
    ToMyst(ConvertArgs),
}
