//! Path safety: no conversion reads or writes outside its declared roots,
//! follows a symlink out of the input tree, or recurses into its own
//! output.
//!
//! Every function here treats "declared root" as something the caller
//! canonicalizes exactly once, at the start of a run ([`canonicalize_root`]),
//! and then passes down. Per-call canonicalization of paths that are known
//! to already be inside a canonicalized root would be redundant I/O; this
//! module canonicalizes only the parts of a path that are not already known
//! to be canonical (a target being resolved for the first time).
//!
//! Error variant names are chosen so a later diagnostics layer (Phase 7) can
//! wrap them with a code like `MQ06xx` without renaming anything here — see
//! [`PathGuardError`].

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Maximum depth of a resolved include chain (`{include} a.md` including
/// `b.md` including `c.md`, …). MyST/Quarto documents in practice nest
/// includes at most two or three levels; this cap exists purely as a
/// circuit breaker against a runaway or maliciously deep chain, not because
/// legitimate documents approach it. 32 is a generous round power-of-two —
/// deep enough that no real document should ever hit it, shallow enough
/// that hitting it is unambiguously a sign of trouble (a cycle the
/// same-canonical-path check somehow missed, or a deliberately adversarial
/// input) rather than a false positive on a legitimate deeply-nested doc.
pub const MAX_INCLUDE_DEPTH: usize = 32;

/// An error from a path-guard check. Every variant corresponds to one of
/// the phase spec's required refusals; `Display` produces a plain
/// human-readable message today, and the variant itself (not the message
/// text) is what a future diagnostics layer should match on to attach a
/// stable code.
#[derive(Debug)]
pub enum PathGuardError {
    /// `fs::canonicalize` failed on a path this module needed to resolve
    /// (typically because no ancestor of it exists, or a permissions
    /// error).
    Canonicalize { path: PathBuf, source: io::Error },
    /// The resolved path is not the declared root or a descendant of it —
    /// via `..` traversal, a symlink, or any other means. Covers both the
    /// "include escapes the project root" and "output root escapes the
    /// input root" (D16-adjacent, though D16 itself is the opposite
    /// direction — see [`effective_output_excluded_from_walk`]) cases.
    EscapesRoot { path: PathBuf, root: PathBuf },
    /// An include/embed/toc target was given as an absolute path. Refused
    /// unconditionally — no opt-in flag — because an absolute path
    /// sidesteps root-relative resolution entirely; a project-relative
    /// target is always expressible without one.
    AbsoluteTarget { path: PathBuf },
    /// `path` is already present in the current include-resolution chain —
    /// including it again would recurse forever.
    IncludeCycle { path: PathBuf },
    /// Resolving one more include would exceed [`MAX_INCLUDE_DEPTH`].
    DepthExceeded { depth: usize, max: usize },
}

impl fmt::Display for PathGuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathGuardError::Canonicalize { path, source } => {
                write!(f, "could not resolve path {}: {source}", path.display())
            }
            PathGuardError::EscapesRoot { path, root } => write!(
                f,
                "{} escapes its declared root {}",
                path.display(),
                root.display()
            ),
            PathGuardError::AbsoluteTarget { path } => write!(
                f,
                "{} is an absolute path; include/embed/toc targets must be project-relative",
                path.display()
            ),
            PathGuardError::IncludeCycle { path } => {
                write!(f, "include cycle detected at {}", path.display())
            }
            PathGuardError::DepthExceeded { depth, max } => {
                write!(f, "include depth {depth} exceeds the maximum of {max}")
            }
        }
    }
}

