//! The updater's notes.json generator: parsing published release bodies, the size cap, and
//! that core's notes types read what it writes.

use apinata_core::update::{GroupKind, NotesRelease, ReleaseNotes};
use std::path::PathBuf;
use std::process::Command;
use xtask::notes_json::{Current, Published, SIZE_CAP, build, read_published};

const REPO: &str = "https://github.com/example/apinata";

fn fixture(name: &str) -> PathBuf {
    xtask::workspace_root()
        .join("crates/xtask/tests/fixtures/notes")
        .join(name)
}

fn read(name: &str) -> String {
    std::fs::read_to_string(fixture(name)).unwrap()
}

/// The repository's own label categories, so a renamed section fails here first.
fn categories() -> String {
    std::fs::read_to_string(xtask::workspace_root().join(".github/release.yml")).unwrap()
}

fn current<'a>(generated: &'a str, intro: Option<&'a str>) -> Current<'a> {
    Current {
        version: "0.3.0",
        date: "2026-12-01",
        generated,
        intro,
    }
}

/// The notes built from the recorded fixtures for v0.3.0.
fn fixture_notes() -> String {
    let generated = read("generated.md");
    let annotation = read("annotation.txt");
    let published = read_published(&fixture("published")).unwrap();
    build(
        &current(&generated, Some(&annotation)),
        published,
        &categories(),
    )
    .unwrap()
}

fn parsed(json: &str) -> ReleaseNotes {
    serde_json::from_str(json).unwrap()
}

fn release<'a>(notes: &'a ReleaseNotes, version: &str) -> &'a NotesRelease {
    (notes.releases.iter())
        .find(|r| r.version == version)
        .unwrap_or_else(|| panic!("no {version}"))
}

/// `(text, pr)` for every item of every group of `r`.
fn items(r: &NotesRelease) -> Vec<(String, Option<u64>)> {
    (r.groups.iter())
        .flat_map(|g| g.items.iter().map(|i| (i.text.clone(), i.pr)))
        .collect()
}

fn bullets(n: usize, width: usize) -> String {
    (0..n)
        .map(|i| {
            format!(
                "* Change {i:04} {} by @dev in {REPO}/pull/{i}\n",
                "x".repeat(width)
            )
        })
        .collect()
}

fn published(version: &str, body: String) -> Published {
    Published {
        tag: format!("v{version}"),
        date: Some("2026-11-01".to_owned()),
        body,
    }
}

#[test]
fn releases_run_newest_first_each_naming_the_one_before() {
    let notes = parsed(&fixture_notes());
    assert_eq!(notes.schema, 1);
    let got: Vec<(&str, Option<&str>, Option<&str>)> = (notes.releases.iter())
        .map(|r| {
            let (prev, date) = (r.previous.as_deref(), r.date.as_deref());
            (r.version.as_str(), prev, date)
        })
        .collect();
    assert_eq!(
        got,
        [
            ("0.3.0", Some("0.2.0"), Some("2026-12-01")),
            ("0.2.0", Some("0.1.1"), Some("2026-11-10")),
            ("0.1.1", Some("0.1.0"), Some("2026-11-05")),
            ("0.1.0", None, Some("2026-11-01")),
        ],
        "drafts, pre-releases and other crates' tags are left out"
    );
}

#[test]
fn the_newest_intro_comes_from_the_tag_and_older_ones_from_their_bodies() {
    let notes = parsed(&fixture_notes());
    let intros: Vec<Option<&str>> = notes.releases.iter().map(|r| r.intro.as_deref()).collect();
    assert_eq!(
        intros,
        [
            Some("Find any request in a large collection."),
            Some("This release adds a sidebar filter and keeps the cookie jar on reload."),
            Some("A small fix release."),
            Some("The first release of Apinata."),
        ]
    );
}

