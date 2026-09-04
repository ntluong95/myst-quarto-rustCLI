use std::fs;
use std::path::{Path, PathBuf};

use mystquarto_core::pipeline;
use mystquarto_core::registry::sidecar;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoundTripClass {
    Stable,
    Normalized,
    Lossy,
}

impl RoundTripClass {
    fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "stable" => Some(Self::Stable),
            "normalized" => Some(Self::Normalized),
            "lossy" => Some(Self::Lossy),
            _ => None,
        }
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn parity_root() -> PathBuf {
    workspace_root().join("tests/corpus/parity")
}

fn read_fixture(path: &Path) -> (RoundTripClass, String) {
    let text = fs::read_to_string(path).unwrap();
    let mut lines = text.lines();
    let first = lines.next().unwrap();
    let value = first
        .strip_prefix("<!-- mystquarto-roundtrip: ")
        .and_then(|s| s.strip_suffix(" -->"))
        .unwrap_or_else(|| {
            panic!(
                "{} must declare `mystquarto-roundtrip` in its first line",
                path.display()
            )
        });
    let rest = if text.ends_with('\n') {
        format!("{}\n", lines.collect::<Vec<_>>().join("\n"))
    } else {
        lines.collect::<Vec<_>>().join("\n")
    };
    (
        RoundTripClass::parse(value)
            .unwrap_or_else(|| panic!("invalid round-trip class {value:?}")),
        rest,
    )
}

fn constructs_root() -> PathBuf {
    workspace_root().join("tests/corpus/constructs")
}

fn roundtrip_inputs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in [parity_root(), constructs_root()] {
        if !root.exists() {
            continue;
        }
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                for name in ["input.md", "input.qmd"] {
                    let input = path.join(name);
                    if input.exists() {
                        out.push(input);
                    }
                }
            }
        }
    }
    out.sort();
    out
}

#[test]
fn roundtrip_classes_are_honored() {
    let mut failures = Vec::new();
    for input in roundtrip_inputs() {
        let (class, source) = read_fixture(&input);
        let case_name = input
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        match input.extension().and_then(|e| e.to_str()) {
            Some("md") => check_myst_roundtrip(&case_name, class, &source, &mut failures),
            Some("qmd") => check_quarto_roundtrip(&case_name, class, &source, &mut failures),
            _ => {}
        }
    }

    assert!(
        failures.is_empty(),
        "round-trip class failure(s):\n{}",
        failures.join("\n")
    );
}

fn check_myst_roundtrip(
    case_name: &str,
    class: RoundTripClass,
    source: &str,
    failures: &mut Vec<String>,
) {
    let root = tempdir("myst-rt");
    let myst = root.join("input.md");
    fs::write(&myst, source).unwrap();
    let forward = pipeline::convert_myst_to_quarto_batch(std::slice::from_ref(&myst), &[], &root);
    let Some(quarto_text) = forward.rendered.get(&myst).cloned() else {
        failures.push(format!(
            "{case_name}: forward conversion produced no output"
        ));
        cleanup(&root);
        return;
    };

    let qroot = tempdir("quarto-rt");
    let qmd = qroot.join("input.qmd");
    fs::write(&qmd, &quarto_text).unwrap();
    let labels_path = qroot.join(".mystquarto").join("labels.json");
    sidecar::write_merged(&forward.sidecar, &labels_path).unwrap();
    let reverse = pipeline::convert_quarto_to_myst_batch(
        std::slice::from_ref(&qmd),
        &[],
        &qroot,
        Some(&labels_path),
    );
    let Some(back) = reverse.rendered.get(&qmd).cloned() else {
        failures.push(format!(
            "{case_name}: reverse conversion produced no output"
        ));
        cleanup(&root);
        cleanup(&qroot);
        return;
    };

    match class {
        RoundTripClass::Stable if back != source => {
            failures.push(format!("{case_name}: Stable case changed after round trip:\n--- source\n{source}\n+++ back\n{back}"));
        }
        RoundTripClass::Normalized => {
            fs::write(&myst, &back).unwrap();
            let again =
                pipeline::convert_myst_to_quarto_batch(std::slice::from_ref(&myst), &[], &root);
            if again.rendered.get(&myst) != Some(&quarto_text) {
                failures.push(format!("{case_name}: Normalized case did not settle"));
            }
        }
        RoundTripClass::Lossy if forward.warnings.is_empty() && reverse.warnings.is_empty() => {
            failures.push(format!("{case_name}: Lossy case emitted no diagnostic"));
        }
        _ => {}
    }

    cleanup(&root);
    cleanup(&qroot);
}

fn check_quarto_roundtrip(
    case_name: &str,
    class: RoundTripClass,
    source: &str,
    failures: &mut Vec<String>,
) {
    let root = tempdir("quarto-rt");
    let qmd = root.join("input.qmd");
    fs::write(&qmd, source).unwrap();
    let forward =
        pipeline::convert_quarto_to_myst_batch(std::slice::from_ref(&qmd), &[], &root, None);
    let Some(myst_text) = forward.rendered.get(&qmd).cloned() else {
        failures.push(format!(
            "{case_name}: forward conversion produced no output"
        ));
        cleanup(&root);
        return;
    };

    let mroot = tempdir("myst-rt");
    let md = mroot.join("input.md");
    fs::write(&md, &myst_text).unwrap();
    let reverse = pipeline::convert_myst_to_quarto_batch(std::slice::from_ref(&md), &[], &mroot);
    let Some(back) = reverse.rendered.get(&md).cloned() else {
        failures.push(format!(
            "{case_name}: reverse conversion produced no output"
        ));
        cleanup(&root);
        cleanup(&mroot);
        return;
    };

    match class {
        RoundTripClass::Stable if back != source => {
            failures.push(format!("{case_name}: Stable case changed after round trip"));
        }
        RoundTripClass::Normalized => {
            fs::write(&qmd, &back).unwrap();
            let again = pipeline::convert_quarto_to_myst_batch(
                std::slice::from_ref(&qmd),
                &[],
                &root,
                None,
            );
            if again.rendered.get(&qmd) != Some(&myst_text) {
                failures.push(format!("{case_name}: Normalized case did not settle"));
            }
        }
        RoundTripClass::Lossy if forward.warnings.is_empty() && reverse.warnings.is_empty() => {
            failures.push(format!("{case_name}: Lossy case emitted no diagnostic"));
        }
        _ => {}
    }

    cleanup(&root);
    cleanup(&mroot);
}

fn tempdir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mystquarto-roundtrip-test-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn cleanup(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}
