//! The release notes that `scripts/release.sh notes` drafts from a scratch git history.
#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const REPO: &str = "example/apinata";

/// A command in DIR with git isolated from the user's config and from any
/// repository a calling hook points git at.
fn isolated(program: &str, dir: &Path, home: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd
}

/// Run git in DIR, isolated from the user's config and hooks.
fn git(dir: &Path, home: &Path, args: &[&str]) {
    let out = isolated("git", dir, home)
        .args(args)
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

/// What GitHub's generate-notes returns as `.body` for a release with two pull requests.
const GENERATED: &str = "## What's Changed\n### New features\n* Add a sidebar filter by @dev in https://github.com/example/apinata/pull/7\n### Fixes\n* Keep the cookie jar on reload by @dev in https://github.com/example/apinata/pull/8\n\n\n**Full Changelog**: https://github.com/example/apinata/compare/v0.1.0...v0.2.0";

/// A fake `gh` in T that logs its arguments to `gh.log`, one per line, and prints
/// `gh-body.md` (GENERATED unless a test wrote one); `gh-fail` makes it fail.
fn fake_gh(t: &TempDir) -> std::path::PathBuf {
    let body = t.path().join("gh-body.md");
    if !body.exists() {
        std::fs::write(&body, GENERATED).unwrap();
    }
    let gh = t.path().join("gh");
    std::fs::write(
        &gh,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >>'{dir}/gh.log'\nif [ -e '{dir}/gh-fail' ]; then echo 'HTTP 404: Not Found' >&2; exit 1; fi\ncat '{dir}/gh-body.md'\n",
            dir = t.path().display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    gh
}

/// Run `release.sh notes` for TAG with DIST holding FILES and a fake `gh`.
fn run_notes(t: &TempDir, tag: &str, files: &[&str]) -> std::process::Output {
    let dist = t.path().join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    for f in files {
        std::fs::write(dist.join(f), b"x").unwrap();
    }
    let repo = t.path().join("repo");
    isolated("bash", &repo, &t.path().join("home"))
        .arg(xtask::workspace_root().join("scripts/release.sh"))
        .args(["notes", tag, REPO])
        .arg(&dist)
        .arg(t.path().join("notes.md"))
        .env("GH", fake_gh(t))
        .output()
        .unwrap()
}

/// Draft the notes for TAG with DIST holding FILES, and return them.
fn notes(t: &TempDir, tag: &str, files: &[&str]) -> String {
    let out = run_notes(t, tag, files);
    assert!(out.status.success(), "{out:?}");
    std::fs::read_to_string(t.path().join("notes.md")).unwrap()
}

/// The arguments the fake `gh` was called with, one per line.
fn gh_args(t: &TempDir) -> Vec<String> {
    std::fs::read_to_string(t.path().join("gh.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
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
fn whats_changed_is_the_generated_list_of_merged_pull_requests() {
    let t = history(&["feat: first", "tag:v0.1.0", "fix: second", "tag:v0.2.0"]);
    let text = notes(&t, "v0.2.0", &[]);
    assert_eq!(
        gh_args(&t),
        [
            "api",
            "repos/example/apinata/releases/generate-notes",
            "-f",
            "tag_name=v0.2.0",
            "-f",
            "previous_tag_name=v0.1.0",
            "--jq",
            ".body"
        ]
    );
    let changed = section(&text, "What's Changed");
    assert_eq!(
        changed,
        "## What's Changed\n\n### New features\n* Add a sidebar filter by @dev in https://github.com/example/apinata/pull/7\n### Fixes\n* Keep the cookie jar on reload by @dev in https://github.com/example/apinata/pull/8\n\n**Full Changelog**: https://github.com/example/apinata/compare/v0.1.0...v0.2.0\n\n",
    );
    assert_eq!(text.matches("## What's Changed").count(), 1, "{text}");
}

#[test]
fn an_empty_generated_list_reads_as_maintenance_only() {
    let t = history(&["feat: first", "tag:v0.1.0", "chore: tidy", "tag:v0.2.0"]);
    std::fs::write(
        t.path().join("gh-body.md"),
        "**Full Changelog**: https://github.com/example/apinata/compare/v0.1.0...v0.2.0",
    )
    .unwrap();
    let changed = section(&notes(&t, "v0.2.0", &[]), "What's Changed").to_owned();
    assert_eq!(
        changed,
        "## What's Changed\n\n- Maintenance only.\n\n**Full Changelog**: https://github.com/example/apinata/compare/v0.1.0...v0.2.0\n\n",
    );
}

#[test]
fn a_failing_gh_fails_the_notes_and_writes_nothing() {
    let t = history(&["feat: first", "tag:v0.1.0", "fix: second", "tag:v0.2.0"]);
    std::fs::write(t.path().join("gh-fail"), "").unwrap();
    let out = run_notes(&t, "v0.2.0", &[]);
    assert!(!out.status.success(), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("generate-notes"), "{err}");
    assert!(!t.path().join("notes.md").exists());
}

#[test]
fn an_empty_gh_reply_fails_the_notes() {
    let t = history(&["feat: first", "tag:v0.1.0", "fix: second", "tag:v0.2.0"]);
    std::fs::write(t.path().join("gh-body.md"), "").unwrap();
    let out = run_notes(&t, "v0.2.0", &[]);
    assert!(!out.status.success(), "{out:?}");
    assert!(!t.path().join("notes.md").exists());
}

#[test]
fn the_intro_skips_housekeeping_since_the_previous_tag() {
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
    assert!(
        text.starts_with("This release adds 1 feature and fixes 2 bugs.\n"),
        "{text}"
    );
}

#[test]
fn the_first_release_asks_without_a_previous_tag_and_has_no_changelog_link() {
    let t = history(&["feat: the first feature", "chore: setup", "tag:v0.1.0"]);
    std::fs::write(
        t.path().join("gh-body.md"),
        "## What's Changed\n* The first feature by @dev in https://github.com/example/apinata/pull/1\n\n**Full Changelog**: https://github.com/example/apinata/commits/v0.1.0",
    )
    .unwrap();
    let text = notes(&t, "v0.1.0", &[]);
    assert_eq!(
        gh_args(&t),
        [
            "api",
            "repos/example/apinata/releases/generate-notes",
            "-f",
            "tag_name=v0.1.0",
            "--jq",
            ".body"
        ]
    );
    let changed = section(&text, "What's Changed");
    assert!(changed.contains("* The first feature by @dev"), "{changed}");
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

#[test]
fn a_pre_release_tag_is_not_the_previous_release() {
    let t = history(&[
        "feat: one",
        "tag:v0.1.0",
        "feat: two",
        "tag:v0.2.0-rc.1",
        "feat: three",
        "tag:v0.2.0",
    ]);
    let text = notes(&t, "v0.2.0", &[]);
    assert!(gh_args(&t).contains(&"previous_tag_name=v0.1.0".to_owned()));
    assert!(
        text.starts_with("This release adds 2 features.\n"),
        "{text}"
    );
    assert!(
        section(&text, "What's Changed").contains("compare/v0.1.0...v0.2.0"),
        "{text}"
    );
}

#[test]
fn housekeeping_scopes_are_left_out_inside_a_comma_list() {
    let t = history(&[
        "feat: base",
        "tag:v0.1.0",
        "fix(ci, deps): x",
        "fix(deps,build): w",
        "fix(app, core): y",
        "tag:v0.2.0",
    ]);
    let text = notes(&t, "v0.2.0", &[]);
    assert!(text.starts_with("This release fixes 1 bug.\n"), "{text}");
}

#[test]
fn the_helper_removes_the_variables_that_redirect_git() {
    let t = tempfile::tempdir().unwrap();
    let cmd = isolated("git", t.path(), t.path());
    for key in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"] {
        let set = cmd.get_envs().find(|(k, _)| *k == key);
        assert_eq!(set, Some((std::ffi::OsStr::new(key), None)), "{key}");
    }
}