impl std::error::Error for PathGuardError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PathGuardError::Canonicalize { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Canonicalizes a run's declared input or output root. Call this exactly
/// once per root, at the start of a run, before any other path-guard
/// function is used — everything else here assumes its `root`/
/// `declared_root` argument is already in this form.
///
/// # Errors
/// Returns [`PathGuardError::Canonicalize`] if `root` does not exist or
/// cannot be resolved (permissions, a broken symlink as the root itself,
/// etc).
pub fn canonicalize_root(root: &Path) -> Result<PathBuf, PathGuardError> {
    fs::canonicalize(root).map_err(|source| PathGuardError::Canonicalize {
        path: root.to_path_buf(),
        source,
    })
}

/// Resolves `path` to canonical form even if it (or a trailing part of it)
/// does not exist yet — the case an output file is in before it has been
/// written for the first time. Walks up to the nearest existing ancestor,
/// canonicalizes that (resolving any symlinks in it), then re-appends the
/// non-existent trailing components verbatim (a component that does not
/// exist cannot be a symlink, so there is nothing to resolve in it).
///
/// # Errors
/// Returns [`PathGuardError::Canonicalize`] only if no ancestor of `path`
/// exists at all (not even `/`), which in practice should not happen.
pub fn canonicalize_best_effort(path: &Path) -> Result<PathBuf, PathGuardError> {
    let mut existing = path;
    let mut trailing: Vec<std::ffi::OsString> = Vec::new();

    loop {
        match fs::canonicalize(existing) {
            Ok(mut base) => {
                for component in trailing.into_iter().rev() {
                    base.push(component);
                }
                return Ok(base);
            }
            Err(_) => {
                let Some(parent) = existing.parent() else {
                    return Err(PathGuardError::Canonicalize {
                        path: path.to_path_buf(),
                        source: io::Error::new(
                            io::ErrorKind::NotFound,
                            "no existing ancestor to canonicalize",
                        ),
                    });
                };
                if let Some(name) = existing.file_name() {
                    trailing.push(name.to_os_string());
                }
                existing = parent;
            }
        }
    }
}

/// Returns `true` if `candidate` is `root` itself or a descendant of it.
/// Both arguments **must already be canonicalized** — this is a pure
/// path-component comparison (`Path::starts_with`, which compares
/// components, not raw bytes), never a string-prefix check, so it isn't
/// fooled by e.g. `/in` vs `/input` sharing a string prefix but not a path
/// component prefix.
#[must_use]
pub fn is_descendant(root: &Path, candidate: &Path) -> bool {
    candidate.starts_with(root)
}

/// The actual D16 fix: returns `true` if `candidate_dir` — a directory
/// encountered while walking some tree — is the effective output root or a
/// descendant of it, and should therefore be pruned from that walk.
/// `discover.rs` and `assets.rs` both call this on every directory they are
/// about to descend into, so an output directory nested inside the input
/// tree is excluded from **both** the discovery walk and the asset walk,
/// instead of being walked into and having its own contents copied one
/// level deeper (the reproduced `docs-quarto/docs-quarto/` defect).
///
/// `effective_output_root` and `candidate_dir` should both be resolved the
/// same way (either both canonicalized, or both left as plain joined paths
/// from a canonical root with no `..`/symlink indirection in between) so
/// the component comparison in [`is_descendant`] is meaningful.
#[must_use]
pub fn effective_output_excluded_from_walk(
    effective_output_root: &Path,
    candidate_dir: &Path,
) -> bool {
    is_descendant(effective_output_root, candidate_dir)
}

/// Resolves `target` (as written in a document — an include target, figure
/// source, or toc entry) against `base_dir` (the directory containing the
/// document that referenced it), then asserts the result is a descendant of
/// `declared_root` (the canonicalized input or output root for this run).
///
/// Refuses:
/// - an absolute `target` outright ([`PathGuardError::AbsoluteTarget`]) —
///   see that variant's docs for why there's no opt-in.
/// - a resolved path that escapes `declared_root`, whether via `..`
///   traversal or a symlink resolved during canonicalization
///   ([`PathGuardError::EscapesRoot`]).
///
/// `target` does not need to exist: see [`canonicalize_best_effort`].
pub fn guard_target(
    declared_root: &Path,
    base_dir: &Path,
    target: &Path,
) -> Result<PathBuf, PathGuardError> {
    if target.is_absolute() {
        return Err(PathGuardError::AbsoluteTarget {
            path: target.to_path_buf(),
        });
    }

    let joined = base_dir.join(target);
    let resolved = canonicalize_best_effort(&joined)?;

    if !is_descendant(declared_root, &resolved) {
        return Err(PathGuardError::EscapesRoot {
            path: resolved,
            root: declared_root.to_path_buf(),
        });
    }

    Ok(resolved)
}

/// Tracks the canonical paths currently being resolved along one include
/// chain, so `a` including `b` including `a` is caught as a cycle before it
/// recurses forever, and enforces [`MAX_INCLUDE_DEPTH`].
///
/// Usage: push the canonical target before recursing into it, pop it after
/// returning.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IncludeChain {
    stack: Vec<PathBuf>,
}

impl IncludeChain {
    /// Creates an empty chain (the top-level document being converted has
    /// no includes resolved yet).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Current chain depth (number of includes currently being resolved).
    #[must_use]
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Attempts to push `canonical_target` onto the chain.
    ///
    /// # Errors
    /// [`PathGuardError::IncludeCycle`] if `canonical_target` is already on
    /// the chain; [`PathGuardError::DepthExceeded`] if pushing it would
    /// exceed [`MAX_INCLUDE_DEPTH`].
    pub fn push(&mut self, canonical_target: PathBuf) -> Result<(), PathGuardError> {
        if self.stack.len() >= MAX_INCLUDE_DEPTH {
            return Err(PathGuardError::DepthExceeded {
                depth: self.stack.len() + 1,
                max: MAX_INCLUDE_DEPTH,
            });
        }
        if self.stack.contains(&canonical_target) {
            return Err(PathGuardError::IncludeCycle {
                path: canonical_target,
            });
        }
        self.stack.push(canonical_target);
        Ok(())
    }

