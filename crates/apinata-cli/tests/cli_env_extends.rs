//! `apinata env extends`: set or clear an environment's parent through core's validating setter,
//! the bytes the app writes, and the refusals a run would otherwise only warn about.

use crate::common;

use apinata_core::Collection;
use common::{ROOT, apinata, write_tree};
use serde_json::Value;
use std::path::Path;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const BASE: &str = "name: base\nvariables:\n  - name: host\n    value: base.test\n";
const DEV: &str = "name: dev\nvariables:\n  - name: port\n    value: \"8080\"\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/base.yml", BASE),
        ("environments/dev.yml", DEV),
    ])
}

fn read(dir: &Path, env: &str) -> String {
    std::fs::read_to_string(dir.join(format!("environments/{env}.yml"))).unwrap()
}

/// The command's exit code, stdout and stderr.
fn run(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = apinata().current_dir(dir).args(args).output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn env_extends_writes_the_bytes_the_app_writes() {
    let dir = tree();
    let (code, out, err) = run(dir.path(), &["env", "extends", "dev", "base"]);
    assert_eq!((code, out.as_str()), (0, "dev extends base\n"), "{err}");

    // The app's edit: the core setter on the loaded file, then a save.
    let twin = tree();
    let mut c = Collection::open(twin.path()).unwrap();
    let env = c.environment_mut("dev").unwrap();
    env.set_extends(Some("base".into()));
    env.save().unwrap();
    assert_eq!(read(dir.path(), "dev"), read(twin.path(), "dev"));
}

#[test]
fn env_extends_clear_removes_the_parent() {
    let dir = tree();
    run(dir.path(), &["env", "extends", "dev", "base"]);
    let (code, out, err) = run(dir.path(), &["env", "extends", "dev", "--clear"]);
    assert_eq!((code, out.as_str()), (0, "dev extends nothing\n"), "{err}");
    assert_eq!(read(dir.path(), "dev"), DEV);
}

#[test]
fn env_extends_refuses_a_cycle_with_exit_3() {
    let dir = tree();
    run(dir.path(), &["env", "extends", "dev", "base"]);
    let (code, _, err) = run(dir.path(), &["env", "extends", "base", "dev"]);
    assert_eq!(code, 3, "{err}");
    assert!(
        err.contains("environment inheritance cycle: base -> dev -> base"),
        "{err}"
    );
    assert_eq!(read(dir.path(), "base"), BASE);
}

#[test]
fn env_extends_refuses_a_missing_parent_with_exit_3() {
    let dir = tree();
    let (code, _, err) = run(dir.path(), &["env", "extends", "dev", "gone"]);
    assert_eq!(code, 3, "{err}");
    assert!(
        err.contains("environment `dev` extends `gone`, which does not exist"),
        "{err}"
    );
    assert_eq!(read(dir.path(), "dev"), DEV);
}

#[test]
fn env_extends_refuses_itself_with_exit_3() {
    let dir = tree();
    let (code, _, err) = run(dir.path(), &["env", "extends", "dev", "dev"]);
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("cannot extend itself"), "{err}");
}

#[test]
fn env_extends_needs_a_parent_or_clear() {
    let dir = tree();
    let (code, _, _) = run(dir.path(), &["env", "extends", "dev"]);
    assert_ne!(code, 0);
    let (code, _, _) = run(dir.path(), &["env", "extends", "dev", "base", "--clear"]);
    assert_ne!(code, 0);
    assert_eq!(read(dir.path(), "dev"), DEV);
}

#[test]
fn env_show_reports_the_parent() {
    let dir = tree();
    run(dir.path(), &["env", "extends", "dev", "base"]);
    let (code, out, err) = run(dir.path(), &["env", "show", "dev"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "extends: base\nport = 8080\n");
    let (_, out, _) = run(dir.path(), &["env", "show", "dev", "--json"]);
    let doc: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["extends"], "base");
    let (_, out, _) = run(dir.path(), &["env", "show", "base", "--json"]);
    let doc: Value = serde_json::from_str(&out).unwrap();
    assert!(doc.get("extends").is_none(), "{out}");
}

#[test]
fn env_extends_works_on_global_environments() {
    let profile = write_tree(&[
        ("global-environments/a.yml", "name: a\n"),
        ("global-environments/b.yml", "name: b\nextends: a\n"),
    ]);
    let cwd = tempfile::TempDir::new().unwrap();
    let go = |args: &[&str]| {
        let out = apinata()
            .env("APINATA_PROFILE_DIR", profile.path())
            .current_dir(cwd.path())
            .args(args)
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    };
    let (code, err) = go(&["env", "extends", "a", "b", "--global"]);
    assert_eq!(code, 3, "{err}");
    assert!(
        err.contains("environment inheritance cycle: a -> b -> a"),
        "{err}"
    );
    let (code, err) = go(&["env", "extends", "b", "--clear", "--global"]);
    assert_eq!(code, 0, "{err}");
    let b = std::fs::read_to_string(profile.path().join("global-environments/b.yml")).unwrap();
    assert_eq!(b, "name: b\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_with_the_child_resolves_a_variable_from_the_parent() {
    let s = MockServer::start().await;
    Mock::given(path("/ping"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let base = format!(
        "name: base\nvariables:\n  - name: host\n    value: {}\n",
        s.uri()
    );
    let req = "info:\n  name: ping\n  type: http\nhttp:\n  method: GET\n  url: \"{{host}}/ping\"\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/base.yml", &base),
        ("environments/dev.yml", DEV),
        ("ping.yml", req),
    ]);
    let (code, _, err) = run(dir.path(), &["env", "extends", "dev", "base"]);
    assert_eq!(code, 0, "{err}");
    let out = apinata()
        .current_dir(dir.path())
        .args(["run", "--env", "dev"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
