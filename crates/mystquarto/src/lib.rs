//! Library half of the `mystquarto` package: shared by all three binaries
//! (`mystquarto`, `myst2quarto`, `quarto2myst`) so they can reuse argument
//! parsing, discovery, and orchestration without duplicating source across
//! `src/bin/*.rs` files.
//!
//! Not listed in the phase spec's "Related Code Files" (which names only
//! `main.rs`, `args.rs`, `discover.rs`), but standard practice for a
//! package with one default binary (`main.rs`) plus additional binaries
//! under `src/bin/`: without a `lib.rs`, each `src/bin/*.rs` file is its own
//! crate root with no visibility into `main.rs`'s modules. Adding this file
//! is the idiomatic way to let `bin/myst2quarto.rs` and `bin/quarto2myst.rs`
//! reuse `args`/`discover`/`orchestrate` instead of copy-pasting their
//! contents three times — flagged in this phase's report as a deviation
//! worth a sanity check, not hidden.
#![forbid(unsafe_code)]

pub mod args;
pub mod discover;
pub mod orchestrate;