    /// Pops the most recently pushed target — call after returning from
    /// resolving it, regardless of whether that resolution succeeded.
    pub fn pop(&mut self) {
        self.stack.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn symlink_escaping_input_root_is_refused() {
        let tmp = tempdir();
        let input_root = tmp.join("input");
        let outside = tmp.join("outside");
        fs::create_dir_all(&input_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let secret = outside.join("secret.txt");
        write(&secret, "top secret");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, input_root.join("escape.md")).unwrap();
        #[cfg(not(unix))]
        panic!("this test requires unix symlink support");

        let canonical_root = canonicalize_root(&input_root).unwrap();
        let err = guard_target(&canonical_root, &input_root, Path::new("escape.md"))
            .expect_err("symlink escaping the input root must be refused");
        assert!(matches!(err, PathGuardError::EscapesRoot { .. }));

        cleanup(&tmp);
    }

    #[test]
    fn dot_dot_traversal_outside_root_is_refused() {
        let tmp = tempdir();
        let input_root = tmp.join("input");
        fs::create_dir_all(input_root.join("sub")).unwrap();
        write(&tmp.join("secret.txt"), "outside");

        let canonical_root = canonicalize_root(&input_root).unwrap();
        let err = guard_target(
            &canonical_root,
            &input_root.join("sub"),
            Path::new("../../secret.txt"),
        )
        .expect_err("`..` traversal outside the root must be refused");
        assert!(matches!(err, PathGuardError::EscapesRoot { .. }));

        cleanup(&tmp);
    }

    #[test]
    fn dot_dot_traversal_staying_inside_root_is_allowed() {
        let tmp = tempdir();
        let input_root = tmp.join("input");
        fs::create_dir_all(input_root.join("sub")).unwrap();
        write(&input_root.join("shared.md"), "shared");

        let canonical_root = canonicalize_root(&input_root).unwrap();
        let resolved = guard_target(
            &canonical_root,
            &input_root.join("sub"),
            Path::new("../shared.md"),
        )
        .expect("`..` that stays inside the root must be allowed");
        assert_eq!(resolved, canonical_root.join("shared.md"));

        cleanup(&tmp);
    }

    #[test]
    fn absolute_include_target_is_refused() {
        let tmp = tempdir();
        let input_root = tmp.join("input");
        fs::create_dir_all(&input_root).unwrap();

        let canonical_root = canonicalize_root(&input_root).unwrap();
        let absolute = if cfg!(windows) {
            PathBuf::from("C:\\etc\\passwd")
        } else {
            PathBuf::from("/etc/passwd")
        };
        let err = guard_target(&canonical_root, &input_root, &absolute)
            .expect_err("an absolute include target must be refused");
        assert!(matches!(err, PathGuardError::AbsoluteTarget { .. }));

        cleanup(&tmp);
    }

    #[test]
    fn output_inside_input_is_detected_as_descendant() {
        let tmp = tempdir();
        let input_root = tmp.join("input");
        let output_root = input_root.join("docs-quarto");
        fs::create_dir_all(&output_root).unwrap();

        let canonical_input = canonicalize_root(&input_root).unwrap();
        let canonical_output = canonicalize_root(&output_root).unwrap();

        assert!(is_descendant(&canonical_input, &canonical_output));
        assert!(effective_output_excluded_from_walk(
            &canonical_output,
            &canonical_output.join("images")
        ));
        assert!(effective_output_excluded_from_walk(
            &canonical_output,
            &canonical_output
        ));
        assert!(!effective_output_excluded_from_walk(
            &canonical_output,
            &canonical_input.join("images")
        ));

        cleanup(&tmp);
    }

    #[test]
    fn include_cycle_is_rejected() {
        let mut chain = IncludeChain::new();
        let a = PathBuf::from("/project/a.md");
        let b = PathBuf::from("/project/b.md");
        chain.push(a.clone()).unwrap();
        chain.push(b).unwrap();
        let err = chain
            .push(a)
            .expect_err("re-pushing a path already on the chain must be a cycle");
        assert!(matches!(err, PathGuardError::IncludeCycle { .. }));
    }

    #[test]
    fn include_depth_beyond_cap_is_rejected() {
        let mut chain = IncludeChain::new();
        for i in 0..MAX_INCLUDE_DEPTH {
            chain
                .push(PathBuf::from(format!("/project/{i}.md")))
                .unwrap_or_else(|e| panic!("push {i} should succeed, got {e}"));
        }
        assert_eq!(chain.depth(), MAX_INCLUDE_DEPTH);

        let err = chain
            .push(PathBuf::from("/project/one-too-many.md"))
            .expect_err("pushing beyond MAX_INCLUDE_DEPTH must be rejected");
        assert!(matches!(err, PathGuardError::DepthExceeded { .. }));
    }

    #[test]
    fn canonicalize_best_effort_resolves_nonexistent_trailing_components() {
        let tmp = tempdir();
        let root = tmp.join("root");
        fs::create_dir_all(&root).unwrap();

        let not_yet_written = root.join("out").join("doc.qmd");
        let resolved = canonicalize_best_effort(&not_yet_written).unwrap();
        let canonical_root = canonicalize_root(&root).unwrap();
        assert_eq!(resolved, canonical_root.join("out").join("doc.qmd"));

        cleanup(&tmp);
    }

    // --- test helpers -------------------------------------------------

    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mystquarto-path-guard-test-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn unique_suffix() -> u128 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        nanos + n as u128
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }
}
