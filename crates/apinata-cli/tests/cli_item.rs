//! `apinata item`: each action leaves the collection exactly as the matching MCP tool does.
//! One fixture is copied twice; the CLI edits one copy, `apinata mcp` the other, and the two
//! trees must match byte for byte. Every run has scratch home and data directories.

use crate::common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{ROOT, apinata, write_tree};
use predicates::str::contains;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};
use tempfile::TempDir;

fn request(name: &str, seq: u32, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: {url}\n"
    )
}

fn folder(name: &str, seq: u32) -> String {
    format!("info:\n  name: {name}\n  type: folder\n  seq: {seq}\n")
}

/// The shared fixture: two root items and a folder holding two requests.
fn fixture() -> TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1, "http://localhost/a")),
        ("users/folder.yml", &folder("users", 2)),
        (
            "users/get.yml",
            &request("get", 1, "http://localhost/users/1"),
        ),
        (
            "users/list.yml",
            &request("list", 2, "http://localhost/users"),
        ),
    ])
}

/// A copy of the fixture with its own scratch home, so the CLI and MCP runs never share state.
pub(crate) struct Copy {
    dir: TempDir,
}

impl Copy {
    pub(crate) fn new() -> Copy {
        Copy::of(&fixture())
    }

    /// A copy of the collection in `src`.
    pub(crate) fn of(src: &TempDir) -> Copy {
        Copy::of_dir(src.path())
    }

    /// A copy of the collection at `src`.
    pub(crate) fn of_dir(src: &Path) -> Copy {
        let dir = TempDir::new().expect("tempdir");
        copy_dir(src, &dir.path().join("c"));
        fs::create_dir_all(dir.path().join("home/data")).expect("mkdir");
        fs::create_dir_all(dir.path().join("run")).expect("mkdir");
        Copy { dir }
    }

    pub(crate) fn root(&self) -> PathBuf {
        self.dir.path().join("c")
    }

    pub(crate) fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    pub(crate) fn data(&self) -> PathBuf {
        self.home().join("data")
    }

    /// The scratch environment every spawned binary runs with.
    pub(crate) fn envs(&self) -> Vec<(&'static str, PathBuf)> {
        let home = self.home();
        vec![
            ("HOME", home.clone()),
            ("XDG_DATA_HOME", self.data()),
            ("XDG_CONFIG_HOME", home.join("config")),
            ("XDG_CACHE_HOME", home.join("cache")),
            ("XDG_STATE_HOME", home.join("state")),
            ("XDG_RUNTIME_DIR", self.dir.path().join("run")),
        ]
    }

    /// `apinata item --collection <root> <args>`.
    pub(crate) fn item(&self, args: &[&str]) -> assert_cmd::assert::Assert {
        let mut cmd = apinata();
        for (k, v) in self.envs() {
            cmd.env(k, v);
        }
        cmd.arg("item")
            .arg("--collection")
            .arg(self.root())
            .args(args)
            .current_dir(self.dir.path())
            .timeout(Duration::from_secs(30))
            .assert()
    }

    /// Call MCP tool `tool` with `args` over `apinata mcp`: the structured result, or the
    /// error text of an `isError` result.
    pub(crate) async fn mcp(&self, tool: &str, args: Value) -> Result<Value, String> {
        let Value::Object(args) = args else {
            panic!("tool arguments must be a JSON object")
        };
        let mut cmd = common::apinata_tokio();
        for (k, v) in self.envs() {
            cmd.env(k, v);
        }
        cmd.arg("mcp")
            .arg("--collection")
            .arg(self.root())
            .current_dir(self.dir.path())
            .kill_on_drop(true);
        let exchange = async {
            let client =
                ().serve(TokioChildProcess::new(cmd).expect("spawns"))
                    .await
                    .expect("handshake");
            let result = client
                .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(args))
                .await
                .expect("tools/call");
            client.cancel().await.expect("closes");
            result
        };
        let result = tokio::time::timeout(Duration::from_secs(30), exchange)
            .await
            .expect("the stdio exchange finishes within 30 s");
        if result.is_error == Some(true) {
            let text: Vec<String> = result
                .content
                .iter()
                .filter_map(|c| c.as_text().map(|t| t.text.clone()))
                .collect();
            return Err(text.join("\n"));
        }
        Ok(result.structured_content.expect("structured content"))
    }

    /// Every file under the collection, by relative path.
    pub(crate) fn tree(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        walk(&self.root(), &self.root(), &mut out);
        out
    }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("mkdir");
    for entry in fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy");
        }
    }
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
    for entry in fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            out.insert(rel(root, &path) + "/", String::new());
            walk(root, &path, out);
        } else {
            out.insert(
                rel(root, &path),
                fs::read_to_string(&path).expect("utf-8 file"),
            );
        }
    }
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("inside")
        .to_string_lossy()
        .into_owned()
}

