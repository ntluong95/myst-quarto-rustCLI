use std::fs;
use std::path::{Path, PathBuf};

use similar::TextDiff;

use mystquarto_core::config;
use mystquarto_core::pipeline;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    MystToQuarto,
    QuartoToMyst,
}

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

#[derive(Debug)]
struct CorpusCase {
    path: PathBuf,
    input: PathBuf,
    expected: Option<PathBuf>,
    python_actual: Option<PathBuf>,
    direction: Direction,
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn corpus_root() -> PathBuf {
    workspace_root().join("tests/corpus")
}

fn discover_cases() -> Vec<CorpusCase> {
    fn walk(dir: &Path, out: &mut Vec<CorpusCase>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if let Some(case) = case_at(&path) {
                    out.push(case);
                } else {
                    walk(&path, out);
                }
            }
        }
    }

    let mut out = Vec::new();
    walk(&corpus_root(), &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn case_at(path: &Path) -> Option<CorpusCase> {
    let input = ["input.md", "input.qmd", "input.yml"]
        .into_iter()
        .map(|name| path.join(name))
        .find(|p| p.exists())?;
    let direction = if input.extension().and_then(|e| e.to_str()) == Some("qmd") {
        Direction::QuartoToMyst
    } else if path.join("direction").exists() {
        match fs::read_to_string(path.join("direction")).unwrap().trim() {
            "myst_to_quarto" => Direction::MystToQuarto,
            "quarto_to_myst" => Direction::QuartoToMyst,
            other => panic!("{} has invalid direction {other:?}", path.display()),
        }
    } else {
        Direction::MystToQuarto
    };

    let expected = ["expected.md", "expected.qmd", "expected.yml"]
        .into_iter()
        .map(|name| path.join(name))
        .find(|p| p.exists());
    let python_actual = ["python-actual.md", "python-actual.qmd", "python-actual.yml"]
        .into_iter()
        .map(|name| path.join(name))
        .find(|p| p.exists());

    Some(CorpusCase {
        path: path.to_path_buf(),
        input,
        expected,
        python_actual,
        direction,
    })
}

fn read_fixture(path: &Path) -> (RoundTripClass, String) {
    let text = fs::read_to_string(path).unwrap();
    let mut lines = text.lines();
    let first = lines
        .next()
        .unwrap_or_else(|| panic!("{} is empty", path.display()));
    let Some(value) = first
        .strip_prefix("<!-- mystquarto-roundtrip: ")
        .and_then(|s| s.strip_suffix(" -->"))
        .or_else(|| {
            first
                .strip_prefix("# mystquarto-roundtrip: ")
                .map(str::trim)
        })
    else {
        panic!(
            "{} must declare `mystquarto-roundtrip` in its first line",
            path.display()
        );
    };
    let class = RoundTripClass::parse(value)
        .unwrap_or_else(|| panic!("{} has invalid round-trip class {value:?}", path.display()));
    let rest = if text.ends_with('\n') {
        format!("{}\n", lines.collect::<Vec<_>>().join("\n"))
    } else {
        lines.collect::<Vec<_>>().join("\n")
    };
    (class, rest)
}

fn diff(expected: &str, actual: &str) -> String {
    TextDiff::from_lines(expected, actual)
        .unified_diff()
        .context_radius(3)
        .header("expected", "actual")
        .to_string()
}

fn render_case(case: &CorpusCase, source: &str) -> (String, usize) {
    match (
        case.direction,
        case.input.extension().and_then(|e| e.to_str()),
    ) {
        (Direction::MystToQuarto, Some("md")) => {
            let tmp = tempdir("myst");
            let input = tmp.join("input.md");
            fs::write(&input, source).unwrap();
            let notebooks = if case
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("d11-"))
            {
                let notebook = tmp.join("analysis.ipynb");
                fs::write(
                    &notebook,
                    r##"{"cells":[{"cell_type":"code","source":["#| label: nb:analysis\n","1"],"outputs":[]}]}"##,
                )
                .unwrap();
                vec![notebook]
            } else {
                Vec::new()
            };
            let result = pipeline::convert_myst_to_quarto_batch(
                std::slice::from_ref(&input),
                &notebooks,
                &tmp,
            );
            cleanup(&tmp);
            (
                result.rendered.get(&input).cloned().unwrap_or_default(),
                result.warnings.len(),
            )
        }
        (Direction::QuartoToMyst, Some("qmd")) => {
            let tmp = tempdir("quarto");
            let input = tmp.join("input.qmd");
            fs::write(&input, source).unwrap();
            let result = pipeline::convert_quarto_to_myst_batch(
                std::slice::from_ref(&input),
                &[],
                &tmp,
                None,
            );
            cleanup(&tmp);
            (
                result.rendered.get(&input).cloned().unwrap_or_default(),
                result.warnings.len(),
            )
        }
        (Direction::MystToQuarto, Some("yml")) => {
            let result = config::myst_to_quarto::convert(source, None).unwrap();
            (result.text, result.warnings.len())
        }
        (Direction::QuartoToMyst, Some("yml")) => {
            let result = config::quarto_to_myst::convert(source, None).unwrap();
            (result.text, result.warnings.len())
        }
        _ => panic!("unsupported corpus case {}", case.path.display()),
    }
}

