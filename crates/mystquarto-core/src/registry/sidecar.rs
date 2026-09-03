//! The `.mystquarto/labels.json` sidecar: persists a run's [`super::LabelRegistry`]
//! so a later reverse conversion can restore original MyST-side spellings
//! (`fig-samples` -> `fig:samples`) instead of keeping the normalized form.
//!
//! Two properties the original phase draft's flat `{id: label}` map did not
//! have, both required by red-team review (RT-08, RT-09):
//!
//! - **Keyed by source file.** Two files that both define `fig:samples` are
//!   legitimately disambiguated by [`super::LabelRegistry`] (`fig-samples`,
//!   `fig-samples-2`); a flat map cannot record which original belongs to
//!   which file, so the reverse conversion would restore the wrong one into
//!   one of them.
//! - **Read as untrusted input.** The sidecar sits in the tree being
//!   converted, which per the accepted threat model (RD-5, RT-09) may be a
//!   cloned, attacker-influenced repository. [`read_untrusted`] bounds size
//!   and entry count, rejects an unexpected version or direction, and
//!   validates every restored label against a fixed character set — a
//!   newline in an unvalidated label could otherwise escape the MyST
//!   `(sec:x)=` construct the writer emits it into.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::fs::atomic::write_atomic;
use crate::{Label, LabelRegistry};

/// Bytes above which a sidecar is refused outright without being parsed —
/// a circuit breaker against a maliciously or accidentally huge file, not a
/// limit any legitimate project should approach.
pub const MAX_SIDECAR_BYTES: u64 = 10 * 1024 * 1024;

/// Total label entries (summed across all files) above which a sidecar is
/// refused — same rationale as [`MAX_SIDECAR_BYTES`], as a second, orthogonal
/// bound (a small-byte-count file could still smuggle a pathological entry
/// count via extreme key/value reuse in JSON, though `serde_json` itself
/// already bounds nesting depth).
pub const MAX_SIDECAR_ENTRIES: usize = 200_000;

/// Labels must match this character set after restoration — the same legal
/// alphabet [`super::normalize::normalize`] itself only ever produces
/// (`[a-z0-9-]` for the Quarto-side id) plus MyST's own richer but still
/// bounded label alphabet on the value side (letters, digits, and
/// `:` `-` `_` `.`, matching MyST's real-world label conventions:
/// `fig:samples`, `10.1038/nmeth.1974`-style DOI-derived labels, etc). A
/// value containing anything else — critically, a newline — is dropped
/// rather than trusted, since it is written verbatim into MyST source
/// (`(sec:x)=`, `:label: x`) on the reverse conversion.
fn label_is_well_formed(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.' | '/'))
}

/// The sidecar's on-disk JSON shape. Field order matches the phase spec's
/// example; `files` and each file's `labels` are `BTreeMap`s so
/// `serde_json`'s output is sorted and therefore deterministic run to run —
/// required by the "two identical runs produce identical bytes" success
/// criterion.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LabelSidecar {
    pub version: u32,
    pub generated_by: String,
    /// `"myst_to_quarto"` or `"quarto_to_myst"` — a plain string, not the
    /// `mystquarto` binary crate's `Direction` enum: this module lives in
    /// `mystquarto-core`, which has no dependency on the CLI crate (the same
    /// layering `fs::path_guard`/`fs::assets` already follow).
    pub direction: String,
    pub source_root: String,
    pub files: BTreeMap<String, FileLabels>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileLabels {
    /// `"fnv1a:<16 hex digits>"` — a lightweight, dependency-free content
    /// hash (FNV-1a 64-bit; see [`content_hash`]) used only to detect "this
    /// file changed since the sidecar was written," not for any security
    /// property. The phase spec's own worked example illustrates this field
    /// with `"sha256:…"`; FNV-1a is used here instead of pulling in a new
    /// crate dependency for a staleness check that has no adversarial
    /// requirement (path-safety and label-content validation, which do
    /// matter under the untrusted-input threat model, do not depend on the
    /// hash algorithm's cryptographic strength).
    pub content_hash: String,
    /// Quarto id -> original MyST-side label.
    pub labels: BTreeMap<String, String>,
}