#[test]
fn section_titles_map_to_kinds_through_the_label_categories() {
    let notes = parsed(&fixture_notes());
    let kinds = |v: &str| -> Vec<(GroupKind, String)> {
        (release(&notes, v).groups.iter())
            .map(|g| (g.kind, g.title.clone()))
            .collect()
    };
    assert_eq!(
        kinds("0.3.0"),
        [
            (GroupKind::Features, "New features".to_owned()),
            (
                GroupKind::Usability,
                "Usability and accessibility".to_owned()
            ),
            (GroupKind::Performance, "Performance".to_owned()),
        ]
    );
    assert_eq!(
        kinds("0.2.0"),
        [
            (GroupKind::Features, "New features".to_owned()),
            (GroupKind::Fixes, "Fixes".to_owned()),
            (GroupKind::Other, "Other changes".to_owned()),
        ]
    );
}

#[test]
fn items_lose_their_author_link_and_markdown_but_keep_the_pull_request() {
    let json = fixture_notes();
    let notes = parsed(&json);
    let new = items(release(&notes, "0.3.0"));
    assert_eq!(
        new,
        [
            ("Search the request list".to_owned(), Some(20)),
            ("Add --json output to apinata run".to_owned(), Some(21)),
            ("Read the status bar aloud".to_owned(), Some(22)),
            ("Open large collections faster".to_owned(), Some(23)),
        ]
    );
    for gone in [
        "http",
        "@dev",
        "@other",
        "first contribution",
        "Full Changelog",
        "<!--",
    ] {
        assert!(!json.contains(gone), "{gone:?} in {json}");
    }
}

#[test]
fn an_edited_body_still_parses_its_links_references_and_own_sections() {
    let notes = parsed(&fixture_notes());
    let r = release(&notes, "0.1.1");
    assert_eq!(
        items(r),
        [
            ("Fix the proxy toggle".to_owned(), Some(5)),
            ("Show curl errors in plain words".to_owned(), Some(6)),
            ("Handle redirects again".to_owned(), Some(4)),
            (
                "Name the active environment in the title bar".to_owned(),
                None
            ),
        ]
    );
    let own = &r.groups[1];
    assert_eq!(
        (own.kind, own.title.as_str()),
        (GroupKind::Other, "Something new we called it")
    );
}

