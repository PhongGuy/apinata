//! Apinata driven end to end over MCP: a `gpui-pilot` server in a scratch project launches
//! this package's binary headless on a fixture collection.

use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::process::Command;

/// The server binary: `GPUI_PILOT_BIN`, else `gpui-pilot` on `PATH`.
fn server_bin() -> PathBuf {
    if let Some(bin) = std::env::var_os("GPUI_PILOT_BIN") {
        return PathBuf::from(bin);
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("gpui-pilot"))
        .find(|bin| bin.is_file())
        .expect("no gpui-pilot server: install it (see gpui-pilot.toml) or set GPUI_PILOT_BIN")
}

/// A collection with one folder and one request at the root.
fn fixture() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        (
            "opencollection.yml",
            "opencollection: 1.0.0\ninfo:\n  name: Fixture\n",
        ),
        (
            "users/folder.yml",
            "info:\n  name: Users\n  type: folder\n  seq: 1\n",
        ),
        (
            "health.yml",
            "info:\n  name: Health\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/health\"\n",
        ),
    ];
    for (rel, text) in files {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

struct Pilot {
    client: RunningService<RoleClient, ()>,
    _root: TempDir,
    temp: TempDir,
    /// Stand-in session bus; nothing under the pilot may connect to it.
    bus: UnixListener,
}

impl Pilot {
    /// Starts the server in a scratch project whose config launches this package's binary,
    /// with a cleared environment that points the session bus and displays at canaries.
    async fn start() -> Pilot {
        let (root, temp) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        // bwrap binds the session binary at its own path, so the path must be free of `..`;
        // until the app is built only the raw path exists, and only the session test needs it.
        let app = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/debug/apinata-app"
        );
        let app = std::fs::canonicalize(app).unwrap_or_else(|_| PathBuf::from(app));
        let config = format!(
            "[headless]\nbinary = {:?}\n\n[session]\nbinary = {:?}\n",
            env!("CARGO_BIN_EXE_apinata-pilot"),
            app
        );
        std::fs::write(root.path().join("gpui-pilot.toml"), config).unwrap();
        let bus_path = temp.path().join("session_bus");
        let bus = UnixListener::bind(&bus_path).unwrap();
        bus.set_nonblocking(true).unwrap();
        let mut command = Command::new(server_bin());
        command
            .arg("mcp")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", root.path())
            .env("TMPDIR", temp.path())
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", bus_path.display()),
            )
            .env("DISPLAY", ":99")
            .env("WAYLAND_DISPLAY", "gpui-pilot-canary")
            .current_dir(root.path());
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        Pilot {
            client,
            _root: root,
            temp,
            bus,
        }
    }

    /// Calls `tool`, requires success, and returns the whole result.
    async fn call(&self, tool: &str, args: Value) -> CallToolResult {
        let Value::Object(args) = args else {
            panic!("arguments must be an object")
        };
        let result = self
            .client
            .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(args))
            .await
            .unwrap_or_else(|e| panic!("{tool}: {e}"));
        assert_ne!(
            result.is_error,
            Some(true),
            "{tool} failed: {:?}",
            result.structured_content
        );
        result
    }

    async fn ok(&self, tool: &str, args: Value) -> Value {
        self.call(tool, args)
            .await
            .structured_content
            .expect("structured content")
    }

    fn assert_bus_untouched(&self) {
        match self.bus.accept() {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            other => panic!("something connected to the session bus: {other:?}"),
        }
    }
}

/// The last path segment of every node in a `tree` result.
fn last_segments(tree: &Value) -> Vec<String> {
    tree["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| match n["path"].as_array().unwrap().last().unwrap() {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect()
}

fn node<'a>(tree: &'a Value, last: &str) -> &'a Value {
    tree["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["path"].as_array().unwrap().last() == Some(&json!(last)))
        .unwrap_or_else(|| panic!("no node ending in {last}"))
}