/// The tree as one text, for the snapshot.
pub(crate) fn render(tree: &BTreeMap<String, String>) -> String {
    tree.iter()
        .map(|(path, text)| format!("== {path}\n{text}"))
        .collect()
}

pub(crate) fn stdout(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(a.get_output().stdout.clone()).expect("utf-8")
}

/// Run the CLI action on one copy and the MCP tool on another; the trees must match. Returns
/// the CLI's stdout, the MCP result and the tree.
async fn parity(
    cli: &[&str],
    tool: &str,
    args: Value,
) -> (String, Value, BTreeMap<String, String>) {
    parity_with(&[], cli, tool, args).await
}

/// `parity` over the fixture with `extra` files written into both copies first.
async fn parity_with(
    extra: &[(&str, &str)],
    cli: &[&str],
    tool: &str,
    args: Value,
) -> (String, Value, BTreeMap<String, String>) {
    let a = Copy::new();
    let b = Copy::new();
    for (rel, text) in extra {
        fs::write(a.root().join(rel), text).expect("write");
        fs::write(b.root().join(rel), text).expect("write");
    }
    let out = stdout(&a.item(cli).code(0));
    let result = b.mcp(tool, args).await.expect("the tool succeeds");
    let tree = a.tree();
    assert_eq!(tree, b.tree(), "cli {cli:?} and {tool} differ");
    (out, result, tree)
}

#[tokio::test]
async fn new_matches_create_request() {
    let (out, result, tree) = parity(
        &[
            "new",
            "Find",
            "--in",
            "users",
            "--method",
            "m-search",
            "--url",
            "http://localhost/find?q=1",
        ],
        "create_request",
        json!({"parent": "users", "name": "Find", "method": "m-search",
               "url": "http://localhost/find?q=1"}),
    )
    .await;
    assert_eq!(out, "created users/Find.yml\n");
    assert_eq!(result, json!({"path": "users/Find.yml"}));
    insta::assert_snapshot!("item_new", render(&tree));
}

#[tokio::test]
async fn new_graphql_at_the_root_matches_create_request() {
    let (out, _, _) = parity(
        &["new", "Query", "--kind", "graphql"],
        "create_request",
        json!({"parent": "", "name": "Query", "kind": "graphql"}),
    )
    .await;
    assert_eq!(out, "created Query.yml\n");
}

#[tokio::test]
async fn mkdir_matches_create_folder() {
    let (out, result, tree) = parity(
        &["mkdir", "Admin", "--in", "users"],
        "create_folder",
        json!({"parent": "users", "name": "Admin"}),
    )
    .await;
    assert_eq!(out, "created users/Admin\n");
    assert_eq!(result, json!({"path": "users/Admin"}));
    insta::assert_snapshot!("item_mkdir", render(&tree));
}

#[tokio::test]
async fn cp_matches_clone_request() {
    let (out, result, tree) = parity(
        &["cp", "users/get.yml", "Get copy"],
        "clone_request",
        json!({"path": "users/get.yml", "name": "Get copy"}),
    )
    .await;
    assert_eq!(
        out,
        format!(
            "copied users/get.yml to {}\n",
            result["path"].as_str().expect("path")
        )
    );
    insta::assert_snapshot!("item_cp", render(&tree));
}

#[tokio::test]
async fn cp_of_a_folder_matches_clone_folder() {
    let (out, result, tree) = parity(
        &["cp", "users", "users 2"],
        "clone_folder",
        json!({"path": "users", "name": "users 2"}),
    )
    .await;
    assert_eq!(result, json!({"path": "users 2"}));
    assert_eq!(out, "copied users to users 2\n");
    // The copy goes through the writer, which puts a blank line between sections; the content
    // is the source's.
    for child in ["users 2/get.yml", "users 2/list.yml"] {
        let source = child.replacen("users 2/", "users/", 1);
        let copy = tree[child].replace("\n\n", "\n");
        assert_eq!(copy, tree[&source], "{child}");
    }
    insta::assert_snapshot!("item_cp_folder", render(&tree));
}

