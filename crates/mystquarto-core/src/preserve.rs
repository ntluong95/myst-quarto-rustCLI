//! `.mystquarto/preserved.json`'s block-preservation section (reference §11,
//! decisions RD-2/RT-02/RT-11): the sidecar unmappable/preserved block
//! content lives in, so the document itself only ever holds a single-line,
//! content-free marker comment — never the original source, which could
//! otherwise be interpreted as markup by the renderer.
//!
//! **RT-02.** An earlier stopgap (`crate::writer::quarto`/`crate::writer::myst`'s
//! now-removed `preserved()` helper) wrapped the original source in a fenced
//! code block directly in the document — already injection-safe (a fenced
//! block's content is always literal text, unlike an HTML comment, which
//! Pandoc ends at the first blank line rather than at `-->`). This module
//! goes further, per the phase spec's architecture: the original source
//! moves out of the document entirely, into this JSON sidecar, so recovering
//! it after a round trip does not depend on the document's own fenced-block
//! content surviving untouched, and the document itself carries zero bytes
//! of unmappable/attacker-influenced source.
//!
//! Untrusted-input handling mirrors `crate::registry::sidecar` and
//! `crate::config::sidecar`: size-capped, version-checked, degrades to "no
//! sidecar" (or "no entry") rather than erroring on anything malformed —
//! see [`crate::diagnostics::codes::block::PRESERVATION_ENTRY_MISSING`] for
//! the diagnostic a missing/stale entry produces.
//!
//! Entries are keyed by a content hash of `original` (reusing
//! [`crate::registry::sidecar::content_hash`]) — the same content preserved
//! twice gets the same id, which is also why this is a plain
//! `BTreeMap<String, PreservedEntry>` rather than an insertion-ordered list:
//! id collisions on identical content are a merge, not a conflict.
//!
//! **One file, two independently-written sections.** [`crate::config::sidecar`]
//! (Phase 6) already writes `.mystquarto/preserved.json` for §8.2's
//! unmappable *config fields* (`fields`) — that module's own docs call this
//! file "the authoritative recovery channel" and note the two features were
//! expected to "eventually share the `.mystquarto/preserved.json` file."
//! [`PreservedSidecar`] is that shared schema (`fields` + `entries`); both
//! [`write_fields`] and [`write_entries`] read-merge-write so a config-file
//! conversion run and a content-block conversion run — which happen at
//! different points in the same `mystquarto` invocation, see
//! `mystquarto::orchestrate` — never clobber each other's section, and so
//! that either section survives being the only one written on a given run.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::fs::atomic::write_atomic;
use crate::registry::sidecar::content_hash;

/// Bytes above which the sidecar is refused outright — covers both this
/// module's block content and [`crate::config::sidecar`]'s config fields,
/// since both now share one file.
pub const MAX_SIDECAR_BYTES: u64 = 8 * 1024 * 1024;

/// Which dialect `PreservedEntry::original` is written in — the fix for a
/// real cross-dialect content-mangling defect (RT-02-class): the reader
/// that captured `original` couldn't express it (that's why it was
/// preserved), so reparsing it through the *other* dialect's parser can
/// "succeed" by misclassifying it as some other, wrong construct — a
/// backtick-fenced MyST directive like `` ```{glossary} `` reparses cleanly
/// as a Quarto executable code cell, silently changing meaning and, if the
/// body itself contains a fence, potentially terminating early and letting
/// trailing lines escape as literal document content. Recording the
/// dialect lets a reader refuse to reparse foreign content at all — see
/// `crate::reader::myst::MystReader::push_preserved_or_marker`'s doc.
/// `#[serde(default)]` so an entry from a prior schema version (no field at
/// all) deserializes to `Unknown`, which a reader treats as foreign to
/// every dialect — the safe direction to fail in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Dialect {
    #[default]
    Unknown,
    Myst,
    Quarto,
}

/// One preserved construct's original source, plus enough context to make
/// `preserved.json` readable directly by a human, not only by this crate.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PreservedEntry {
    pub file: String,
    pub line: u32,
    pub code: String,
    pub kind: String,
    #[serde(default)]
    pub dialect: Dialect,
    pub original: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct PreservedSidecar {
    pub version: u32,
    /// §8.2's unmappable `myst.yml` config fields, owned by
    /// [`crate::config::sidecar`] — present here so this module's
    /// [`write_entries`] can preserve it across a content-only write.
    #[serde(default)]
    pub fields: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub entries: BTreeMap<String, PreservedEntry>,
}

