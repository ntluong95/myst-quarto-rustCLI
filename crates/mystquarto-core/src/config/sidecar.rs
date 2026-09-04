//! `.mystquarto/preserved.json`'s config-field section (reference §8.2,
//! RT-02, RT-11): the **authoritative** recovery channel for a §8.2
//! unmappable field (`abbreviations`, `open_access`, `venue`, `id`) — the
//! informational `# ` comment [`super::myst_to_quarto`] writes above it in
//! `_quarto.yml` is for a human reader, not for round-trip. Splitting the
//! two means a hand-edited or deleted comment is harmless: recovery depends
//! only on this structured file.
//!
//! Deliberately not the same struct as [`crate::registry::sidecar::LabelSidecar`]:
//! that sidecar's per-file, content-hash-gated shape solves a per-document
//! staleness problem this one does not have (a project has exactly one
//! `myst.yml`/`_quarto.yml`) — reusing its shape here would carry machinery
//! (per-file keys, content hashing) this narrower problem does not need.
//!
//! This module owns the `fields` section of `.mystquarto/preserved.json`;
//! [`crate::preserve`] owns the `entries` section (block-level preservation)
//! and the file's actual read/write/merge logic — see that module's docs.
//! [`read`]/[`write`] here are thin, config-domain-typed wrappers over it.

use std::path::Path;

use crate::preserve::PreservedSidecar;
use crate::yaml::YamlValue;

#[cfg(test)]
use std::fs;

/// Converts a config-field [`YamlValue`] to JSON for storage. Handles every
/// variant (not just the ones §8.2's fields currently use) so a future field
/// added to the unmappable set does not need this function revisited.
#[must_use]
pub fn yaml_to_json(v: &YamlValue) -> serde_json::Value {
    match v {
        YamlValue::String(s) | YamlValue::BlockLiteral(s) => serde_json::Value::String(s.clone()),
        YamlValue::Int(i) => serde_json::Value::from(*i),
        YamlValue::Float(f) => serde_json::json!(*f),
        YamlValue::Bool(b) => serde_json::Value::Bool(*b),
        YamlValue::Null => serde_json::Value::Null,
        YamlValue::Sequence(items) => {
            serde_json::Value::Array(items.iter().map(yaml_to_json).collect())
        }
        YamlValue::Mapping(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), yaml_to_json(v)))
                .collect(),
        ),
    }
}

/// The inverse of [`yaml_to_json`]. A JSON number without an exact `i64`
/// representation becomes [`YamlValue::Float`].
#[must_use]
pub fn json_to_yaml(v: &serde_json::Value) -> YamlValue {
    match v {
        serde_json::Value::String(s) => YamlValue::String(s.clone()),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(YamlValue::Int)
            .unwrap_or_else(|| YamlValue::Float(n.as_f64().unwrap_or(0.0))),
        serde_json::Value::Bool(b) => YamlValue::Bool(*b),
        serde_json::Value::Null => YamlValue::Null,
        serde_json::Value::Array(items) => {
            YamlValue::Sequence(items.iter().map(json_to_yaml).collect())
        }
        serde_json::Value::Object(map) => YamlValue::Mapping(
            map.iter()
                .map(|(k, v)| (k.clone(), json_to_yaml(v)))
                .collect(),
        ),
    }
}

/// Reads the config-field preservation sidecar at `path`, as untrusted input
/// (RT-09: it sits inside the tree being converted). Any problem — missing
/// file, oversized, malformed JSON, unexpected version — is treated as "no
/// sidecar," never a hard error: a missing or hand-edited sidecar must not
/// block a reverse conversion, only degrade it to "unmappable fields are not
/// restored." Delegates to [`crate::preserve::read`], which owns the shared
/// file — see module docs.
#[must_use]
pub fn read(path: &Path) -> Option<PreservedSidecar> {
    crate::preserve::read(path)
}

/// Writes `fields` to `path`'s `fields` section, preserving whatever
/// `entries` [`crate::preserve`] already wrote there.
///
/// # Errors
/// Propagates [`crate::fs::atomic::write_atomic`]'s error if the write
/// fails.
pub fn write(
    fields: &std::collections::BTreeMap<String, YamlValue>,
    path: &Path,
) -> Result<(), crate::fs::atomic::AtomicWriteError> {
    let json_fields = fields
        .iter()
        .map(|(k, v)| (k.clone(), yaml_to_json(v)))
        .collect();
    crate::preserve::write_fields(&json_fields, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn tempdir(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("mystquarto-config-sidecar-test-{label}-{n}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trips_bool_string_and_mapping_fields() {
        let tmp = tempdir("round-trip");
        let path = tmp.join(".mystquarto").join("preserved.json");

        let mut fields = BTreeMap::new();
        fields.insert("open_access".to_string(), YamlValue::Bool(true));
        fields.insert(
            "venue".to_string(),
            YamlValue::String("The Morganton Scientific".to_string()),
        );
        fields.insert(
            "abbreviations".to_string(),
            YamlValue::Mapping(vec![(
                "CRISPR".to_string(),
                YamlValue::String(
                    "Clustered regularly interspaced short palindromic repeats".to_string(),
                ),
            )]),
        );
        write(&fields, &path).unwrap();

        let read_back = read(&path).expect("just-written sidecar must read back");
        assert_eq!(
            read_back.fields.get("open_access"),
            Some(&serde_json::Value::Bool(true))
        );
        assert_eq!(
            json_to_yaml(read_back.fields.get("venue").unwrap()),
            YamlValue::String("The Morganton Scientific".to_string())
        );
        let YamlValue::Mapping(abbrevs) =
            json_to_yaml(read_back.fields.get("abbreviations").unwrap())
        else {
            panic!("expected a mapping");
        };
        assert_eq!(abbrevs[0].0, "CRISPR");

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
        fs::write(&path, r#"{"version":2,"fields":{}}"#).unwrap();
        assert!(read(&path).is_none());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn oversized_sidecar_is_refused() {
        let tmp = tempdir("oversized");
        let path = tmp.join("preserved.json");
        let padding = "x".repeat((crate::preserve::MAX_SIDECAR_BYTES + 1) as usize);
        fs::write(&path, format!(r#"{{"pad":"{padding}"}}"#)).unwrap();
        assert!(read(&path).is_none());
        let _ = fs::remove_dir_all(&tmp);
    }
}