#[tokio::test]
async fn cp_of_an_opaque_item_matches_clone_request() {
    let app = "info:\n  name: app\n  type: app\n  seq: 3\napp:\n  url: x\n";
    let (out, result, tree) = parity_with(
        &[("app.yml", app)],
        &["cp", "app.yml", "app 2"],
        "clone_request",
        json!({"path": "app.yml", "name": "app 2"}),
    )
    .await;
    assert_eq!(result, json!({"path": "app 2.yml"}));
    assert_eq!(out, "copied app.yml to app 2.yml\n");
    // The copy goes through the writer, which puts a blank line between sections.
    assert_eq!(
        tree["app 2.yml"].replace("\n\n", "\n"),
        app.replace("name: app\n", "name: app 2\n")
            .replace("seq: 3", "seq: 4")
    );
}

#[tokio::test]
async fn rename_matches_rename_item() {
    let (out, result, tree) = parity(
        &["rename", "users/get.yml", "Fetch user"],
        "rename_item",
        json!({"path": "users/get.yml", "name": "Fetch user"}),
    )
    .await;
    assert_eq!(result, json!({"path": "users/Fetch user.yml"}));
    assert_eq!(out, "renamed users/get.yml to users/Fetch user.yml\n");
    insta::assert_snapshot!("item_rename", render(&tree));
}

#[tokio::test]
async fn mv_into_matches_move_item_inside() {
    let (out, result, tree) = parity(
        &["mv", "a.yml", "--into", "users"],
        "move_item",
        json!({"path": "a.yml", "inside": "users"}),
    )
    .await;
    assert_eq!(result, json!({"path": "users/a.yml"}));
    assert_eq!(out, "moved a.yml to users/a.yml\n");
    insta::assert_snapshot!("item_mv_into", render(&tree));
}

#[tokio::test]
async fn mv_into_lands_past_a_hand_set_seq_in_the_folder() {
    let (_, _, tree) = parity_with(
        &[("users/zz.yml", &request("zz", 9, "http://localhost/zz"))],
        &["mv", "a.yml", "--into", "users"],
        "move_item",
        json!({"path": "a.yml", "inside": "users"}),
    )
    .await;
    assert!(
        tree["users/a.yml"].contains("  seq: 10\n"),
        "{}",
        tree["users/a.yml"]
    );
}

#[tokio::test]
async fn mv_into_dot_is_the_root() {
    let (out, _, _) = parity(
        &["mv", "users/list.yml", "--into", "."],
        "move_item",
        json!({"path": "users/list.yml", "inside": ""}),
    )
    .await;
    assert_eq!(out, "moved users/list.yml to list.yml\n");
}

#[tokio::test]
async fn mv_before_and_after_match_move_item() {
    let (out, _, tree) = parity(
        &["mv", "users/list.yml", "--before", "users/get.yml"],
        "move_item",
        json!({"path": "users/list.yml", "before": "users/get.yml"}),
    )
    .await;
    assert_eq!(out, "moved users/list.yml to users/list.yml\n");
    insta::assert_snapshot!("item_mv_before", render(&tree));
    parity(
        &["mv", "a.yml", "--after", "users"],
        "move_item",
        json!({"path": "a.yml", "after": "users"}),
    )
    .await;
}

#[tokio::test]
async fn rm_moves_the_item_to_the_trash_and_leaves_the_tree_delete_item_leaves() {
    let (out, result, tree) = parity(
        &["rm", "users/get.yml"],
        "delete_item",
        json!({"path": "users/get.yml"}),
    )
    .await;
    assert_eq!(out, "removed users/get.yml (moved to the trash)\n");
    assert_eq!(
        result,
        json!({"deleted": "users/get.yml", "destination": "trash"})
    );
    insta::assert_snapshot!("item_rm", render(&tree));
}

#[test]
fn rm_files_the_item_in_the_scratch_trash() {
    let a = Copy::new();
    a.item(&["rm", "users"])
        .code(0)
        .stdout("removed users (moved to the trash)\n");
    assert!(!a.root().join("users").exists());
    assert!(a.data().join("Trash/files/users/get.yml").is_file());
}

