//! `apinata ls`: the item tree as text or JSON, in the order the MCP `list_items` tool uses.

use crate::common;

use common::{ROOT, apinata, write_tree};
use serde_json::Value;

fn request(name: &str, seq: u32, method: &str, tags: &[&str]) -> String {
    let tags = if tags.is_empty() {
        String::new()
    } else {
        let items: String = tags.iter().map(|t| format!("    - {t}\n")).collect();
        format!("  tags:\n{items}")
    };
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\n{tags}http:\n  method: {method}\n  url: \"http://x/{name}\"\n"
    )
}

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "second.yml",
            &request("second", 2, "POST", &["smoke", "slow"]),
        ),
        ("first.yml", &request("first", 1, "GET", &[])),
        ("users/get.yml", &request("get-user", 1, "GET", &["smoke"])),
    ])
}

fn stdout(args: &[&str], dir: &std::path::Path) -> String {
    let out = apinata().current_dir(dir).args(args).assert().success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

#[test]
fn ls_prints_an_indented_tree_in_sibling_order() {
    let dir = tree();
    let text = stdout(&["ls"], dir.path());
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    assert_eq!(lines[0], "users/", "{text}");
    assert!(lines[1].starts_with("  GET  get-user"), "{text}");
    assert!(lines[1].ends_with("users/get.yml"), "{text}");
    assert!(lines[2].starts_with("GET  first"), "{text}");
    assert!(lines[2].ends_with("first.yml"), "{text}");
    assert!(lines[3].starts_with("POST  second"), "{text}");
    assert!(!text.contains("smoke"), "tags only with --tags: {text}");
}

#[test]
fn ls_tags_appends_the_request_tags() {
    let dir = tree();
    let text = stdout(&["ls", "--tags"], dir.path());
    assert!(text.contains("[smoke, slow]"), "{text}");
    assert!(text.contains("[smoke]"), "{text}");
}

#[test]
fn ls_path_lists_one_folder() {
    let dir = tree();
    let text = stdout(&["ls", "users"], dir.path());
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.starts_with("GET  get-user"), "{text}");
}

#[test]
fn ls_json_has_the_list_items_shape() {
    let dir = tree();
    let json: Value = serde_json::from_str(&stdout(&["ls", "--json"], dir.path())).unwrap();
    let items = json["items"].as_array().unwrap();
    let paths: Vec<&str> = items.iter().map(|i| i["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["users", "first.yml", "second.yml"]);
    assert_eq!(items[1]["kind"], "request");
    assert_eq!(items[1]["method"], "GET");
    assert_eq!(items[1]["url"], "http://x/first");
    assert_eq!(items[1]["seq"], 1);
    assert!(items[1].get("tags").is_none());
    assert_eq!(items[0]["kind"], "folder");
    assert_eq!(items[0]["items"][0]["path"], "users/get.yml");

    let json: Value =
        serde_json::from_str(&stdout(&["ls", "--json", "--tags"], dir.path())).unwrap();
    assert_eq!(
        json["items"][2]["tags"],
        serde_json::json!(["smoke", "slow"])
    );
    assert_eq!(
        json["items"][0]["items"][0]["tags"],
        serde_json::json!(["smoke"])
    );
}

#[test]
fn ls_unknown_folder_and_request_file_paths_exit_3() {
    let dir = tree();
    for path in ["nope", "first.yml"] {
        let out = apinata()
            .current_dir(dir.path())
            .args(["ls", path])
            .assert()
            .code(3);
        assert!(!out.get_output().stderr.is_empty(), "{path}");
    }
}

#[test]
fn ls_outside_a_collection_exits_3() {
    let dir = write_tree(&[("note.txt", "hi")]);
    apinata().current_dir(dir.path()).arg("ls").assert().code(3);
}

#[test]
fn ls_collection_flag_names_the_root() {
    let dir = tree();
    let other = tempfile::tempdir().unwrap();
    let out = apinata()
        .current_dir(other.path())
        .arg("ls")
        .arg("--collection")
        .arg(dir.path())
        .assert()
        .success();
    assert!(String::from_utf8_lossy(&out.get_output().stdout).contains("first.yml"));
}

#[test]
fn ls_text_escapes_control_characters_and_keeps_one_entry_per_line() {
    let evil = "info:\n  name: \"bad\\u001b[31m\\nforged\"\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/\"\n  \nruntime:\n  tags: []\n";
    let tagged = "info:\n  name: t\n  type: http\n  seq: 2\n  tags:\n    - \"tag\\u001b[1m\"\nhttp:\n  method: GET\n  url: \"http://x/\"\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("evil.yml", evil),
        ("tagged.yml", tagged),
        ("dir\u{1b}[2J/r.yml", &request("r", 1, "GET", &[])),
    ]);
    let text = stdout(&["ls", "--tags"], dir.path());
    assert!(!text.contains('\u{1b}'), "{text:?}");
    assert!(text.contains("\\u{1b}"), "{text:?}");
    assert!(!text.contains("bad\u{1b}"), "{text:?}");
    // Folder, nested request and two top-level requests: one line each.
    assert_eq!(text.lines().count(), 4, "{text:?}");
    let json = stdout(&["ls", "--json"], dir.path());
    assert!(json.contains("forged"), "JSON keeps the real name");
}