/// Computes the sidecar id for `original` — a short, stable, content-derived
/// key (not a running counter), so the same content preserved in two
/// separate runs gets the same id.
#[must_use]
pub fn entry_id(original: &[String]) -> String {
    content_hash(original.join("\n").as_bytes())
}

/// Renders the single-line, content-free marker left in the document at a
/// preserved construct's original location — recognized on read by
/// `crate::reader::preservation_marker_id`, which this function's output is
/// paired with in tests. `code` is always a compile-time `&'static str`
/// (never attacker-influenced); `kind` is a short human-readable label that
/// may derive from source (a directive name) — `-->`/`--!>` sequences in it
/// are neutralized so it cannot end the comment early, even though (unlike
/// the old fenced-block stopgap) no user-controlled *content* is in the
/// document at all for that to expose.
#[must_use]
pub fn marker(code: &str, kind: &str, id: &str) -> String {
    // Three neutralizations, all defending against `kind`, which (via
    // `crate::writer::preserved_kind`) can carry source-derived text:
    // `-->`/`--!>` could end the comment early; a literal newline would
    // break the single-line invariant `preservation_marker_id` and every
    // caller of this function rely on; and — the actual hole a prior
    // version of this function had — the id-lookup needle itself
    // (`.mystquarto/preserved.json#`) could appear *inside* `kind`, and
    // since the old reader resolved the id from the *first* occurrence,
    // an attacker-chosen `kind` could redirect the lookup to any other
    // entry in the sidecar, substituting one preserved block's content for
    // another's on the next reverse conversion. Neutralizing it here closes
    // the hole at the source; `preservation_marker_id` additionally
    // resolves from the *last* occurrence as defense in depth.
    let safe_kind = kind
        .replace("-->", "- >")
        .replace("--!>", "- !>")
        .replace(['\n', '\r'], " ")
        .replace(".mystquarto/preserved.json#", ".mystquarto/preserved_json#");
    format!(
        "<!-- mystquarto {code}: {safe_kind} preserved — see .mystquarto/preserved.json#{id} -->"
    )
}

/// Reads the block-preservation sidecar at `path`, as untrusted input (same
/// threat model as `crate::registry::sidecar`/`crate::config::sidecar`: it
/// sits inside the tree being converted). Any problem — missing file,
/// oversized, malformed JSON, unexpected version — degrades to "no
/// sidecar," never a hard error.
#[must_use]
pub fn read(path: &Path) -> Option<PreservedSidecar> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > MAX_SIDECAR_BYTES {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let parsed: PreservedSidecar = serde_json::from_str(&text).ok()?;
    (parsed.version == 1).then_some(parsed)
}

fn write_sidecar(
    sidecar: &PreservedSidecar,
    path: &Path,
) -> Result<(), crate::fs::atomic::AtomicWriteError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| {
            crate::fs::atomic::AtomicWriteError::WriteTemp {
                target: path.to_path_buf(),
                temp_path: parent.to_path_buf(),
                source,
            }
        })?;
    }
    let json = serde_json::to_string_pretty(sidecar)
        .expect("PreservedSidecar serialization cannot fail (no non-finite floats, no cycles)");
    write_atomic(path, json.as_bytes())
}

/// Writes `entries` (this module's own section) to `path`, preserving
/// whatever `fields` [`crate::config::sidecar`] already wrote there — see
/// module docs. A read failure (missing/malformed/oversized file) is
/// treated as "no existing `fields` to preserve," not an error, matching
/// every other sidecar in this crate's untrusted-input handling.
///
/// # Errors
/// Propagates [`write_atomic`]'s error if the write fails.
pub fn write_entries(
    entries: &BTreeMap<String, PreservedEntry>,
    path: &Path,
) -> Result<(), crate::fs::atomic::AtomicWriteError> {
    let fields = read(path).map(|s| s.fields).unwrap_or_default();
    write_sidecar(
        &PreservedSidecar {
            version: 1,
            fields,
            entries: entries.clone(),
        },
        path,
    )
}