#[test]
fn json_output_matches_the_mcp_results() {
    let a = Copy::new();
    let json_of = |args: &[&str]| -> Value {
        let mut all = vec!["--format", "json"];
        all.extend_from_slice(args);
        serde_json::from_str(&stdout(&a.item(&all).code(0))).expect("json")
    };
    assert_eq!(json_of(&["new", "N"]), json!({"path": "N.yml"}));
    assert_eq!(json_of(&["mkdir", "F"]), json!({"path": "F"}));
    assert_eq!(json_of(&["cp", "a.yml", "A2"]), json!({"path": "A2.yml"}));
    assert_eq!(
        json_of(&["rename", "A2.yml", "A3"]),
        json!({"path": "A3.yml"})
    );
    assert_eq!(
        json_of(&["mv", "A3.yml", "--into", "F"]),
        json!({"path": "F/A3.yml"})
    );
    assert_eq!(
        json_of(&["rm", "F"]),
        json!({"removed": "F", "destination": "trash", "warnings": []})
    );
}

#[tokio::test]
async fn an_unknown_path_exits_3_with_the_mcp_message() {
    let a = Copy::new();
    let b = Copy::new();
    let message = b
        .mcp("rename_item", json!({"path": "nope.yml", "name": "x"}))
        .await
        .expect_err("refused");
    a.item(&["rename", "nope.yml", "x"])
        .code(3)
        .stdout("")
        .stderr(format!("error: {message}\n"));
    assert_eq!(a.tree(), Copy::new().tree());
}

#[test]
fn mv_with_two_targets_is_a_usage_error() {
    let a = Copy::new();
    a.item(&["mv", "a.yml", "--into", "users", "--after", "users"])
        .code(3)
        .stdout("")
        .stderr(contains("cannot be used with"));
    a.item(&["mv", "a.yml"]).code(3).stdout("");
    assert_eq!(a.tree(), Copy::new().tree());
}

#[test]
fn rm_with_no_trash_exits_3_and_removes_nothing() {
    let a = Copy::new();
    // A data directory that is a file leaves no place for the trash.
    fs::remove_dir_all(a.data()).expect("rm");
    fs::write(a.data(), "").expect("write");
    a.item(&["rm", "a.yml"])
        .code(3)
        .stdout("")
        .stderr(contains("error: could not move it to the trash"));
    assert!(a.root().join("a.yml").is_file());
}

#[test]
fn help_lists_the_actions_and_exit_codes() {
    apinata()
        .args(["item", "--help"])
        .assert()
        .code(0)
        .stdout(contains("Exit codes:"))
        .stdout(contains("mkdir"))
        .stdout(contains("rename"));
}

/// Makes a directory read-only and gives it its mode back when dropped, even after a panic.
#[cfg(unix)]
struct ReadOnly(PathBuf);

#[cfg(unix)]
impl ReadOnly {
    fn set(dir: PathBuf) -> ReadOnly {
        use std::os::unix::fs::PermissionsExt as _;
        let guard = ReadOnly(dir);
        fs::set_permissions(&guard.0, fs::Permissions::from_mode(0o555)).expect("chmod");
        guard
    }
}

#[cfg(unix)]
impl Drop for ReadOnly {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}

#[cfg(unix)]
#[test]
fn rm_warns_when_a_sibling_cannot_be_renumbered_and_still_exits_0() {
    let text = Copy::new();
    let json = Copy::new();
    let _locks = [
        ReadOnly::set(text.root().join("users")),
        ReadOnly::set(json.root().join("users")),
    ];
    // Permissions are not enforced for root, so the failure cannot be staged there.
    if fs::write(text.root().join("users/probe"), "").is_ok() {
        return;
    }
    let out = text.item(&["rm", "a.yml"]).code(0);
    assert_eq!(stdout(&out), "removed a.yml (moved to the trash)\n");
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf-8");
    assert!(
        stderr.starts_with("warning: deleted, but could not renumber the siblings: "),
        "{stderr}"
    );
    assert!(stderr.contains("users/folder.yml"), "{stderr}");
    assert!(
        !stderr.contains(&text.root().display().to_string()),
        "{stderr}"
    );
    assert!(!text.root().join("a.yml").exists());

    let out = json.item(&["--format", "json", "rm", "a.yml"]).code(0);
    let doc: Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert_eq!(
        (&doc["removed"], &doc["destination"]),
        (&json!("a.yml"), &json!("trash"))
    );
    let warnings = doc["warnings"].as_array().expect("warnings");
    assert_eq!(warnings.len(), 1, "{doc}");
    assert!(
        warnings[0]
            .as_str()
            .expect("text")
            .starts_with("deleted, but could not renumber the siblings: "),
        "{doc}"
    );
}
