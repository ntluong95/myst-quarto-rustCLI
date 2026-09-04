//! `quarto2myst` — standalone Quarto->MyST binary. See `myst2quarto.rs`'s
//! module docs for why this exists as its own binary rather than only a
//! `mystquarto` subcommand.
#![forbid(unsafe_code)]

use clap::Parser;

use mystquarto::args::Quarto2MystCli;
use mystquarto::discover::Direction;
use mystquarto::orchestrate::{execute, print_summary};

fn main() {
    let cli = Quarto2MystCli::parse();

    match execute(&cli.args, Direction::QuartoToMyst) {
        Ok(report) => {
            let code = print_summary(&report, cli.args.dry_run, cli.args.strict);
            std::process::exit(code);
        }
        Err(err) => {
            eprintln!("error: {err:?}");
            std::process::exit(1);
        }
    }
}
