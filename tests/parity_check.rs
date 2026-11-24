//! Parity checker: replays committed fixtures against THIS port's binary.
//!
//! Each `tests/parity/cases/*.json` records an invocation (args, stdin, files, env)
//! plus the behavior captured from the ORIGINAL binary (`expected`). This test runs
//! the same invocation against the Rust binary and diffs the observable behavior.
//!
//! CI runs this without the original binary or the `upstream/` checkout: the fixtures
//! are committed (see `capture.py`, which is only needed to (re)generate `expected`).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;
use tempfile::TempDir;

/// Absolute path to the built `bibtex-tidy` binary (provided by cargo for integration tests).
const BIN: &str = env!("CARGO_BIN_EXE_bibtex-tidy");

fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/parity/cases")
}

/// Result of running one invocation: normalized stdout/stderr, exit code, and the
/// resulting file tree (relative path -> contents).
struct Observation {
    stdout: String,
    stderr: String,
    exit: i32,
    files: BTreeMap<String, String>,
}

fn mask(text: &str, tmp: &Path) -> String {
    // Mask the per-run temp directory so comparisons are machine-independent.
    let dir = tmp.to_string_lossy();
    text.replace(&*dir, "<TMP>")
}

fn read_file_tree(root: &Path) -> std::io::Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                let bytes = fs::read(&path)?;
                files.insert(rel, String::from_utf8_lossy(&bytes).to_string());
            }
        }
    }
    Ok(files)
}

fn observe(case: &Value, tmp: &Path) -> Observation {
    // Materialize input files.
    if let Some(files) = case["files"].as_object() {
        for (rel, content) in files {
            let target = tmp.join(rel);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).expect("create input dir");
            }
            fs::write(&target, content.as_str().unwrap_or("")).expect("write input file");
        }
    }

    let args: Vec<String> = case["args"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Hermetic env: drop ambient PRE_COMMIT, then apply the case's explicit env.
    let mut cmd = Command::new(BIN);
    cmd.args(&args)
        .current_dir(tmp)
        .env_remove("PRE_COMMIT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(env) = case["env"].as_object() {
        for (k, v) in env {
            cmd.env(k, v.as_str().unwrap_or(""));
        }
    }

    let mut child = cmd.spawn().expect("spawn binary");
    let stdin = case["stdin"].as_str().unwrap_or("");
    {
        let mut stdin_handle = child.stdin.take().expect("stdin");
        // The binary may exit without reading stdin (e.g. --version/--help or an error
        // path); a broken pipe there is expected, so ignore write errors.
        let _ = stdin_handle.write_all(stdin.as_bytes());
    } // closing stdin signals EOF for cases that do read it

    let out = child.wait_with_output().expect("wait for binary");
    Observation {
        stdout: mask(&String::from_utf8_lossy(&out.stdout), tmp),
        stderr: mask(&String::from_utf8_lossy(&out.stderr), tmp),
        exit: out.status.code().unwrap_or(-1),
        files: read_file_tree(tmp).expect("read result files"),
    }
}

/// Compare one dimension; returns a description of the mismatch, if any.
fn diff(label: &str, expected: &str, actual: &str) -> Option<String> {
    if expected == actual {
        None
    } else {
        Some(format!(
            "  {label} mismatch:\n    expected: {expected:?}\n    actual:   {actual:?}"
        ))
    }
}

fn check_case(case: &Value) -> Result<(), String> {
    let name = case["name"].as_str().unwrap_or("<unnamed>");
    let expected = match case.get("expected") {
        Some(e) if !e.is_null() => e,
        _ => {
            return Err(format!(
                "{name}: fixture has no captured `expected` (run capture.py)"
            ));
        }
    };
    let policy = case["match"].as_str().unwrap_or("full");

    let tmp = TempDir::new().expect("create temp dir");
    let actual = observe(case, tmp.path());

    let mut problems: Vec<String> = Vec::new();

    // Exit code is always compared.
    let want_exit = expected["exit"].as_i64().unwrap_or(i64::from(actual.exit));
    if want_exit != i64::from(actual.exit) {
        problems.push(format!(
            "  exit mismatch:\n    expected: {want_exit}\n    actual:   {}",
            actual.exit
        ));
    }

    let compare_stdout = matches!(policy, "full" | "exit_stdout_files");
    let compare_files = matches!(policy, "full" | "exit_stdout_files");
    let compare_stderr = matches!(policy, "full");

    if compare_stdout {
        let want = expected["stdout"].as_str().unwrap_or("");
        if let Some(p) = diff("stdout", want, &actual.stdout) {
            problems.push(p);
        }
    }
    if compare_stderr {
        let want = expected["stderr"].as_str().unwrap_or("");
        if let Some(p) = diff("stderr", want, &actual.stderr) {
            problems.push(p);
        }
    }
    if compare_files {
        let want_files = expected["files"].as_object();
        let want: BTreeMap<String, String> = want_files
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        if want != actual.files {
            let want_keys: Vec<&String> = want.keys().collect();
            let actual_keys: Vec<&String> = actual.files.keys().collect();
            problems.push(format!(
                "  files mismatch:\n    expected keys: {want_keys:?}\n    actual keys:   {actual_keys:?}"
            ));
            for (k, wv) in &want {
                match actual.files.get(k) {
                    Some(av) if av != wv => {
                        problems.push(format!("    file {k} content differs:\n      expected: {wv:?}\n      actual:   {av:?}"))
                    }
                    Some(_) => {}
                    None => problems.push(format!("    file {k} missing in actual")),
                }
            }
            for k in actual.files.keys() {
                if !want.contains_key(k) {
                    problems.push(format!("    unexpected file {k} in actual"));
                }
            }
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} failed (match={policy}):\n{}",
            name,
            problems.join("\n")
        ))
    }
}

/// G-PARITY gate. Ignored by default so `cargo test` (G-TEST) stays green while the
/// port is still being built out slice-by-slice (INV-MAIN). Run it explicitly at slice
/// boundaries and in CI with:
///     cargo test --test parity_check -- --ignored
/// The full corpus is expected green by S4 (hardening).
#[test]
#[ignore = "G-PARITY: run with `cargo test --test parity_check -- --ignored`; green by S4"]
fn parity_fixtures_match_original_behavior() {
    let dir = cases_dir();
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read cases dir {dir:?}: {e}"))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();

    assert!(!paths.is_empty(), "no parity cases found in {dir:?}");

    let mut failures: Vec<String> = Vec::new();
    for path in &paths {
        let raw = fs::read_to_string(path).expect("read case");
        let case: Value = serde_json::from_str(&raw).expect("parse case json");
        if let Err(msg) = check_case(&case) {
            failures.push(msg);
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} parity cases failed:\n{}",
        failures.len(),
        paths.len(),
        failures.join("\n\n")
    );
}
