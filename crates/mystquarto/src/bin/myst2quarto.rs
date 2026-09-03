//! `myst2quarto` — standalone MyST->Quarto binary, one of the three
//! `[project.scripts]` entry points the Python `pyproject.toml` exposes
//! (`myst2quarto`, `quarto2myst`, `mystquarto`) that Phase 1's ported
//! bucket-B tests invoke by name.
#![forbid(unsafe_code)]

use clap::Parser;

use mystquarto::args::Myst2QuartoCli;
use mystquarto::discover::Direction;
use mystquarto::orchestrate::{execute, print_summary};

fn main() {
    let cli = Myst2QuartoCli::parse();

    match execute(&cli.args, Direction::MystToQuarto) {
        Ok(report) => {
            let code = print_summary(&report, cli.args.dry_run);
            std::process::exit(code);
        }
        Err(err) => {
            eprintln!("error: {err:?}");
            std::process::exit(1);
        }
    }
}
