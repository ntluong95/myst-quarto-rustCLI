#![cfg(feature = "renderer-tests")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn tempdir(label: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "mystquarto-renderer-test-{label}-{}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn cleanup(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

fn tree_snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                out.push((
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(&path).unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn run(mut cmd: Command, label: &str) -> String {
    let output = cmd
        .output()
        .unwrap_or_else(|e| panic!("{label} failed: {e}"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{label} exited with {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
    format!("{stdout}{stderr}")
}

#[test]
fn converted_article_template_renders_and_builds_with_resolved_references() {
    let tmp = tempdir("article");
    let quarto_dir = tmp.join("quarto");
    let myst_dir = tmp.join("myst");
    let root = workspace_root();

    let mut convert = Command::new(env!("CARGO_BIN_EXE_myst2quarto"));
    convert
        .arg(root.join("article-template"))
        .arg("-o")
        .arg(&quarto_dir)
        .arg("--strict");
    run(convert, "myst2quarto --strict article-template");

    let mut strict_all = Command::new(env!("CARGO_BIN_EXE_myst2quarto"));
    strict_all
        .arg(root.join("article-template"))
        .arg("-o")
        .arg(tmp.join("quarto-strict-all"))
        .arg("--strict=all");
    assert!(
        !strict_all.output().unwrap().status.success(),
        "--strict=all must fail on expected-lossy preservation"
    );

    let mut quarto = Command::new("quarto");
    quarto
        .current_dir(&quarto_dir)
        .arg("render")
        .arg("article.qmd")
        .arg("--to")
        .arg("html")
        .arg("--no-execute");
    let quarto_log = run(quarto, "quarto render");
    let quarto_log_path = tmp.join("quarto-render.log");
    fs::write(&quarto_log_path, quarto_log).unwrap();
    assert!(
        quarto_dir.join("_manuscript/index.html").exists(),
        "quarto render must produce manuscript HTML"
    );

    let mut refs = Command::new(root.join("scripts/check-refs.sh"));
    refs.arg(quarto_dir.join("_manuscript"))
        .arg(&quarto_log_path)
        .arg("10.1038/nmeth.1974")
        .arg("10.1038/nprot.2013.143");
    run(refs, "check-refs.sh");

    let mut reverse = Command::new(env!("CARGO_BIN_EXE_quarto2myst"));
    reverse.arg(&quarto_dir).arg("-o").arg(&myst_dir);
    run(reverse, "quarto2myst converted article-template");

    // RT-14 / Hermetic CI: Seed the offline CSL-JSON cache so myst build does not make live network calls
    let cache_src = root.join("tests/fixtures/csl_cache");
    let cache_dst = myst_dir.join("_build/cache");
    if cache_src.exists() {
        fs::create_dir_all(&cache_dst).unwrap();
        for entry in fs::read_dir(&cache_src).unwrap().flatten() {
            let _ = fs::copy(entry.path(), cache_dst.join(entry.file_name()));
        }
    }

    let mut myst = Command::new("myst");
    myst.current_dir(&myst_dir)
        .arg("build")
        .arg("article.md")
        .arg("--md")
        .arg("--force");
    let myst_log = run(myst, "myst build");
    assert!(
        !myst_log.contains("Unable to resolve")
            && !myst_log.contains("not found")
            && !myst_log.contains("Could not link citation")
            && !myst_log.contains("unexpected option"),
        "myst build reported unresolved references or options:\n{myst_log}"
    );

    cleanup(&tmp);
}

#[test]
fn converting_article_template_twice_is_byte_identical_without_nesting() {
    let tmp = tempdir("idempotent");
    let quarto_dir = tmp.join("quarto");
    let source = workspace_root().join("article-template");

    let mut first = Command::new(env!("CARGO_BIN_EXE_myst2quarto"));
    first.arg(&source).arg("-o").arg(&quarto_dir);
    run(first, "first article-template conversion");
    let before = tree_snapshot(&quarto_dir);

    let mut second = Command::new(env!("CARGO_BIN_EXE_myst2quarto"));
    second
        .arg(&source)
        .arg("-o")
        .arg(&quarto_dir)
        .arg("--force");
    run(second, "second article-template conversion");
    let after = tree_snapshot(&quarto_dir);

    assert_eq!(before, after, "second conversion changed output bytes");
    assert!(
        !quarto_dir.join("quarto").exists(),
        "second conversion must not nest the output inside itself"
    );

    cleanup(&tmp);
}