#[test]
fn bullets_outside_any_section_form_one_changes_group_with_no_kind() {
    let json = fixture_notes();
    let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
    let first = &raw["releases"][3];
    assert_eq!(first["version"], "0.1.0");
    let groups = first["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["title"], "Changes");
    assert!(groups[0].get("kind").is_none(), "{}", groups[0]);
    assert_eq!(
        groups[0]["items"][1]["text"],
        "Send requests over HTTP/1.1 and HTTP/2"
    );
    assert!(first.get("previous").is_none_or(|p| p.is_null()));
}

#[test]
fn the_output_matches_the_recorded_notes_core_reads() {
    // core's update tests serve this same file through the client, so a change on either
    // side of the format fails one of them.
    let want = read("notes.json");
    assert_eq!(fixture_notes(), want);
}

#[test]
fn a_section_with_bullets_but_nothing_readable_fails() {
    let generated = format!(
        "## What's Changed\n### Fixes\n* {REPO}/pull/3\n* by @dev in {REPO}/pull/4\n* [](https://x)\n"
    );
    let err = build(&current(&generated, None), Vec::new(), &categories()).unwrap_err();
    assert!(format!("{err:#}").contains("0.3.0"), "{err:#}");

    let body = "## What's Changed\n\n* **\n\n## Downloads\n".to_owned();
    let err = build(
        &current("", None),
        vec![published("0.2.0", body)],
        &categories(),
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("0.2.0"), "{err:#}");
}

#[test]
fn a_release_with_no_pull_requests_says_maintenance_only() {
    // GitHub's reply for a tag with no merged pull requests is the changelog link alone.
    for generated in [
        format!("**Full Changelog**: {REPO}/compare/v0.2.0...v0.3.0\n"),
        "## What's Changed\n\n".to_owned(),
    ] {
        let json = build(&current(&generated, None), Vec::new(), &categories()).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
        let groups = raw["releases"][0]["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1, "{generated:?}");
        assert_eq!(groups[0]["title"], "Changes");
        assert!(groups[0].get("kind").is_none());
        assert_eq!(
            groups[0]["items"],
            serde_json::json!([{"text": "Maintenance only."}])
        );
        assert_eq!(parsed(&json).releases[0].previous, None);
    }
}

#[test]
fn generated_notes_without_their_section_fail_but_an_old_body_is_only_empty() {
    let generated = format!("### Fixes\n* A fix by @dev in {REPO}/pull/4\n");
    let err = build(&current(&generated, None), Vec::new(), &categories()).unwrap_err();
    assert!(format!("{err:#}").contains("What's Changed"), "{err:#}");

    let body = "Hand written.\n\n### Fixes\n* A fix\n".to_owned();
    let notes = parsed(
        &build(
            &current("## What's Changed\n* New\n", None),
            vec![published("0.2.0", body)],
            &categories(),
        )
        .unwrap(),
    );
    let old = release(&notes, "0.2.0");
    assert!(old.groups.is_empty() && old.intro.is_none());
}

/// The items one published bullet list parses to.
fn parse_bullets(bullets: &str) -> Vec<(String, Option<u64>)> {
    let body = format!("## What's Changed\n\n{bullets}");
    let notes = parsed(
        &build(
            &current("", None),
            vec![published("0.2.0", body)],
            &categories(),
        )
        .unwrap(),
    );
    items(release(&notes, "0.2.0"))
}

#[test]
fn only_paired_markdown_markers_are_removed() {
    let got = parse_bullets(
        "* Read settings from `~/.config/apinata` and *.yml files\n\
         * Keep a ~ in paths, 2 * 3 and snake_case names\n\
         * **Bold**, *it*, _under_, __strong__ and ~~gone~~ text\n",
    );
    let texts: Vec<&str> = got.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        texts,
        [
            "Read settings from ~/.config/apinata and *.yml files",
            "Keep a ~ in paths, 2 * 3 and snake_case names",
            "Bold, it, under, strong and gone text",
        ]
    );
}

#[test]
fn only_a_trailing_parenthesised_reference_is_taken_as_the_pull_request() {
    assert_eq!(
        parse_bullets("* Close the dialog, see issue #3 here\n* Fix it #4\n* Squashed (#5)\n"),
        [
            ("Close the dialog, see issue #3 here".to_owned(), None),
            ("Fix it #4".to_owned(), None),
            ("Squashed".to_owned(), Some(5)),
        ]
    );
}

#[test]
fn an_annotation_that_starts_with_the_release_name_keeps_only_the_rest() {
    let annotation = "Apinata 0.3.0\n\nFind any request\nin a large collection.\n";
    let notes = parsed(&build(&current("", Some(annotation)), Vec::new(), &categories()).unwrap());
    assert_eq!(
        notes.releases[0].intro.as_deref(),
        Some("Find any request in a large collection.")
    );
}

#[test]
fn only_text_between_whats_changed_and_downloads_is_read() {
    let body = "Intro.\n\n* Before the section\n\n## What's Changed\n\n* Inside\n\n## Downloads\n\n* After the section\n".to_owned();
    let notes = parsed(
        &build(
            &current("", None),
            vec![published("0.2.0", body)],
            &categories(),
        )
        .unwrap(),
    );
    assert_eq!(
        items(release(&notes, "0.2.0")),
        [("Inside".to_owned(), None)]
    );
}

#[test]
fn an_annotation_that_only_names_the_release_is_no_intro() {
    for annotation in ["Apinata 0.3.0\n\n", "v0.3.0", "0.3.0\n", "  \n", ""] {
        let notes =
            parsed(&build(&current("", Some(annotation)), Vec::new(), &categories()).unwrap());
        assert_eq!(notes.releases[0].intro, None, "{annotation:?}");
    }
}

#[test]
fn releases_at_or_above_this_version_are_left_out() {
    let body = || "## What's Changed\n\n* A change\n".to_owned();
    let notes = parsed(
        &build(
            &current("", None),
            vec![
                published("0.4.0", body()),
                published("0.3.0", body()),
                published("0.2.0", body()),
            ],
            &categories(),
        )
        .unwrap(),
    );
    let versions: Vec<&str> = notes.releases.iter().map(|r| r.version.as_str()).collect();
    assert_eq!(versions, ["0.3.0", "0.2.0"]);
}

#[test]
fn at_most_twenty_releases_are_kept() {
    let history: Vec<Published> = (0..30)
        .map(|i| {
            published(
                &format!("0.2.{i}"),
                "## What's Changed\n\n* A change\n".to_owned(),
            )
        })
        .collect();
    let notes = parsed(&build(&current("", None), history, &categories()).unwrap());
    assert_eq!(notes.releases.len(), 20);
    let oldest = notes.releases.last().unwrap();
    assert_eq!(
        (oldest.version.as_str(), oldest.previous.as_deref()),
        ("0.2.11", Some("0.2.10"))
    );
}

#[test]
fn an_oversized_history_drops_the_oldest_releases_first() {
    let history: Vec<Published> = (0..19)
        .map(|i| {
            let body = format!("## What's Changed\n### Fixes\n{}", bullets(40, 80));
            published(&format!("0.2.{i}"), body)
        })
        .collect();
    let generated = format!("## What's Changed\n### New features\n{}", bullets(40, 80));
    let json = build(&current(&generated, None), history, &categories()).unwrap();
    assert!(json.len() <= SIZE_CAP, "{} bytes", json.len());
    let notes = parsed(&json);
    assert!(notes.releases.len() < 20, "something was dropped");
    assert!(notes.releases.len() > 1, "only as much as needed");
    assert_eq!(items(&notes.releases[0]).len(), 40, "the newest is whole");
    assert_eq!(
        notes.releases[1].version, "0.2.18",
        "the newest published stays"
    );
    let oldest = notes.releases.last().unwrap();
    let below = oldest
        .version
        .trim_start_matches("0.2.")
        .parse::<u32>()
        .unwrap()
        - 1;
    assert_eq!(oldest.previous, Some(format!("0.2.{below}")));
}

#[test]
fn a_newest_release_over_the_cap_alone_is_cut_with_a_count_of_the_rest() {
    let generated = format!(
        "## What's Changed\n### New features\n{}### Fixes\n{}",
        bullets(700, 60),
        bullets(800, 60)
    );
    let history = vec![published(
        "0.2.0",
        "## What's Changed\n\n* Old\n".to_owned(),
    )];
    let json = build(&current(&generated, None), history, &categories()).unwrap();
    assert!(json.len() <= SIZE_CAP, "{} bytes", json.len());
    assert!(json.len() > SIZE_CAP - 1024, "cut no more than needed");
    let notes = parsed(&json);
    assert_eq!(notes.releases.len(), 1, "older releases go first");
    let all = items(&notes.releases[0]);
    let (last, _) = all.last().unwrap();
    let more: usize = last
        .strip_prefix("...and ")
        .and_then(|s| s.strip_suffix(" more"))
        .unwrap_or_else(|| panic!("last item {last:?}"))
        .parse()
        .unwrap();
    assert_eq!(all.len() - 1 + more, 1500);
    assert_eq!(
        all[0].0,
        format!("Change 0000 {}", "x".repeat(60)),
        "cut from the end"
    );
}

#[test]
fn a_newest_release_that_cannot_fit_alone_fails() {
    let intro = "word ".repeat(SIZE_CAP / 4);
    let err = build(
        &current("## What's Changed\n\n* A change\n", Some(&intro)),
        Vec::new(),
        &categories(),
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("0.3.0"), "{err:#}");
}

#[test]
fn a_pre_release_version_is_refused() {
    let mut c = current("", None);
    c.version = "0.3.0-rc.1";
    assert!(build(&c, Vec::new(), &categories()).is_err());
}

#[test]
fn the_command_writes_notes_json_from_the_recorded_inputs() {
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("notes.json");
    let status = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["notes-json", "--version", "0.3.0", "--date", "2026-12-01"])
        .arg("--generated")
        .arg(fixture("generated.md"))
        .arg("--published")
        .arg(fixture("published"))
        .arg("--intro")
        .arg(fixture("annotation.txt"))
        .arg("--output")
        .arg(&file)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), read("notes.json"));
}
