//! File orchestration primitives shared by every phase that reads or
//! writes a conversion's input/output tree: path safety (`path_guard`),
//! asset copying (`assets`), and atomic single-file writes (`atomic`).
//!
//! These live in the library crate, not the `mystquarto` binary crate,
//! because Phase 4/5/6 need path-guard and atomic-write primitives too, not
//! just this phase's CLI. `discover.rs` (walking a directory to decide
//! *which* files a CLI invocation touches) stays in the binary crate — see
//! its module docs for why that one split the other way.

pub mod assets;
pub mod atomic;
pub mod path_guard;
