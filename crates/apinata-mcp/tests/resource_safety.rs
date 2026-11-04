//! What resources and prompts must never show: secrets, and anything outside the loaded items.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::json;

const SECRET: &str = "Zx9-very-secret-value-42";
const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";

/// A request whose header holds `SECRET` literally, as a user might paste it.
fn leaky() -> String {
    format!(
        "{}  headers:\n    - name: X-Key\n      value: {SECRET}\n",
        request("Leaky", 1, "GET", "http://127.0.0.1:1/leaky")
    )
}

async fn everything(server: &TestServer) -> String {
    let mut out = String::new();
    for resource in server.resources().await {
        out.push_str(&format!("{resource:?}\n"));
        out.push_str(&server.resource_text(&resource.uri).await);
    }
    for prompt in server.prompts().await {
        out.push_str(&format!("{prompt:?}\n"));
    }
    for (name, args) in [
        ("write_tests", json!({"path": "leaky.yml"})),
        ("document_folder", json!({})),
        ("explain_failure", json!({"path": "leaky.yml"})),
    ] {
        out.push_str(&server.prompt_text(name, args).await);
    }
    out
}

#[tokio::test]
async fn a_stored_secret_pasted_into_a_request_is_scrubbed() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("leaky.yml", &leaky()),
        ("environments/dev.yml", DEV),
    ])
    .await;
    server.set_secret("dev", "token", SECRET);
    let shown = everything(&server).await;
    assert!(!shown.contains(SECRET), "{shown}");
    assert!(shown.contains("<secret>"), "{shown}");
}

#[tokio::test]
async fn a_launch_env_var_value_is_scrubbed() {
    let server = TestServer::start_with(
        &[("opencollection.yml", ROOT), ("leaky.yml", &leaky())],
        |options| options.env_var("API_KEY", SECRET),
    )
    .await;
    let shown = everything(&server).await;
    assert!(!shown.contains(SECRET), "{shown}");
}

#[tokio::test]
async fn paths_that_leave_the_listed_items_are_refused() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("leaky.yml", &leaky()),
        (".env", &format!("API_KEY={SECRET}\n")),
        (".git/config", &format!("[remote]\n\turl = {SECRET}\n")),
        ("environments/dev.yml", DEV),
    ])
    .await;
    for uri in [
        "apinata://shop/../x",
        "apinata://shop/%2e%2e/x",
        "apinata://shop/%2E%2E/opencollection.yml",
        "apinata://shop/environments/../.env",
        "apinata://shop/.env",
        "apinata://shop/.git/config",
        "apinata://shop/opencollection.yml",
        "apinata://shop/environments/dev.yml",
        "apinata://shop/@environment/../.env",
        "apinata://other/leaky.yml",
        "apinata://Shop/leaky.yml",
        "apinata:///leaky.yml",
        "file:///etc/passwd",
    ] {
        let error = server.try_read_resource(uri).await.err();
        let error = error.unwrap_or_else(|| panic!("{uri} was served"));
        assert!(!error.contains(SECRET), "{uri}: {error}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_out_of_the_collection_shows_nothing_of_its_target() {
    let outside = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(
        outside.path().join("out.yml"),
        request("Out", 1, "GET", "http://127.0.0.1:1/outside-marker"),
    )
    .expect("write");
    std::fs::create_dir(outside.path().join("dir")).expect("mkdir");
    std::fs::write(
        outside.path().join("dir/in.yml"),
        request("In", 1, "GET", "http://127.0.0.1:1/outside-marker"),
    )
    .expect("write");
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    std::os::unix::fs::symlink(
        outside.path().join("out.yml"),
        server.root().join("link.yml"),
    )
    .expect("symlink");
    std::os::unix::fs::symlink(outside.path().join("dir"), server.root().join("linked"))
        .expect("symlink");
    // The listing is not empty (the root at least), and the links are neither listed nor served.
    let listed = server.resources().await;
    assert!(!listed.is_empty());
    for resource in &listed {
        assert!(
            !resource.uri.contains("link") && !format!("{resource:?}").contains("outside-marker"),
            "{resource:?}"
        );
    }
    for uri in ["apinata://shop/link.yml", "apinata://shop/linked/in.yml"] {
        let served = server.try_read_resource(uri).await;
        assert!(served.is_err(), "{uri} was served: {served:?}");
    }
}

#[tokio::test]
async fn a_bad_argument_holding_a_known_secret_is_not_echoed() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("leaky.yml", &leaky()),
        ("environments/dev.yml", DEV),
    ])
    .await;
    server.set_secret("dev", "token", SECRET);
    let prompt = server
        .try_prompt("write_tests", json!({"path": format!("{SECRET}.yml")}))
        .await
        .expect_err("unknown path");
    assert!(!prompt.contains(SECRET), "{prompt}");
    let resource = server
        .try_read_resource(&format!("apinata://shop/{SECRET}.yml"))
        .await
        .expect_err("unknown path");
    assert!(!resource.contains(SECRET), "{resource}");
    let named = server
        .try_prompt("explain_failure", json!({"path": SECRET}))
        .await
        .expect_err("unknown path");
    assert!(!named.contains(SECRET), "{named}");
}