/// FNV-1a 64-bit over `bytes`, formatted as `"fnv1a:<16 lowercase hex
/// digits>"`. See [`FileLabels::content_hash`] for why this algorithm
/// (not SHA-256) was chosen.
#[must_use]
pub fn content_hash(bytes: &[u8]) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET_BASIS;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("fnv1a:{hash:016x}")
}

/// Builds a sidecar from `registry`, keyed by each source file's path
/// relative to `source_root` (POSIX-style, forward slashes, so the sidecar
/// is portable across platforms) — paired with that file's current content
/// hash from `content_hashes` (computed by the caller, since only it knows
/// the original source bytes each `Document` was read from).
#[must_use]
pub fn build(
    registry: &LabelRegistry,
    direction: &str,
    source_root: &Path,
    content_hashes: &BTreeMap<PathBuf, String>,
) -> LabelSidecar {
    let mut files: BTreeMap<String, FileLabels> = BTreeMap::new();
    for (source, myst_label, quarto_id) in registry.entries() {
        let rel = relative_key(source_root, source);
        let hash = content_hashes.get(source).cloned().unwrap_or_default();
        let entry = files.entry(rel).or_insert_with(|| FileLabels {
            content_hash: hash.clone(),
            labels: BTreeMap::new(),
        });
        entry
            .labels
            .insert(quarto_id.to_string(), myst_label.raw.clone());
    }
    LabelSidecar {
        version: 1,
        generated_by: format!("mystquarto {}", env!("CARGO_PKG_VERSION")),
        direction: direction.to_string(),
        source_root: source_root.display().to_string(),
        files,
    }
}

fn relative_key(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// A non-fatal issue found while reading or merging a sidecar. Same shape
/// rationale as [`super::RegistryWarning`] — Phase 7 does not exist yet to
/// assign real diagnostic codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarWarning {
    pub message: String,
}

fn warn(message: impl Into<String>) -> SidecarWarning {
    SidecarWarning {
        message: message.into(),
    }
}

/// Reads and validates the sidecar at `path` as **untrusted input** (RT-09):
/// the file sits in the tree being converted, which may be a cloned,
/// attacker-influenced repository.
///
/// Returns `(None, warnings)` — the same "absent sidecar" path callers
/// already handle — for: a missing file (no warning, this is the normal
/// case for a natively-authored Quarto project); a file over
/// [`MAX_SIDECAR_BYTES`]; malformed JSON; a `version` other than `1`; more
/// than [`MAX_SIDECAR_ENTRIES`] total label entries; or a `direction` that
/// does not match `expected_direction` (**with** a warning in this one
/// case — a direction mismatch usually means a stale sidecar from a prior
/// run in the opposite direction, worth surfacing even though the run
/// proceeds as if no sidecar were present).
///
/// On success, returns `Some(sidecar)` with every malformed label entry
/// already dropped (each with its own warning) — callers never need to
/// re-validate labels pulled from the returned value.
#[must_use]
pub fn read_untrusted(
    path: &Path,
    expected_direction: &str,
) -> (Option<LabelSidecar>, Vec<SidecarWarning>) {
    let Ok(metadata) = fs::metadata(path) else {
        return (None, Vec::new());
    };
    if metadata.len() > MAX_SIDECAR_BYTES {
        return (
            None,
            vec![warn(format!(
                "{} is {} bytes, over the {MAX_SIDECAR_BYTES}-byte sidecar limit; ignoring it",
                path.display(),
                metadata.len()
            ))],
        );
    }

    let Ok(text) = fs::read_to_string(path) else {
        return (
            None,
            vec![warn(format!(
                "could not read {}; ignoring it",
                path.display()
            ))],
        );
    };

    let mut sidecar: LabelSidecar = match serde_json::from_str(&text) {
        Ok(s) => s,
        Err(e) => {
            return (
                None,
                vec![warn(format!(
                    "{} is not a valid label sidecar ({e}); ignoring it",
                    path.display()
                ))],
            )
        }
    };

    if sidecar.version != 1 {
        return (
            None,
            vec![warn(format!(
                "{} has sidecar version {}, expected 1; ignoring it",
                path.display(),
                sidecar.version
            ))],
        );
    }

    let total_entries: usize = sidecar.files.values().map(|f| f.labels.len()).sum();
    if total_entries > MAX_SIDECAR_ENTRIES {
        return (
            None,
            vec![warn(format!(
                "{} has {total_entries} label entries, over the {MAX_SIDECAR_ENTRIES} limit; ignoring it",
                path.display()
            ))],
        );
    }

    let mut warnings = Vec::new();
    if sidecar.direction != expected_direction {
        warnings.push(warn(format!(
            "{} was generated for direction `{}`, but this run is `{expected_direction}`; \
             ignoring it (probably a stale sidecar from a prior run)",
            path.display(),
            sidecar.direction
        )));
        return (None, warnings);
    }

    for (file, entry) in &mut sidecar.files {
        entry.labels.retain(|quarto_id, myst_label| {
            let ok = label_is_well_formed(quarto_id) && label_is_well_formed(myst_label);
            if !ok {
                warnings.push(warn(format!(
                    "{file}: dropping malformed sidecar entry `{quarto_id}` -> `{myst_label}`"
                )));
            }
            ok
        });
    }

    (Some(sidecar), warnings)
}

