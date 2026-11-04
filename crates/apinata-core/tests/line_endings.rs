//! Byte-compared files must be LF on every checkout, including Windows with autocrlf.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn git(args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("git prints UTF-8")
}

const BINARY_EXTENSIONS: [&str; 4] = ["png", "ico", "icns", "ttf"];

#[test]
fn no_tracked_text_file_contains_a_carriage_return() {
    let mut offenders = Vec::new();
    for rel in git(&["ls-files", "-z"])
        .split('\0')
        .filter(|p| !p.is_empty())
    {
        let path = repo_root().join(rel);
        if path
            .extension()
            .is_some_and(|e| BINARY_EXTENSIONS.iter().any(|b| e == *b))
        {
            continue;
        }
        // A tracked file deleted in the work tree is another test's concern.
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if bytes.contains(&b'\r') {
            offenders.push(rel.to_owned());
        }
    }
    assert!(offenders.is_empty(), "CR bytes in {offenders:?}");
}

#[test]
fn corpora_snapshots_and_scripts_are_forced_to_lf() {
    let paths = [
        "crates/apinata-core/tests/fixtures/reference/2026-10-09-writer-parity/expected/cases/scalar-control.yml",
        "crates/apinata-core/tests/fixtures/reference/2026-10-09-writer-parity/cases.json",
        "crates/apinata-core/tests/fixtures/reference/2026-10-09-acceptance-verdicts/verdicts.tsv",
        "crates/apinata-core/tests/snapshots/any.snap",
        "crates/apinata-core/tests/fixtures/bruno/cli-docs/opencollection.yml",
        "scripts/package-linux.sh",
    ];
    let mut args = vec!["check-attr", "eol", "--"];
    args.extend(paths);
    let report = git(&args);
    for path in paths {
        assert!(
            report.contains(&format!("{path}: eol: lf")),
            "{path}:\n{report}"
        );
    }
}

#[test]
fn icon_exports_are_binary() {
    let report = git(&[
        "check-attr",
        "text",
        "--",
        "assets/icon/generated/apinata.ico",
        "assets/icon/generated/apinata.icns",
        "assets/icon/generated/png/16x16.png",
    ]);
    assert_eq!(report.lines().count(), 3, "{report}");
    for line in report.lines() {
        assert!(line.ends_with(": text: unset"), "{line}");
    }
}