#[test]
fn ls_collection_pointing_at_a_file_exits_3() {
    let dir = tree();
    apinata()
        .arg("ls")
        .arg("--collection")
        .arg(dir.path().join("first.yml"))
        .assert()
        .code(3);
}

fn folder_file(name: &str) -> String {
    format!("info:\n  name: \"{name}\"\n  type: folder\n")
}

#[test]
fn ls_shows_the_path_of_a_renamed_folder_and_accepts_its_display_name() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("Posts/folder.yml", &folder_file("Blog Posts")),
        ("Posts/p.yml", &request("p", 1, "GET", &[])),
        ("plain/p.yml", &request("q", 1, "GET", &[])),
    ]);
    let text = stdout(&["ls"], dir.path());
    assert!(text.contains("Blog Posts/  Posts\n"), "{text:?}");
    assert!(text.starts_with("plain/\n"), "{text:?}");
    let by_name = stdout(&["ls", "Blog Posts"], dir.path());
    assert!(by_name.contains("Posts/p.yml"), "{by_name:?}");
}

#[test]
fn ls_ambiguous_display_name_exits_3_and_lists_the_paths() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a/folder.yml", &folder_file("Twin")),
        ("b/folder.yml", &folder_file("Twin")),
    ]);
    let out = apinata()
        .current_dir(dir.path())
        .args(["ls", "Twin"])
        .assert()
        .code(3);
    let err = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(err.contains("a, b"), "{err}");
}

#[test]
fn ls_escapes_control_characters_in_a_renamed_folder() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("Posts/folder.yml", &folder_file("bad\\u001b[31m")),
        ("Posts/p.yml", &request("p", 1, "GET", &[])),
    ]);
    let text = stdout(&["ls"], dir.path());
    assert!(!text.contains('\u{1b}'), "{text:?}");
    assert!(text.contains("\\u{1b}"), "{text:?}");
}

const CONFLICT: &str =
    "info:\n  name: a\n<<<<<<< HEAD\n  type: http\n=======\n  type: ws\n>>>>>>> other\n";
const REASON: &str = "merge conflict markers at line 3; resolve them in git";

fn broken_tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("ok.yml", &request("ok", 1, "GET", &[])),
        ("bad.yml", CONFLICT),
    ])
}

#[test]
fn ls_lists_a_file_that_fails_to_load_as_broken_with_its_reason() {
    let dir = broken_tree();
    let text = stdout(&["ls"], dir.path());
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].starts_with("GET  ok"), "{text}");
    assert_eq!(
        lines[1],
        format!("broken  bad  bad.yml  {REASON}"),
        "{text}"
    );
}

#[test]
fn ls_json_carries_kind_broken_and_an_error_field_only_there() {
    let dir = broken_tree();
    let json: Value = serde_json::from_str(&stdout(&["ls", "--json"], dir.path())).unwrap();
    let items = json["items"].as_array().unwrap();
    assert_eq!(items[0]["kind"], "request");
    assert!(items[0].get("error").is_none());
    assert_eq!(items[1]["kind"], "broken");
    assert_eq!(items[1]["name"], "bad");
    assert_eq!(items[1]["path"], "bad.yml");
    assert_eq!(items[1]["error"], REASON);
}

fn broken_folder_tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", CONFLICT),
        ("f/get.yml", &request("get", 1, "GET", &[])),
    ])
}

