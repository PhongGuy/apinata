//! The release notes that `scripts/release.sh notes` drafts from a scratch git history.
#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const REPO: &str = "example/apinata";

/// Run git in DIR, isolated from the user's config and hooks.
fn git(dir: &Path, home: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A scratch repository whose history is SUBJECTS in order; a subject starting
/// with `tag:` tags the current commit instead.
fn history(subjects: &[&str]) -> TempDir {
    let t = tempfile::tempdir().unwrap();
    let (repo, home) = (t.path().join("repo"), t.path().join("home"));
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    git(&repo, &home, &["init", "-q", "-b", "master"]);
    for s in subjects {
        match s.strip_prefix("tag:") {
            Some(tag) => git(&repo, &home, &["tag", "-a", tag, "-m", tag]),
            None => git(&repo, &home, &["commit", "-q", "--allow-empty", "-m", s]),
        }
    }
    t
}

/// Draft the notes for TAG with DIST holding FILES, and return them.
fn notes(t: &TempDir, tag: &str, files: &[&str]) -> String {
    let dist = t.path().join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    for f in files {
        std::fs::write(dist.join(f), b"x").unwrap();
    }
    let out_file = t.path().join("notes.md");
    let out = Command::new("bash")
        .arg(xtask::workspace_root().join("scripts/release.sh"))
        .args(["notes", tag, REPO])
        .arg(&dist)
        .arg(&out_file)
        .current_dir(t.path().join("repo"))
        .env("HOME", t.path().join("home"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    std::fs::read_to_string(out_file).unwrap()
}

/// The section between HEADING and the next `## ` heading.
fn section<'a>(text: &'a str, heading: &str) -> &'a str {
    let start = text
        .find(&format!("## {heading}\n"))
        .unwrap_or_else(|| panic!("no {heading} in {text}"));
    let body = &text[start..];
    let end = body[3..].find("\n## ").map_or(body.len(), |i| i + 4);
    &body[..end]
}

#[test]
fn sections_follow_the_intro_in_order() {
    let t = history(&["feat: first", "tag:v0.1.0", "fix: second", "tag:v0.2.0"]);
    let text = notes(&t, "v0.2.0", &[]);
    assert!(!text.starts_with('#'), "the intro comes first: {text}");
    assert!(
        text.starts_with("This release fixes 1 bug.\n\n## What's Changed\n"),
        "{text}"
    );
    let at = |h: &str| {
        text.find(&format!("\n## {h}\n"))
            .unwrap_or_else(|| panic!("no {h}"))
    };
    let order = [
        at("What's Changed"),
        at("Downloads"),
        at("Before you install"),
        at("Updates"),
    ];
    assert!(order.is_sorted(), "{text}");
    assert!(!text.contains("Bruno") && !text.contains(" -- "));
}

#[test]
fn whats_changed_keeps_feat_and_fix_since_the_previous_tag() {
    let t = history(&[
        "feat: before the last tag",
        "tag:v0.1.0",
        "feat(app): add a sidebar filter",
        "fix: keep the cookie jar on reload",
        "fix(core)!: refuse a bad header",
        "chore: tidy",
        "ci: cache the toolchain",
        "test(core): cover redirects",
        "build: bump the linker",
        "fix(deps): keep ico at 0.5.0",
        "feat(release): sign the manifest",
        "fix(ci): pin actionlint",
        "Bump the cargo group with 3 updates",
        "docs: explain cookies",
        "tag:v0.2.0",
        "feat: after the tag",
    ]);
    let text = notes(&t, "v0.2.0", &[]);
    let changed = section(&text, "What's Changed");
    let items: Vec<&str> = changed.lines().filter(|l| l.starts_with("- ")).collect();
    assert_eq!(
        items,
        [
            "- add a sidebar filter",
            "- keep the cookie jar on reload",
            "- refuse a bad header"
        ],
        "{changed}"
    );
    assert!(
        text.starts_with("This release adds 1 feature and fixes 2 bugs.\n"),
        "{text}"
    );
    assert!(
        changed.contains(
            "**Full Changelog**: https://github.com/example/apinata/compare/v0.1.0...v0.2.0"
        ),
        "{changed}"
    );
}

#[test]
fn the_first_release_lists_everything_without_a_compare_link() {
    let t = history(&["feat: the first feature", "chore: setup", "tag:v0.1.0"]);
    let text = notes(&t, "v0.1.0", &[]);
    let changed = section(&text, "What's Changed");
    assert!(changed.contains("- the first feature\n"), "{changed}");
    assert!(
        text.starts_with("The first release of Apinata.\n"),
        "{text}"
    );
    assert!(!text.contains("Full Changelog"), "{text}");
    assert!(!text.contains("compare/"), "{text}");
}

#[test]
fn the_intro_counts_features_and_fixes_and_omits_a_zero_part() {
    let cases: [(&[&str], &str); 4] = [
        (&["feat: a", "feat: b"], "This release adds 2 features."),
        (
            &["fix: a", "feat: b", "fix: c"],
            "This release adds 1 feature and fixes 2 bugs.",
        ),
        (&["fix: a"], "This release fixes 1 bug."),
        (
            &["chore: a", "fix(deps): b"],
            "This release has maintenance changes only.",
        ),
    ];
    for (commits, intro) in cases {
        let mut subjects = vec!["feat: base", "tag:v0.1.0"];
        subjects.extend_from_slice(commits);
        subjects.push("tag:v0.2.0");
        let text = notes(&history(&subjects), "v0.2.0", &[]);
        assert!(
            text.starts_with(&format!("{intro}\n\n")),
            "{commits:?}: {text}"
        );
    }
}

#[test]
fn downloads_carry_the_version_and_mark_an_unsigned_windows_build() {
    let t = history(&["feat: one", "tag:v1.4.2"]);
    let unsigned = notes(&t, "v1.4.2", &["apinata-app_1.4.2_x64-setup-unsigned.exe"]);
    let table = section(&unsigned, "Downloads");
    assert!(
        table.contains(
            "| Linux x86_64 | `apinata-app_1.4.2_amd64.deb` or `apinata-app_1.4.2_x86_64.AppImage` | `apinata-1.4.2-x86_64-unknown-linux-gnu.tar.gz` |"
        ),
        "{table}"
    );
    assert!(
        table.contains(
            "| Windows x86_64 | `apinata-app_1.4.2_x64-setup-unsigned.exe` | `apinata-1.4.2-x86_64-pc-windows-msvc-unsigned.zip` |"
        ),
        "{table}"
    );
    assert!(table.contains("Check a download with `sha256sum -c SHA256SUMS`."));

    let t = history(&["feat: one", "tag:v1.4.2"]);
    let signed = notes(&t, "v1.4.2", &["apinata-app_1.4.2_x64-setup.exe"]);
    assert!(
        section(&signed, "Downloads").contains(
            "| Windows x86_64 | `apinata-app_1.4.2_x64-setup.exe` | `apinata-1.4.2-x86_64-pc-windows-msvc.zip` |"
        ),
        "{signed}"
    );
}

#[test]
fn before_you_install_and_updates_cover_each_platform() {
    let t = history(&["feat: one", "tag:v1.0.0"]);
    let text = notes(&t, "v1.0.0", &[]);
    let before = section(&text, "Before you install");
    for needle in [
        "**Windows:**",
        "SmartScreen",
        "**macOS:**",
        "xattr -dr com.apple.quarantine /Applications/Apinata.app",
        "xattr -d com.apple.quarantine apinata",
        "**Linux:** no extra steps.",
    ] {
        assert!(before.contains(needle), "{needle}: {before}");
    }
    let updates = section(&text, "Updates");
    assert!(
        updates.contains("install in place") && updates.contains("keychain"),
        "{updates}"
    );
}