/// Writes `fields` (§8.2's config-field section, owned by
/// [`crate::config::sidecar`]) to `path`, preserving whatever `entries`
/// this module already wrote there — see module docs.
///
/// # Errors
/// Propagates [`write_atomic`]'s error if the write fails.
pub fn write_fields(
    fields: &BTreeMap<String, serde_json::Value>,
    path: &Path,
) -> Result<(), crate::fs::atomic::AtomicWriteError> {
    let entries = read(path).map(|s| s.entries).unwrap_or_default();
    write_sidecar(
        &PreservedSidecar {
            version: 1,
            fields: fields.clone(),
            entries,
        },
        path,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("mystquarto-preserve-test-{label}-{n}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn marker_is_recognized_by_the_reader_side_parser() {
        let id = entry_id(&["```{glossary}".to_string(), "term".to_string()]);
        let line = marker("MQ0201", "glossary", &id);
        assert_eq!(
            crate::reader::preservation_marker_id(&line),
            Some(id.as_str())
        );
    }

    #[test]
    fn marker_neutralizes_an_embedded_comment_terminator_in_kind() {
        let line = marker("MQ0201", "weird--> <script>alert(1)</script>", "abc123");
        // Exactly one `-->` — the real, final terminator — not an earlier
        // one an attacker-influenced `kind` could have smuggled in.
        assert_eq!(line.matches("-->").count(), 1, "got: {line}");
        assert!(line.trim_end().ends_with("-->"));
    }

    #[test]
    fn same_content_produces_the_same_id_across_calls() {
        let a = entry_id(&["x".to_string(), "y".to_string()]);
        let b = entry_id(&["x".to_string(), "y".to_string()]);
        assert_eq!(a, b);
    }

    #[test]
    fn round_trips_an_entry() {
        let tmp = tempdir("round-trip");
        let path = tmp.join(".mystquarto").join("preserved.json");
        let mut entries = BTreeMap::new();
        let id = entry_id(&["```{glossary}".to_string()]);
        entries.insert(
            id.clone(),
            PreservedEntry {
                file: "article.md".to_string(),
                line: 88,
                code: "MQ0201".to_string(),
                kind: "glossary".to_string(),
                dialect: Dialect::Myst,
                original: vec!["```{glossary}".to_string(), "term".to_string()],
            },
        );
        write_entries(&entries, &path).unwrap();
        let back = read(&path).expect("just-written sidecar must read back");
        assert_eq!(back.entries.get(&id).unwrap().kind, "glossary");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn write_entries_preserves_fields_written_by_config_sidecar() {
        let tmp = tempdir("merge-fields-then-entries");
        let path = tmp.join("preserved.json");
        let mut fields = BTreeMap::new();
        fields.insert(
            "venue".to_string(),
            serde_json::json!("The Morganton Scientific"),
        );
        write_fields(&fields, &path).unwrap();

        let mut entries = BTreeMap::new();
        let id = entry_id(&["term".to_string()]);
        entries.insert(
            id.clone(),
            PreservedEntry {
                file: "a.md".to_string(),
                line: 1,
                code: "MQ0201".to_string(),
                kind: "glossary".to_string(),
                dialect: Dialect::Myst,
                original: vec!["term".to_string()],
            },
        );
        write_entries(&entries, &path).unwrap();

        let back = read(&path).expect("sidecar must read back");
        assert_eq!(
            back.fields.get("venue"),
            Some(&serde_json::json!("The Morganton Scientific"))
        );
        assert!(back.entries.contains_key(&id));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn missing_sidecar_is_none_not_an_error() {
        let tmp = tempdir("missing");
        assert!(read(&tmp.join("nope.json")).is_none());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn corrupt_json_is_none_not_a_panic() {
        let tmp = tempdir("corrupt");
        let path = tmp.join("preserved.json");
        fs::write(&path, "{ not json").unwrap();
        assert!(read(&path).is_none());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn wrong_version_is_none() {
        let tmp = tempdir("wrong-version");
        let path = tmp.join("preserved.json");
        fs::write(&path, r#"{"version":2,"entries":{}}"#).unwrap();
        assert!(read(&path).is_none());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn oversized_sidecar_is_refused() {
        let tmp = tempdir("oversized");
        let path = tmp.join("preserved.json");
        let padding = "x".repeat((MAX_SIDECAR_BYTES + 1) as usize);
        fs::write(&path, format!(r#"{{"pad":"{padding}"}}"#)).unwrap();
        assert!(read(&path).is_none());
        let _ = fs::remove_dir_all(&tmp);
    }
}