/// The app's environment for a launch: git reads no system or global config and stops
/// discovery at `ceiling`, so a piloted run never sees the developer's git setup.
fn git_env(ceiling: &Path) -> Value {
    json!({
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_CONFIG_SYSTEM": "/dev/null",
        "GIT_CEILING_DIRECTORIES": ceiling,
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sidebar_click_opens_the_request_in_a_tab() {
    let collection = fixture();
    let pilot = Pilot::start().await;
    let launch = pilot
        .ok(
            "launch",
            json!({
                "mode": "headless",
                "build": false,
                "args": [collection.path()],
                "env": git_env(collection.path().parent().unwrap()),
            }),
        )
        .await;
    assert_eq!(
        launch["window"],
        json!({ "w": 1280.0, "h": 800.0, "scale": 1.0 })
    );
    let data_dir = PathBuf::from(launch["data_dir"].as_str().unwrap());
    assert!(
        data_dir.starts_with(pilot.temp.path()),
        "{}",
        data_dir.display()
    );
    assert!(data_dir.join("session.json").is_file());

    let shot = pilot.call("screenshot", json!({})).await;
    assert_eq!(shot.structured_content.unwrap()["w"], json!(1280));
    let image = shot
        .content
        .iter()
        .find_map(|c| c.as_image())
        .expect("an image block");
    assert_eq!(image.mime_type, "image/png");

    let before = pilot
        .ok("tree", json!({ "wait_for": { "id": ["0:health.yml"] } }))
        .await;
    let names = last_segments(&before);
    assert!(names.contains(&"tab-empty".to_owned()), "{names:?}");
    assert!(!names.contains(&"tab-0".to_owned()), "{names:?}");

    let clicked = pilot
        .ok("click", json!({ "target": { "id": ["0:health.yml"] } }))
        .await;
    assert_eq!(clicked["settled"], json!(true));

    let after = pilot
        .ok("tree", json!({ "wait_for": { "id": ["tab-0"] } }))
        .await;
    let names = last_segments(&after);
    assert!(names.contains(&"preview-0".to_owned()), "{names:?}");
    assert!(!names.contains(&"tab-empty".to_owned()), "{names:?}");
    assert_eq!(
        node(&after, "url")["value"],
        json!("http://127.0.0.1:9/health")
    );

    let closed = pilot.ok("close", json!({})).await;
    assert_eq!(closed["exit_code"], json!(0));
    assert!(!data_dir.exists());
    pilot.assert_bus_untouched();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relative_collection_path_fails_the_launch() {
    let pilot = Pilot::start().await;
    let result = pilot
        .client
        .call_tool(
            CallToolRequestParams::new("launch".to_owned()).with_arguments(
                json!({ "mode": "headless", "build": false, "args": ["collections/demo"], "env": git_env(pilot.temp.path()) })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let error = result.structured_content.unwrap();
    assert_eq!(error["code"], json!("LaunchFailed"));
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("must be absolute"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_real_app_starts_in_a_private_session() {
    if std::env::var_os("GPUI_PILOT_SESSION_TESTS").as_deref() != Some("1".as_ref()) {
        eprintln!(
            "skipped: set GPUI_PILOT_SESSION_TESTS=1 to run the session smoke (needs bwrap, kwin_wayland and a GPU)"
        );
        return;
    }
    let app = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/debug/apinata-app"
    ));
    assert!(
        app.exists(),
        "run `cargo build -p apinata-app --bin apinata-app` in the repository root first"
    );
    let pilot = Pilot::start().await;
    let launched = pilot
        .ok(
            "launch",
            json!({ "mode": "session", "build": false, "width": 1280, "height": 800, "env": git_env(pilot.temp.path()) }),
        )
        .await;
    assert_eq!(launched["mode"], json!("session"));
    // A fresh private HOME opens no collection, so the tab area shows the no-collection view.
    let tree = pilot
        .ok("tree", json!({ "wait_for": { "id": ["no-collection"] } }))
        .await;
    let ids: Vec<&str> = tree["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["id"].as_str())
        .collect();
    assert!(ids.contains(&"no-collection"), "{ids:?}");
    // Open a collection through the real folder dialog: the host-side fixture is outside the
    // sandbox, so the server hands the app a read-only copy in the launch's pick staging.
    let collection = fixture();
    pilot.ok("key", json!({ "keys": "ctrl-o" })).await;
    let picked = pilot
        .ok("pick_file", json!({ "paths": [collection.path()] }))
        .await;
    assert_eq!(picked["answered"], json!("open"));
    let staged = picked["paths"][0].as_str().unwrap().to_owned();
    let name = collection.path().file_name().unwrap().to_str().unwrap();
    assert!(
        staged.ends_with(&format!("/in/1/{name}")),
        "a read-only copy inside the sandbox: {staged}"
    );
    // Sidebar row ids do not reach AT-SPI, so the opened request is found by role and name.
    pilot
        .ok(
            "tree",
            json!({ "wait_for": { "role": "TreeItem", "name": "Health" } }),
        )
        .await;
    let shot = pilot.call("screenshot", json!({})).await;
    assert_eq!(shot.structured_content.unwrap()["w"], json!(1280));
    pilot.ok("close", json!({})).await;
    pilot.assert_bus_untouched();
}

/// Launches the app headless on `collection`, as every popover test below does.
async fn launch_on(pilot: &Pilot, collection: &TempDir) {
    pilot
        .ok(
            "launch",
            json!({
                "mode": "headless",
                "build": false,
                "args": [collection.path()],
                "env": git_env(collection.path().parent().unwrap()),
            }),
        )
        .await;
}

/// Opens the collection switcher by clicking its trigger and returns the tree with it open.
async fn open_switcher(pilot: &Pilot) -> Value {
    pilot
        .ok(
            "click",
            json!({ "target": { "id": ["collection-switcher"] } }),
        )
        .await;
    pilot
        .ok(
            "tree",
            json!({ "wait_for": { "id": ["switcher-open-collection"] } }),
        )
        .await
}

fn contains(bounds: &Value, (x, y): (f64, f64)) -> bool {
    let b = |k: &str| bounds[k].as_f64().unwrap();
    x >= b("x") && x < b("x") + b("w") && y >= b("y") && y < b("y") + b("h")
}

fn centre(bounds: &Value) -> (f64, f64) {
    let b = |k: &str| bounds[k].as_f64().unwrap();
    (b("x") + b("w") / 2.0, b("y") + b("h") / 2.0)
}

/// Kit Buttons fire on the key-up of Enter, so this pins that `key` sends a full press. The
/// title bar's environments button is the fifth tab stop, after the switcher (whose popover
/// binds Enter on key-down), the two environment selects and the variables button.
#[tokio::test(flavor = "multi_thread")]
async fn enter_on_a_focused_kit_button_presses_it() {
    let collection = fixture();
    let pilot = Pilot::start().await;
    launch_on(&pilot, &collection).await;
    pilot
        .ok("key", json!({ "keys": "tab tab tab tab tab" }))
        .await;
    let focused = last_segments(&pilot.ok("tree", json!({})).await);
    assert!(!focused.contains(&"env-editor".to_owned()), "{focused:?}");

    pilot.ok("key", json!({ "keys": "enter" })).await;
    let pressed = pilot
        .ok(
            "tree",
            json!({ "wait_for": { "id": ["env-editor"] }, "timeout_ms": 2000 }),
        )
        .await;
    assert!(last_segments(&pressed).contains(&"tab-0".to_owned()));
    pilot.ok("close", json!({})).await;
    pilot.assert_bus_untouched();
}

/// The open switcher's rows are listed under the popover, drawn over the sidebar.
#[tokio::test(flavor = "multi_thread")]
async fn the_open_collection_switcher_lists_its_rows() {
    let collection = fixture();
    let pilot = Pilot::start().await;
    launch_on(&pilot, &collection).await;
    let tree = open_switcher(&pilot).await;
    let names = last_segments(&tree);
    for id in [
        "switcher-open-collection",
        "switcher-import-collection",
        "switcher-new-collection",
        "switch-open-0",
    ] {
        let row = node(&tree, id);
        assert!(
            row["path"].as_array().unwrap().contains(&json!("switcher")),
            "{id} is not under the popover: {row}"
        );
        assert_eq!(row["visible"], json!(true), "{row}");
        assert!(
            contains(&node(&tree, "sidebar")["bounds"], centre(&row["bounds"])),
            "{id} is not over the sidebar: {row}"
        );
    }
    assert!(
        names.contains(&"switcher-settings-0".to_owned()),
        "{names:?}"
    );
    pilot.ok("close", json!({})).await;
}

/// A point inside an open popover's button hovers that button, not the sidebar row the
/// popover is drawn over.
#[tokio::test(flavor = "multi_thread")]
async fn hovering_a_point_in_the_switcher_hits_its_button() {
    let collection = fixture();
    // Enough extra requests that sidebar rows fill the area below the switcher however tall it is.
    for n in 0..8 {
        std::fs::write(
            collection.path().join(format!("extra{n}.yml")),
            format!("info:\n  name: Extra {n}\n  type: http\n  seq: {}\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/{n}\"\n", n + 2),
        )
        .unwrap();
    }
    let pilot = Pilot::start().await;
    launch_on(&pilot, &collection).await;
    let tree = open_switcher(&pilot).await;
    let (x, y) = centre(&node(&tree, "switch-open-0")["bounds"]);
    // Some sidebar request row lies under the button, so a hit on the sidebar would show.
    assert!(
        tree["nodes"].as_array().unwrap().iter().any(|n| {
            let last = n["path"].as_array().unwrap().last().and_then(Value::as_str);
            last.is_some_and(|l| l.contains(".yml")) && contains(&n["bounds"], (x, y))
        }),
        "the fixture no longer puts a sidebar row under the switcher"
    );

    let hovered = pilot
        .ok("hover", json!({ "target": { "x": x, "y": y } }))
        .await;
    for key in ["hovered", "hit"] {
        let last = hovered[key]["path"].as_array().unwrap().last().unwrap();
        assert_eq!(last, &json!("switch-open-0"), "{key}: {hovered}");
    }
    pilot.ok("close", json!({})).await;
}

/// A right-click on a request row and on a folder row each opens the row's context menu, and
/// the app outlives both: the host must not crash when the menu takes focus mid-frame.
#[tokio::test(flavor = "multi_thread")]
async fn right_clicking_sidebar_rows_opens_their_menus() {
    let collection = fixture();
    let pilot = Pilot::start().await;
    launch_on(&pilot, &collection).await;
    pilot
        .ok("tree", json!({ "wait_for": { "id": ["0:users"] } }))
        .await;
    for row in ["0:health.yml", "0:users"] {
        pilot
            .ok(
                "click",
                json!({ "target": { "id": [row] }, "button": "right" }),
            )
            .await;
        let open = pilot
            .ok("tree", json!({ "wait_for": { "id": ["popup-menu"] } }))
            .await;
        assert!(
            last_segments(&open).contains(&"popup-menu".to_owned()),
            "no menu for {row}"
        );
        pilot.ok("key", json!({ "keys": "escape" })).await;
        pilot
            .ok(
                "wait",
                json!({ "target": { "id": ["popup-menu"] }, "gone": true }),
            )
            .await;
    }
    // A later call still reaches a live app rather than an AppCrashed error.
    let shot = pilot.call("screenshot", json!({})).await;
    assert_eq!(shot.structured_content.unwrap()["w"], json!(1280));
    let closed = pilot.ok("close", json!({})).await;
    assert_eq!(closed["exit_code"], json!(0));
    pilot.assert_bus_untouched();
}

/// A tab's menu offers Pin tab, the palette's Pin tab pins the active tab, and the menu then
/// offers Unpin tab.
#[tokio::test(flavor = "multi_thread")]
async fn pinning_a_tab_from_the_palette_flips_its_menu_entry() {
    let collection = fixture();
    let pilot = Pilot::start().await;
    launch_on(&pilot, &collection).await;
    pilot
        .ok("click", json!({ "target": { "id": ["0:health.yml"] } }))
        .await;
    pilot
        .ok("tree", json!({ "wait_for": { "id": ["tab-0"] } }))
        .await;
    let mut menus = Vec::new();
    for pin_first in [false, true] {
        if pin_first {
            pilot.ok("key", json!({ "keys": "ctrl-shift-p" })).await;
            pilot
                .ok("tree", json!({ "wait_for": { "id": ["command-palette"] } }))
                .await;
            pilot.ok("type", json!({ "text": "Pin tab" })).await;
            pilot.ok("key", json!({ "keys": "enter" })).await;
        }
        pilot
            .ok(
                "click",
                json!({ "target": { "id": ["tab-0"] }, "button": "right" }),
            )
            .await;
        menus.push(
            pilot
                .ok("tree", json!({ "wait_for": { "id": ["popup-menu"] } }))
                .await
                .to_string(),
        );
        pilot.ok("key", json!({ "keys": "escape" })).await;
        pilot
            .ok(
                "wait",
                json!({ "target": { "id": ["popup-menu"] }, "gone": true }),
            )
            .await;
    }
    assert!(menus[0].contains("Pin tab") && !menus[0].contains("Unpin tab"));
    assert!(menus[1].contains("Unpin tab"), "{}", menus[1]);
    let closed = pilot.ok("close", json!({})).await;
    assert_eq!(closed["exit_code"], json!(0));
    pilot.assert_bus_untouched();
}

/// The palette's Expand all and Collapse all open and close the folders, and the collection
/// row's menu offers both entries.
#[tokio::test(flavor = "multi_thread")]
async fn expand_all_and_collapse_all_reach_the_sidebar() {
    let collection = fixture();
    std::fs::write(
        collection.path().join("users/list.yml"),
        "info:\n  name: List\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/users\"\n",
    )
    .unwrap();
    let pilot = Pilot::start().await;
    launch_on(&pilot, &collection).await;
    pilot
        .ok("tree", json!({ "wait_for": { "id": ["0:users"] } }))
        .await;
    // The collection row's menu lists both entries for its own collection.
    pilot
        .ok(
            "click",
            json!({ "target": { "id": ["0:"] }, "button": "right" }),
        )
        .await;
    let menu = pilot
        .ok("tree", json!({ "wait_for": { "id": ["popup-menu"] } }))
        .await;
    for entry in ["Collapse all", "Expand all"] {
        assert!(menu.to_string().contains(entry), "no {entry} entry: {menu}");
    }
    pilot.ok("key", json!({ "keys": "escape" })).await;
    pilot
        .ok(
            "wait",
            json!({ "target": { "id": ["popup-menu"] }, "gone": true }),
        )
        .await;
    for (command, row, gone) in [
        ("Expand all", "0:users/list.yml", false),
        ("Collapse all", "0:users/list.yml", true),
    ] {
        pilot.ok("key", json!({ "keys": "ctrl-shift-p" })).await;
        pilot
            .ok("tree", json!({ "wait_for": { "id": ["command-palette"] } }))
            .await;
        pilot.ok("type", json!({ "text": command })).await;
        pilot.ok("key", json!({ "keys": "enter" })).await;
        if gone {
            pilot
                .ok("wait", json!({ "target": { "id": [row] }, "gone": true }))
                .await;
        } else {
            pilot
                .ok("tree", json!({ "wait_for": { "id": [row] } }))
                .await;
        }
    }
    let closed = pilot.ok("close", json!({})).await;
    assert_eq!(closed["exit_code"], json!(0));
    pilot.assert_bus_untouched();
}

/// The palette lists the Compare command in the real app.
#[tokio::test(flavor = "multi_thread")]
async fn the_palette_offers_compare_response() {
    let collection = fixture();
    let pilot = Pilot::start().await;
    launch_on(&pilot, &collection).await;
    pilot
        .ok("tree", json!({ "wait_for": { "id": ["0:health.yml"] } }))
        .await;
    pilot.ok("key", json!({ "keys": "ctrl-shift-p" })).await;
    pilot
        .ok("tree", json!({ "wait_for": { "id": ["command-palette"] } }))
        .await;
    pilot
        .ok("type", json!({ "text": "Compare response" }))
        .await;
    let tree = pilot.ok("tree", json!({})).await;
    assert!(
        tree.to_string().contains("Compare response"),
        "palette rows: {tree}"
    );
    pilot.ok("key", json!({ "keys": "escape" })).await;
    pilot
        .ok(
            "wait",
            json!({ "target": { "id": ["command-palette"] }, "gone": true }),
        )
        .await;
    let closed = pilot.ok("close", json!({})).await;
    assert_eq!(closed["exit_code"], json!(0));
    pilot.assert_bus_untouched();
}

/// Deleting a request moves it into the launch's folder trash, and the toast's Undo brings
/// the very same bytes back to the same path.
#[tokio::test(flavor = "multi_thread")]
async fn undo_after_a_delete_restores_the_request_file() {
    let collection = fixture();
    let file = collection.path().join("health.yml");
    let original = std::fs::read(&file).unwrap();
    let pilot = Pilot::start().await;
    launch_on(&pilot, &collection).await;
    pilot
        .ok("click", json!({ "target": { "id": ["0:health.yml"] } }))
        .await;
    pilot.ok("key", json!({ "keys": "delete" })).await;
    // With a trash and nothing unsaved the delete needs no prompt: the toast is the safety net.
    let toast = pilot
        .ok("tree", json!({ "wait_for": { "id": ["undo-delete"] } }))
        .await;
    assert!(!file.exists(), "the delete left the file in place");
    // Clicked by position: an id target times out on the toast button although the tree lists it.
    let (x, y) = centre(&node(&toast, "undo-delete")["bounds"]);
    pilot
        .ok("click", json!({ "target": { "x": x, "y": y } }))
        .await;
    pilot
        .ok("tree", json!({ "wait_for": { "id": ["0:health.yml"] } }))
        .await;
    assert_eq!(std::fs::read(&file).unwrap(), original);
    pilot.ok("close", json!({})).await;
    pilot.assert_bus_untouched();
}

/// A one-shot-per-connection 127.0.0.1 server: each connection reads the request, waits
/// `delay`, then writes `reply` (nothing, if it is empty) and closes. Returns its port.
fn mock_server(delay: std::time::Duration, reply: Vec<u8>) -> u16 {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let reply = std::sync::Arc::new(reply);
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let reply = reply.clone();
            std::thread::spawn(move || {
                let _ = stream.read(&mut [0u8; 8192]);
                std::thread::sleep(delay);
                let _ = stream.write_all(&reply);
            });
        }
    });
    port
}

/// The app's own change shows with no input: a link import against a server that hangs up
/// after a delay reports its error while the test only reads the tree.
#[tokio::test(flavor = "multi_thread")]
async fn a_delayed_import_failure_shows_without_input() {
    let collection = fixture();
    let pilot = Pilot::start().await;
    launch_on(&pilot, &collection).await;
    let delay = std::time::Duration::from_millis(1500);
    let port = mock_server(delay, Vec::new());
    pilot.ok("key", json!({ "keys": "ctrl-shift-i" })).await;
    pilot
        .ok(
            "tree",
            json!({ "wait_for": { "id_suffix": "import-link-import" } }),
        )
        .await;
    pilot
        .ok(
            "type",
            json!({ "text": format!("http://127.0.0.1:{port}/spec.json") }),
        )
        .await;
    let started = std::time::Instant::now();
    pilot
        .ok(
            "click",
            json!({ "target": { "id_suffix": "import-link-import" } }),
        )
        .await;
    // Only zero-settle reads from here on, so nothing but the app's own work can redraw.
    let mut polls = Vec::new();
    let shown = loop {
        let tree = pilot.ok("tree", json!({ "settle_ms": 0 })).await;
        let error = last_segments(&tree).contains(&"import-link-error".to_owned());
        polls.push((started.elapsed(), error));
        if error {
            break started.elapsed();
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(15),
            "no error without input: {polls:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    };
    assert!(
        shown >= delay,
        "the error came before the hang-up: {polls:?}"
    );
    assert!(
        !polls[0].1,
        "the first read already showed the error: {polls:?}"
    );
    pilot.ok("close", json!({})).await;
    pilot.assert_bus_untouched();
}

fn http_ok(content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    reply.extend_from_slice(body);
    reply
}

/// A click into the response editor holding the largest body it takes returns a bounded
/// reply: the editor's value is cut, not sent whole (gpui-pilot #9). Bigger bodies go to the
/// rows list, whose nodes carry no value.
#[tokio::test(flavor = "multi_thread")]
async fn clicking_a_large_response_editor_returns_a_bounded_value() {
    let collection = fixture();
    let body: String = (0..6_500)
        .map(|n| format!("line {n:06} of a large response body\n"))
        .collect();
    assert!(body.len() > 200_000 && body.len() < 262_144);
    let port = mock_server(
        std::time::Duration::ZERO,
        http_ok("text/plain", body.as_bytes()),
    );
    std::fs::write(
        collection.path().join("big.yml"),
        format!("info:\n  name: Big\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:{port}/big\"\n"),
    )
    .unwrap();
    let pilot = Pilot::start().await;
    launch_on(&pilot, &collection).await;
    pilot
        .ok("click", json!({ "target": { "id": ["0:big.yml"] } }))
        .await;
    pilot
        .ok("tree", json!({ "wait_for": { "id": ["send"] } }))
        .await;
    pilot
        .ok("click", json!({ "target": { "id": ["send"] } }))
        .await;
    pilot
        .ok(
            "tree",
            json!({ "wait_for": { "id": ["body-editor"] }, "timeout_ms": 20000 }),
        )
        .await;
    let clicked = pilot
        .call(
            "click",
            json!({ "target": { "id": ["body-editor"] }, "timeout_ms": 20000 }),
        )
        .await;
    let reply = clicked.structured_content.expect("structured content");
    let size = reply.to_string().len();
    assert!(size < 64 * 1024, "the click reply is {size} bytes");
    let cut: Vec<&Value> = ["clicked", "hit"]
        .iter()
        .map(|k| &reply[*k])
        .filter(|n| n["value_cut"] == json!(true))
        .collect();
    assert!(!cut.is_empty(), "no cut value in {reply}");
    for node in cut {
        assert!(node["value_chars"].as_u64().unwrap() >= body.len() as u64 - 10);
        assert!(node["value"].as_str().unwrap().chars().count() < 1000);
    }
    pilot.ok("close", json!({})).await;
    pilot.assert_bus_untouched();
}

const SPEC: &str = r#"{"openapi":"3.0.0","info":{"title":"Dropped","version":"1"},"paths":{"/a":{"get":{"responses":{"200":{"description":"ok"}}}}}}"#;

/// Dropping a collection folder on the window opens it, and dropping a spec file asks where
/// to import it.
#[tokio::test(flavor = "multi_thread")]
async fn dropping_a_folder_and_a_spec_file_opens_and_prompts() {
    let collection = fixture();
    let dropped = fixture();
    std::fs::rename(
        dropped.path().join("health.yml"),
        dropped.path().join("dropped.yml"),
    )
    .unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let spec = scratch.path().join("spec.json");
    std::fs::write(&spec, SPEC).unwrap();
    let pilot = Pilot::start().await;
    launch_on(&pilot, &collection).await;
    pilot
        .ok("tree", json!({ "wait_for": { "id": ["0:health.yml"] } }))
        .await;

    let reply = pilot
        .ok(
            "drop",
            json!({ "target": { "id": ["tab-area"] }, "paths": [dropped.path()] }),
        )
        .await;
    assert_eq!(reply["dropped"], json!(true), "{reply}");
    pilot
        .ok(
            "tree",
            json!({ "wait_for": { "id": ["1:dropped.yml"] }, "timeout_ms": 10000 }),
        )
        .await;

    pilot
        .ok(
            "drop",
            json!({ "target": { "id": ["tab-area"] }, "paths": [spec] }),
        )
        .await;
    let started = std::time::Instant::now();
    let prompt = loop {
        let tree = pilot.ok("tree", json!({})).await;
        if !tree["prompt"].is_null() {
            break tree["prompt"].clone();
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "no import prompt: {tree}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    };
    assert_eq!(prompt["kind"], json!("open"), "{prompt}");
    pilot.ok("pick_file", json!({ "cancel": true })).await;
    pilot.ok("close", json!({})).await;
    pilot.assert_bus_untouched();
}

/// Whether any node's path ends in the given id.
fn has_node(tree: &Value, last: &str) -> bool {
    tree["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["path"].as_array().unwrap().last() == Some(&json!(last)))
}

/// The Tests editor's text from the input node under its id; panics when the node is missing,
/// so an empty-string assertion can never pass on an absent editor.
fn editor_text(tree: &Value) -> &str {
    tree["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| {
            n["path"]
                .as_array()
                .unwrap()
                .contains(&json!("script-tests-code"))
        })
        .find_map(|n| n["value"].as_str())
        .expect("no script-tests-code input node with a value in the tree")
}

/// A Tests snippet button inserts its code at the caret and the editor's own undo takes it
/// back out, leaving the request clean-looking text again.
#[tokio::test(flavor = "multi_thread")]
async fn a_tests_snippet_inserts_and_undoes() {
    let collection = fixture();
    let pilot = Pilot::start().await;
    launch_on(&pilot, &collection).await;
    pilot
        .ok("click", json!({ "target": { "id": ["0:health.yml"] } }))
        .await;
    // Tests is the eighth section tab.
    pilot
        .ok("tree", json!({ "wait_for": { "id": ["section-7"] } }))
        .await;
    pilot
        .ok("click", json!({ "target": { "id": ["section-7"] } }))
        .await;
    let before = pilot
        .ok(
            "tree",
            json!({ "wait_for": { "id": ["snippet-status-200"] } }),
        )
        .await;
    assert_eq!(editor_text(&before), "");
    assert!(!has_node(&before, "dirty-0"), "dirty before any edit");
    pilot
        .ok(
            "click",
            json!({ "target": { "id": ["snippet-status-200"] } }),
        )
        .await;
    let after = pilot
        .ok("tree", json!({ "wait_for": { "id": ["dirty-0"] } }))
        .await;
    let value = editor_text(&after);
    assert!(!value.is_empty(), "the click inserted nothing");
    assert!(
        value.contains("expect(res.getStatus()).to.equal(200);"),
        "{value}"
    );
    pilot.ok("key", json!({ "keys": "ctrl-z" })).await;
    let undone = pilot.ok("tree", json!({})).await;
    assert_eq!(editor_text(&undone), "", "ctrl-z did not reach the editor");
    assert!(!has_node(&undone, "dirty-0"), "undo left the tab dirty");
    pilot.ok("close", json!({})).await;
    pilot.assert_bus_untouched();
}

/// Escape on the approval prompt dismisses it, through the headless host's key delivery.
#[tokio::test(flavor = "multi_thread")]
async fn escape_on_the_approval_prompt_dismisses_it() {
    let collection = fixture();
    let pilot = Pilot::start().await;
    let launch = pilot
        .ok(
            "launch",
            json!({
                "mode": "headless",
                "build": false,
                "args": [collection.path()],
                "env": git_env(collection.path().parent().unwrap()),
            }),
        )
        .await;
    let data_dir = PathBuf::from(launch["data_dir"].as_str().unwrap());
    pilot
        .ok("tree", json!({ "wait_for": { "id": ["0:health.yml"] } }))
        .await;
    let inbox = apinata_core::ApprovalInbox::new(data_dir.join("approvals"));
    let store = apinata_core::TrustStore::empty(data_dir.join("trusted-hosts.json"));
    inbox
        .request(&store, collection.path(), "api.example.com", "health.yml")
        .unwrap();
    pilot
        .ok(
            "tree",
            json!({ "wait_for": { "id": ["approval-host"] }, "timeout_ms": 10000 }),
        )
        .await;
    pilot.ok("key", json!({ "keys": "escape" })).await;
    pilot
        .ok(
            "wait",
            json!({ "target": { "id": ["approval-host"] }, "gone": true, "timeout_ms": 5000 }),
        )
        .await;
    pilot.ok("close", json!({})).await;
    pilot.assert_bus_untouched();
}

/// "Open in browser" on an HTML response hands the headless stub an inert page: the opened
/// path is logged in the launch data dir, and the file there carries the policy and the
/// sandboxed frame with the body escaped inside it.
#[tokio::test(flavor = "multi_thread")]
async fn open_in_browser_writes_an_inert_page_into_the_data_dir() {
    let collection = fixture();
    let body = "<!doctype html><h1>Hi</h1><script>alert(1)</script>";
    let port = mock_server(
        std::time::Duration::ZERO,
        http_ok("text/html", body.as_bytes()),
    );
    std::fs::write(
        collection.path().join("page.yml"),
        format!("info:\n  name: Page\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:{port}/page\"\n"),
    )
    .unwrap();
    let pilot = Pilot::start().await;
    let launch = pilot
        .ok(
            "launch",
            json!({
                "mode": "headless",
                "build": false,
                "args": [collection.path()],
                "env": git_env(collection.path().parent().unwrap()),
            }),
        )
        .await;
    let data_dir = PathBuf::from(launch["data_dir"].as_str().unwrap());
    pilot
        .ok("click", json!({ "target": { "id": ["0:page.yml"] } }))
        .await;
    pilot
        .ok("tree", json!({ "wait_for": { "id": ["send"] } }))
        .await;
    pilot
        .ok("click", json!({ "target": { "id": ["send"] } }))
        .await;
    pilot
        .ok(
            "tree",
            json!({ "wait_for": { "id_suffix": "body-open" }, "timeout_ms": 20000 }),
        )
        .await;
    pilot
        .ok("click", json!({ "target": { "id_suffix": "body-open" } }))
        .await;
    // The page is written off the main thread, so poll the log until a whole line names a file.
    let log = data_dir.join("pilot-opened.txt");
    let started = std::time::Instant::now();
    let opened = loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if text.ends_with('\n') {
            break PathBuf::from(text.lines().last().unwrap());
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(15),
            "nothing logged in {}",
            log.display()
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    // The data dir goes on close, so the page is read first.
    let page = std::fs::read_to_string(&opened).unwrap();
    pilot.ok("close", json!({})).await;
    assert_eq!(opened.extension().and_then(|e| e.to_str()), Some("html"));
    assert!(
        opened.starts_with(data_dir.join("pilot-open")),
        "{} is outside the data dir",
        opened.display()
    );
    let head = format!(
        r#"<meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="{}""#,
        apinata_app::open_external::INERT_CSP
    );
    assert!(page.starts_with(&head), "{page}");
    assert!(page.contains(r#"<iframe sandbox="" "#), "{page}");
    assert!(!page.contains("<script>"), "{page}");
    assert!(
        page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "{page}"
    );
    pilot.assert_bus_untouched();
}

/// The redirect target whose Hop cell is cut at 1440 px wide and fits at 2400 px. Both widths
/// stay clear of 1280 px, where the response header row starts to wrap and moves the table.
const LONG_HOP: &str = "/next/a-rather-long-path-segment/that-keeps-going/well-past-the-column/and-on-past-the-wider-column/until-the-end";

/// A 127.0.0.1 server for the Cookies tab: `/start` sets a cookie and redirects to
/// [`LONG_HOP`], which sets another. One request per connection. Returns its port.
fn redirect_server() -> u16 {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut head = [0u8; 8192];
            let n = stream.read(&mut head).unwrap_or(0);
            let reply = if head[..n].starts_with(b"GET /start ") {
                format!("HTTP/1.1 302 Found\r\nSet-Cookie: a=1; Path=/\r\nLocation: {LONG_HOP}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
            } else {
                let mut reply = http_ok("application/json", br#"{"ok":true}"#);
                let at = reply.windows(2).position(|w| w == b"\r\n").unwrap() + 2;
                reply.splice(at..at, b"Set-Cookie: b=2; Path=/\r\n".iter().copied());
                reply
            };
            let _ = stream.write_all(&reply);
        }
    });
    port
}

/// Launches at 1440 x 800, sends a request that redirects once, opens the Cookies tab and
/// hovers the long redirect target's cut Hop cell, asserting its tooltip shows. Returns the
/// hovered point.
async fn hover_a_cut_hop_cell(pilot: &Pilot, collection: &TempDir) -> (f64, f64) {
    let port = redirect_server();
    std::fs::write(
        collection.path().join("cookies.yml"),
        format!("info:\n  name: Cookies\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:{port}/start\"\n"),
    )
    .unwrap();
    pilot
        .ok(
            "launch",
            json!({
                "mode": "headless",
                "build": false,
                "width": 1440,
                "height": 800,
                "args": [collection.path()],
                "env": git_env(collection.path().parent().unwrap()),
            }),
        )
        .await;
    pilot
        .ok("click", json!({ "target": { "id": ["0:cookies.yml"] } }))
        .await;
    pilot
        .ok("tree", json!({ "wait_for": { "id": ["send"] } }))
        .await;
    pilot
        .ok("click", json!({ "target": { "id": ["send"] } }))
        .await;
    pilot
        .ok(
            "tree",
            json!({ "wait_for": { "id": ["response-tab-2"] }, "timeout_ms": 20000 }),
        )
        .await;
    pilot
        .ok("click", json!({ "target": { "id": ["response-tab-2"] } }))
        .await;
    let tree = pilot
        .ok(
            "tree",
            json!({ "wait_for": { "id": ["set-cookie-hop-1"] }, "timeout_ms": 10000 }),
        )
        .await;
    // The second row's Hop cell is the redirect target's; point just inside its left edge so
    // the pointer stays on the cell however wide the column grows or shrinks.
    let cell = &node(&tree, "set-cookie-hop-1")["bounds"];
    let at = (cell["x"].as_f64().unwrap() + 8.0, centre(cell).1);
    pilot
        .ok("hover", json!({ "target": { "x": at.0, "y": at.1 } }))
        .await;
    assert_eq!(
        cut_tooltip(pilot).await.as_deref(),
        Some(hop_text(port).as_str())
    );
    at
}

/// The Hop cell text of the redirect target.
fn hop_text(port: u16) -> String {
    format!("127.0.0.1:{port}{LONG_HOP}")
}

/// The text of the showing cut-cell tooltip, if one shows.
async fn cut_tooltip(pilot: &Pilot) -> Option<String> {
    let tree = pilot.ok("tree", json!({})).await;
    tree["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["path"].as_array().unwrap().last() == Some(&json!("kv-cut-tooltip")))
        .map(|n| n["name"].as_str().unwrap_or_default().to_owned())
}

/// A cut Hop cell's tooltip follows the window width under a still pointer: widening the
/// window until the text fits drops it, and narrowing it back shows it again.
#[tokio::test(flavor = "multi_thread")]
async fn resizing_the_window_toggles_a_cut_cell_tooltip() {
    let collection = fixture();
    let pilot = Pilot::start().await;
    let at = hover_a_cut_hop_cell(&pilot, &collection).await;
    let tip = cut_tooltip(&pilot).await;

    let wide = pilot
        .ok("resize", json!({ "width": 2400, "height": 800 }))
        .await;
    assert_eq!(wide["window"]["w"], json!(2400.0), "{wide}");
    // The pointer is still on the widened cell, so only the fitting text can drop the tooltip.
    let tree = pilot.ok("tree", json!({})).await;
    let cell = &node(&tree, "set-cookie-hop-1")["bounds"];
    assert!(contains(cell, at), "{at:?} left the cell {cell}");
    assert_eq!(cut_tooltip(&pilot).await, None, "the text fits at 2400 px");

    let back = pilot
        .ok("resize", json!({ "width": 1440, "height": 800 }))
        .await;
    assert_eq!(back["window"]["w"], json!(1440.0), "{back}");
    assert_eq!(cut_tooltip(&pilot).await, tip, "cut again at 1440 px");
    pilot.ok("close", json!({})).await;
    pilot.assert_bus_untouched();
}

/// Pressing only a modifier keeps the pointer's hover, so a showing tooltip stays, as on a
/// desktop where a bare modifier never enters keyboard modality.
#[tokio::test(flavor = "multi_thread")]
async fn a_bare_shift_keeps_a_showing_tooltip() {
    let collection = fixture();
    let pilot = Pilot::start().await;
    hover_a_cut_hop_cell(&pilot, &collection).await;
    let tip = cut_tooltip(&pilot).await;
    pilot.ok("key", json!({ "keys": "shift" })).await;
    assert_eq!(cut_tooltip(&pilot).await, tip, "shift dropped the tooltip");
    pilot.ok("close", json!({})).await;
    pilot.assert_bus_untouched();
}

/// The top edge of the node whose path ends in `last`.
fn top_of(tree: &Value, last: &str) -> f64 {
    node(tree, last)["bounds"]["y"].as_f64().unwrap()
}

/// The response header is one row even at its busiest, so a window resized across the width
/// the old row wrapped at leaves the body where it was.
#[tokio::test(flavor = "multi_thread")]
async fn resizing_across_1280_keeps_the_response_body_in_place() {
    let collection = fixture();
    let pilot = Pilot::start().await;
    let port = redirect_server();
    std::fs::write(
        collection.path().join("cookies.yml"),
        format!("info:\n  name: Cookies\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:{port}/start\"\n"),
    )
    .unwrap();
    pilot
        .ok(
            "launch",
            json!({
                "mode": "headless",
                "build": false,
                "width": 1280,
                "height": 800,
                "args": [collection.path()],
                "env": git_env(collection.path().parent().unwrap()),
            }),
        )
        .await;
    pilot
        .ok("click", json!({ "target": { "id": ["0:cookies.yml"] } }))
        .await;
    pilot
        .ok("tree", json!({ "wait_for": { "id": ["send"] } }))
        .await;
    pilot
        .ok("click", json!({ "target": { "id": ["send"] } }))
        .await;
    let tree = pilot
        .ok(
            "tree",
            json!({ "wait_for": { "id": ["response-body"] }, "timeout_ms": 20000 }),
        )
        .await;
    let body = top_of(&tree, "response-body");
    assert_eq!(
        top_of(&tree, "response-address"),
        top_of(&tree, "response-status"),
        "the address left the status's row"
    );
    for width in [1440, 1280] {
        let resized = pilot
            .ok(
                "resize",
                json!({ "width": width, "height": 800, "settle_ms": 300 }),
            )
            .await;
        assert_eq!(resized["window"]["w"], json!(f64::from(width)), "{resized}");
        let tree = pilot.ok("tree", json!({})).await;
        assert_eq!(
            top_of(&tree, "response-body"),
            body,
            "the body moved at {width} px"
        );
        assert_eq!(
            top_of(&tree, "response-address"),
            top_of(&tree, "response-status"),
            "the address left the status's row at {width} px"
        );
    }
    pilot.ok("close", json!({})).await;
    pilot.assert_bus_untouched();
}