#[test]
fn ls_text_shows_a_folders_load_error_on_its_line() {
    let dir = broken_folder_tree();
    let text = stdout(&["ls"], dir.path());
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], format!("f/  {REASON}"), "{text}");
    assert!(lines[1].starts_with("  GET  get"), "{text}");
}

#[test]
fn ls_json_carries_a_folders_load_error() {
    let dir = broken_folder_tree();
    let json: Value = serde_json::from_str(&stdout(&["ls", "--json"], dir.path())).unwrap();
    assert_eq!(json["items"][0]["kind"], "folder");
    assert_eq!(json["items"][0]["error"], REASON);
}

#[test]
fn ls_does_not_list_an_ignored_or_opaque_file_as_broken() {
    let grpc = "info:\n  name: Query\n  type: grpc\n  seq: 3\n";
    let dir = write_tree(&[("opencollection.yml", ROOT), ("q.yml", grpc)]);
    let text = stdout(&["ls"], dir.path());
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.starts_with("other  "), "{text}");
}

const LEGACY_TEXT: &str =
    "users/\n  GET  get-user  users/get.yml\nGET  first  first.yml\nPOST  second  second.yml\n";

#[test]
fn ls_without_paging_flags_prints_exactly_the_legacy_text() {
    let dir = tree();
    assert_eq!(stdout(&["ls"], dir.path()), LEGACY_TEXT);
}

#[test]
fn ls_without_paging_flags_prints_the_legacy_json() {
    let dir = tree();
    let json: Value = serde_json::from_str(&stdout(&["ls", "--json"], dir.path())).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"items": [
            {"path": "users", "kind": "folder", "name": "users", "items": [
                {"path": "users/get.yml", "kind": "request", "name": "get-user", "seq": 1,
                 "method": "GET", "url": "http://x/get-user"}]},
            {"path": "first.yml", "kind": "request", "name": "first", "seq": 1,
             "method": "GET", "url": "http://x/first"},
            {"path": "second.yml", "kind": "request", "name": "second", "seq": 2,
             "method": "POST", "url": "http://x/second"}
        ]})
    );
}

#[test]
fn ls_depth_one_text_counts_folder_children_and_closes_with_a_range() {
    let dir = tree();
    let text = stdout(&["ls", "--depth", "1"], dir.path());
    assert_eq!(
        text,
        "users/  (1 items)\nGET  first  first.yml\nPOST  second  second.yml\n(showing 1-3 of 3)\n"
    );
}

#[test]
fn ls_depth_one_json_carries_child_count_total_and_has_more() {
    let dir = tree();
    let json: Value =
        serde_json::from_str(&stdout(&["ls", "--json", "--depth", "1"], dir.path())).unwrap();
    assert_eq!(json["items"][0]["child_count"], 1);
    assert!(json["items"][0].get("items").is_none());
    assert_eq!(json["total"], 3);
    assert_eq!(json["has_more"], false);
}

#[test]
fn ls_limit_offset_page_and_a_past_the_end_page_is_empty() {
    let dir = tree();
    let text = stdout(&["ls", "--limit", "1", "--offset", "1"], dir.path());
    assert_eq!(text, "GET  first  first.yml\n(showing 2-2 of 3)\n");
    let json: Value = serde_json::from_str(&stdout(
        &["ls", "--json", "--limit", "1", "--offset", "1"],
        dir.path(),
    ))
    .unwrap();
    assert_eq!(
        (&json["has_more"], &json["limit"]),
        (&true.into(), &1.into())
    );
    let past = stdout(&["ls", "--offset", "9"], dir.path());
    assert_eq!(past, "(no items at offset 9 of 3)\n");
}

#[test]
fn ls_limit_zero_exits_3_and_negative_values_are_usage_errors() {
    let dir = tree();
    let out = apinata()
        .current_dir(dir.path())
        .args(["ls", "--limit", "0"])
        .assert()
        .code(3);
    let err = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(err.contains("limit"), "{err}");
    for flag in ["--limit=-1", "--offset=-1", "--depth=-1"] {
        apinata()
            .current_dir(dir.path())
            .args(["ls", flag])
            .assert()
            .failure();
    }
}

#[test]
fn ls_help_mentions_the_paging_flags() {
    let text = stdout(&["ls", "--help"], std::path::Path::new("."));
    for flag in ["--depth", "--limit", "--offset"] {
        assert!(text.contains(flag), "{flag}: {text}");
    }
}
