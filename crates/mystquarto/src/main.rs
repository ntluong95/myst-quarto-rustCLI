//! `mystquarto` — the multi-purpose dispatcher binary: `mystquarto
//! to-quarto <path>` / `mystquarto to-myst <path>`. With no subcommand,
//! prints help and exits `0` (matching the Python `click.Group
//! (invoke_without_command=True)` behavior — `mystquarto` alone is not an
//! error, `tests/test_cli.py::TestMainSubcommands::test_main_no_subcommand`
//! asserts exactly this).
#![forbid(unsafe_code)]

use clap::{CommandFactory, Parser};

use mystquarto::args::{MystquartoCli, MystquartoCommand};
use mystquarto::discover::Direction;
use mystquarto::orchestrate::{execute, print_summary};

fn main() {
    let cli = MystquartoCli::parse();

    let Some(command) = cli.command else {
        let _ = MystquartoCli::command().print_help();
        println!();
        std::process::exit(0);
    };

    let (convert_args, direction) = match command {
        MystquartoCommand::ToQuarto(a) => (a, Direction::MystToQuarto),
        MystquartoCommand::ToMyst(a) => (a, Direction::QuartoToMyst),
    };

    match execute(&convert_args, direction) {
        Ok(report) => {
            let code = print_summary(&report, convert_args.dry_run);
            std::process::exit(code);
        }
        Err(err) => {
            eprintln!("error: {err:?}");
            std::process::exit(1);
        }
    }
}