#[test]
fn text_corpus_cases_match_expected_outputs() {
    let cases: Vec<CorpusCase> = discover_cases()
        .into_iter()
        .filter(|case| case.expected.is_some())
        .collect();
    assert!(!cases.is_empty(), "no text corpus cases discovered");

    let mut failures = Vec::new();
    for case in cases {
        let (_class, source) = read_fixture(&case.input);
        let expected_path = case.expected.as_ref().unwrap();
        let expected = fs::read_to_string(expected_path).unwrap();
        let (actual, _warnings) = render_case(&case, &source);
        if actual != expected {
            failures.push(format!(
                "{}\n{}",
                case.path.strip_prefix(corpus_root()).unwrap().display(),
                diff(&expected, &actual)
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "corpus expectation mismatch(es):\n\n{}",
        failures.join("\n")
    );
}

#[test]
fn defect_expected_outputs_differ_from_recorded_python_actuals() {
    let mut missing_or_equal = Vec::new();
    for case in discover_cases()
        .into_iter()
        .filter(|case| case.path.components().any(|c| c.as_os_str() == "defects"))
        .filter(|case| case.expected.is_some())
    {
        let Some(python_actual) = &case.python_actual else {
            missing_or_equal.push(format!(
                "{}: missing python-actual fixture",
                case.path.strip_prefix(corpus_root()).unwrap().display()
            ));
            continue;
        };
        let expected = fs::read_to_string(case.expected.as_ref().unwrap()).unwrap();
        let actual = fs::read_to_string(python_actual).unwrap();
        if expected == actual {
            missing_or_equal.push(format!(
                "{}: expected output still equals recorded Python output",
                case.path.strip_prefix(corpus_root()).unwrap().display()
            ));
        }
    }

    assert!(
        missing_or_equal.is_empty(),
        "defect corpus must prove the Rust expectation fixes Python behavior:\n{}",
        missing_or_equal.join("\n")
    );
}

#[test]
fn lossy_corpus_cases_emit_at_least_one_diagnostic() {
    let mut silent = Vec::new();
    for case in discover_cases() {
        if case.expected.is_none() {
            continue;
        }
        let (class, source) = read_fixture(&case.input);
        if class != RoundTripClass::Lossy {
            continue;
        }
        let (_actual, warnings) = render_case(&case, &source);
        if warnings == 0 {
            silent.push(
                case.path
                    .strip_prefix(corpus_root())
                    .unwrap()
                    .display()
                    .to_string(),
            );
        }
    }

    assert!(
        silent.is_empty(),
        "lossy corpus cases emitted no diagnostics:\n{}",
        silent.join("\n")
    );
}

fn tempdir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mystquarto-corpus-test-{label}-{}-{}",
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