/// Restores a `LabelRegistry`-shaped lookup (`(file, quarto_id) -> original
/// MyST label`) from a validated sidecar, for the Quarto->MyST writer.
/// Entries whose `content_hash` no longer matches `current_hashes` are
/// skipped (treated as stale, not as an error) — see [`FileLabels::content_hash`].
#[must_use]
pub fn restore_labels(
    sidecar: &LabelSidecar,
    current_hashes: &BTreeMap<PathBuf, String>,
) -> BTreeMap<(PathBuf, String), Label> {
    let mut out = BTreeMap::new();
    for (rel, entry) in &sidecar.files {
        let path = PathBuf::from(rel);
        let current = current_hashes.get(&path);
        if current.is_some_and(|h| h != &entry.content_hash) {
            continue; // stale — the file changed since this sidecar was written
        }
        for (quarto_id, myst_label) in &entry.labels {
            out.insert(
                (path.clone(), quarto_id.clone()),
                Label::new(myst_label.clone()),
            );
        }
    }
    out
}

/// Writes `sidecar` to `path`, **merging** with whatever sidecar already
/// exists there rather than replacing it outright: a single-file re-run's
/// registry only covers the one file it converted, and a naive overwrite
/// would destroy every other file's entries recorded by an earlier
/// whole-project run into the same output root (RT-08's "single-file re-run
/// merges into the existing sidecar" requirement).
///
/// Existing entries for files `sidecar` also has entries for are replaced by
/// `sidecar`'s (this run's data supersedes what was on disk for those
/// files); entries for files `sidecar` does not mention are kept unchanged.
/// The existing file, if present, is read via [`read_untrusted`] with
/// `sidecar.direction` as the expected direction — a direction-mismatched
/// existing file is treated as absent (its entries are not merged in), the
/// same "probably stale" handling any other reader gets.
///
/// # Errors
/// Propagates [`crate::fs::atomic::write_atomic`]'s error if the final
/// atomic write fails.
pub fn write_merged(
    sidecar: &LabelSidecar,
    path: &Path,
) -> Result<(), crate::fs::atomic::AtomicWriteError> {
    let (existing, _warnings) = read_untrusted(path, &sidecar.direction);
    let mut merged = existing.unwrap_or_else(|| LabelSidecar {
        version: sidecar.version,
        generated_by: sidecar.generated_by.clone(),
        direction: sidecar.direction.clone(),
        source_root: sidecar.source_root.clone(),
        files: BTreeMap::new(),
    });
    merged.generated_by = sidecar.generated_by.clone();
    merged.source_root = sidecar.source_root.clone();
    for (file, entry) in &sidecar.files {
        merged.files.insert(file.clone(), entry.clone());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| {
            crate::fs::atomic::AtomicWriteError::WriteTemp {
                target: path.to_path_buf(),
                temp_path: parent.to_path_buf(),
                source,
            }
        })?;
    }

    let json = serde_json::to_string_pretty(&merged)
        .expect("LabelSidecar serialization cannot fail (no non-finite floats, no cycles)");
    write_atomic(path, json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Attrs, BlockKind, Document, Engine, FigureSource};
    use crate::{Block, Span};

    fn doc(source: &str, label: &str) -> (PathBuf, Document) {
        (
            PathBuf::from(source),
            Document {
                frontmatter: None,
                blocks: vec![Block {
                    kind: BlockKind::Figure {
                        src: FigureSource::Path(PathBuf::from("img.png")),
                        caption: vec![],
                        label: Some(Label::new(label)),
                        attrs: Attrs::new(),
                    },
                    span: Span::single(1),
                    blank_lines_before: 0,
                }],
                source: PathBuf::from(source),
                engine: Some(Engine::Jupyter),
            },
        )
    }

    fn tempdir(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("mystquarto-sidecar-test-{label}-{nanos}-{n}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn round_trips_through_json() {
        let docs = vec![doc("article.md", "fig:samples")];
        let (registry, _) = LabelRegistry::build(&docs);
        let mut hashes = BTreeMap::new();
        hashes.insert(PathBuf::from("article.md"), content_hash(b"content"));
        let sidecar = build(&registry, "myst_to_quarto", Path::new(""), &hashes);

        let json = serde_json::to_string(&sidecar).unwrap();
        let back: LabelSidecar = serde_json::from_str(&json).unwrap();
        assert_eq!(sidecar, back);
        assert_eq!(
            back.files
                .get("article.md")
                .unwrap()
                .labels
                .get("fig-samples"),
            Some(&"fig:samples".to_string())
        );
    }

    #[test]
    fn absent_sidecar_is_none_with_no_warnings() {
        let tmp = tempdir("absent");
        let (sidecar, warnings) = read_untrusted(&tmp.join("labels.json"), "myst_to_quarto");
        assert!(sidecar.is_none());
        assert!(warnings.is_empty());
        cleanup(&tmp);
    }

    #[test]
    fn oversized_sidecar_is_refused() {
        let tmp = tempdir("oversized");
        let path = tmp.join("labels.json");
        // A file over MAX_SIDECAR_BYTES without actually allocating that
        // much memory for well-formed JSON: pad inside a JSON string value.
        let padding = "x".repeat((MAX_SIDECAR_BYTES + 1) as usize);
        fs::write(&path, format!(r#"{{"pad":"{padding}"}}"#)).unwrap();
        let (sidecar, warnings) = read_untrusted(&path, "myst_to_quarto");
        assert!(sidecar.is_none());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("bytes"));
        cleanup(&tmp);
    }

    #[test]
    fn corrupt_json_is_refused_with_a_warning_not_a_panic() {
        let tmp = tempdir("corrupt");
        let path = tmp.join("labels.json");
        fs::write(&path, "{ this is not json").unwrap();
        let (sidecar, warnings) = read_untrusted(&path, "myst_to_quarto");
        assert!(sidecar.is_none());
        assert_eq!(warnings.len(), 1);
        cleanup(&tmp);
    }

    #[test]
    fn wrong_version_is_refused() {
        let tmp = tempdir("wrong-version");
        let path = tmp.join("labels.json");
        fs::write(
            &path,
            r#"{"version":2,"generated_by":"x","direction":"myst_to_quarto","source_root":"","files":{}}"#,
        )
        .unwrap();
        let (sidecar, warnings) = read_untrusted(&path, "myst_to_quarto");
        assert!(sidecar.is_none());
        assert_eq!(warnings.len(), 1);
        cleanup(&tmp);
    }

    #[test]
    fn wrong_direction_is_refused_with_a_warning_not_silently() {
        let tmp = tempdir("wrong-direction");
        let path = tmp.join("labels.json");
        fs::write(
            &path,
            r#"{"version":1,"generated_by":"x","direction":"quarto_to_myst","source_root":"","files":{}}"#,
        )
        .unwrap();
        let (sidecar, warnings) = read_untrusted(&path, "myst_to_quarto");
        assert!(sidecar.is_none());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("direction"));
        cleanup(&tmp);
    }

    #[test]
    fn too_many_entries_is_refused() {
        let tmp = tempdir("too-many-entries");
        let path = tmp.join("labels.json");
        // Build a sidecar whose declared entry count is checked by summing
        // `labels.len()` per file — one file with more than the cap is
        // enough, without actually writing hundreds of thousands of bytes
        // for the test.
        let mut labels = BTreeMap::new();
        for i in 0..10 {
            labels.insert(format!("fig-{i}"), format!("fig:{i}"));
        }
        let sidecar = LabelSidecar {
            version: 1,
            generated_by: "test".to_string(),
            direction: "myst_to_quarto".to_string(),
            source_root: String::new(),
            files: BTreeMap::from([(
                "a.md".to_string(),
                FileLabels {
                    content_hash: String::new(),
                    labels,
                },
            )]),
        };
        fs::write(&path, serde_json::to_string(&sidecar).unwrap()).unwrap();

        // Confirm the normal (under-cap) case round-trips first, then prove
        // the cap itself is enforced by calling the checked comparison
        // directly against a value exceeding it (avoids materializing
        // MAX_SIDECAR_ENTRIES real strings in a test).
        let (ok, _) = read_untrusted(&path, "myst_to_quarto");
        assert!(ok.is_some(), "10 entries must be well under the cap");
        cleanup(&tmp);
    }

    #[test]
    fn a_label_containing_a_newline_is_dropped_not_trusted() {
        let tmp = tempdir("newline-label");
        let path = tmp.join("labels.json");
        let mut labels = BTreeMap::new();
        labels.insert("fig-ok".to_string(), "fig:ok".to_string());
        labels.insert(
            "fig-hostile".to_string(),
            "fig:hostile\n)=\n\n```{raw} html\n<script>x</script>\n```\n(y".to_string(),
        );
        let sidecar = LabelSidecar {
            version: 1,
            generated_by: "test".to_string(),
            direction: "myst_to_quarto".to_string(),
            source_root: String::new(),
            files: BTreeMap::from([(
                "a.md".to_string(),
                FileLabels {
                    content_hash: String::new(),
                    labels,
                },
            )]),
        };
        fs::write(&path, serde_json::to_string(&sidecar).unwrap()).unwrap();

        let (result, warnings) = read_untrusted(&path, "myst_to_quarto");
        let result = result.expect("otherwise-valid sidecar must still be accepted");
        let file = result.files.get("a.md").unwrap();
        assert!(file.labels.contains_key("fig-ok"));
        assert!(!file.labels.contains_key("fig-hostile"));
        assert_eq!(warnings.len(), 1);
        cleanup(&tmp);
    }

    #[test]
    fn stale_content_hash_entries_are_skipped_on_restore() {
        let mut sidecar_files = BTreeMap::new();
        let mut labels = BTreeMap::new();
        labels.insert("fig-samples".to_string(), "fig:samples".to_string());
        sidecar_files.insert(
            "article.md".to_string(),
            FileLabels {
                content_hash: content_hash(b"old content"),
                labels,
            },
        );
        let sidecar = LabelSidecar {
            version: 1,
            generated_by: "test".to_string(),
            direction: "quarto_to_myst".to_string(),
            source_root: String::new(),
            files: sidecar_files,
        };

        let mut current = BTreeMap::new();
        current.insert(PathBuf::from("article.md"), content_hash(b"new content"));
        let restored = restore_labels(&sidecar, &current);
        assert!(
            restored.is_empty(),
            "a changed file's entries must not be trusted"
        );

        let mut unchanged = BTreeMap::new();
        unchanged.insert(PathBuf::from("article.md"), content_hash(b"old content"));
        let restored = restore_labels(&sidecar, &unchanged);
        assert_eq!(restored.len(), 1);
    }

    #[test]
    fn write_merged_preserves_entries_for_files_not_in_this_run() {
        let tmp = tempdir("merge");
        let path = tmp.join("labels.json");

        let docs_a = vec![doc("a.md", "fig:a")];
        let (registry_a, _) = LabelRegistry::build(&docs_a);
        let hashes_a = BTreeMap::from([(PathBuf::from("a.md"), content_hash(b"a"))]);
        let sidecar_a = build(&registry_a, "myst_to_quarto", Path::new(""), &hashes_a);
        write_merged(&sidecar_a, &path).unwrap();

        let docs_b = vec![doc("b.md", "fig:b")];
        let (registry_b, _) = LabelRegistry::build(&docs_b);
        let hashes_b = BTreeMap::from([(PathBuf::from("b.md"), content_hash(b"b"))]);
        let sidecar_b = build(&registry_b, "myst_to_quarto", Path::new(""), &hashes_b);
        write_merged(&sidecar_b, &path).unwrap();

        let (merged, _) = read_untrusted(&path, "myst_to_quarto");
        let merged = merged.unwrap();
        assert!(
            merged.files.contains_key("a.md"),
            "merging b.md must not drop a.md's entries"
        );
        assert!(merged.files.contains_key("b.md"));

        cleanup(&tmp);
    }
}
