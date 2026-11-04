//! One response saved as an example through core, the CLI and MCP leaves the same bytes.

use crate::common;

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use apinata_core::{
    CancelHandle, Collection, ContextOptions, ExampleResponse, ExecOptions, Executor, ItemPath,
    RequestFile, RunOptions, VarContext, request_secrets, send_one,
};
use common::{ROOT, apinata, write_tree};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};
use tempfile::TempDir;

/// Every connection gets this reply verbatim, with no `date` or other per-run header, so any
/// difference between the saved files comes from the shells and not from the server.
const REPLY: &str = "HTTP/1.1 201 Created\r\ncontent-type: application/json\r\nx-fixed: one\r\n\
                     content-length: 23\r\nconnection: close\r\n\r\n{\"id\":7,\"name\":\"fixed\"}";

/// Answer every connection on a loopback port with `REPLY`, after reading the request head.
/// The thread lives as long as the test binary; it holds nothing a later test needs.
fn fixed_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let _ = stream.write_all(REPLY.as_bytes());
        }
    });
    format!("http://{addr}")
}

fn collection(url: &str) -> TempDir {
    let request = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\n\nhttp:\n  method: GET\n  url: {url}/ping\n"
    );
    write_tree(&[("opencollection.yml", ROOT), ("r.yml", &request)])
}

/// Save the example the way the CLI and MCP do, through core's public API only.
async fn save_through_core(root: &Path) {
    let c = Collection::open(root).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let request = RequestFile::load(root.join("r.yml")).expect("loads");
    let item = ItemPath::new("r.yml").expect("item path");
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("executor"));
    let result = send_one(
        item.clone(),
        &request,
        &ctx,
        &exec,
        &RunOptions::new(),
        &CancelHandle::new(),
    )
    .await;
    let response = result.response.as_ref().expect("a response");
    let mut example = ExampleResponse::from_response(response);
    let secrets = request_secrets(&request, &ctx, &[]).expect("secrets");
    let description = example.scrub(Some(&secrets)).description("");
    let mut c = Collection::open(root).expect("opens");
    c.add_example(&item, "saved", &description, &example)
        .expect("saves");
}

/// Send and save in one `apinata mcp` session, since `save_example` keeps the session's last
/// response.
async fn save_through_mcp(root: &Path) {
    let mut cmd = common::apinata_tokio();
    cmd.arg("mcp")
        .arg("--collection")
        .arg(root)
        .current_dir(root)
        .kill_on_drop(true);
    let call = |tool: &'static str, args: Value| {
        let Value::Object(args) = args else {
            unreachable!("literal objects")
        };
        CallToolRequestParams::new(tool).with_arguments(args)
    };
    let exchange = async {
        let client =
            ().serve(TokioChildProcess::new(cmd).expect("spawns"))
                .await
                .expect("handshake");
        for (tool, args) in [
            ("send_request", json!({"path": "r.yml"})),
            ("save_example", json!({"path": "r.yml", "name": "saved"})),
        ] {
            let result = client.call_tool(call(tool, args)).await.expect("call");
            assert_ne!(result.is_error, Some(true), "{tool}: {result:?}");
        }
        client.cancel().await.expect("closes");
    };
    tokio::time::timeout(Duration::from_secs(30), exchange)
        .await
        .expect("the stdio exchange finishes within 30 s");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_cli_and_mcp_save_the_same_example_bytes() {
    let url = fixed_server();
    let (core, cli, mcp) = (collection(&url), collection(&url), collection(&url));

    save_through_core(core.path()).await;
    apinata()
        .arg("run")
        .arg(cli.path().join("r.yml"))
        .args(["--no-keychain", "--save-example", "saved"])
        .timeout(Duration::from_secs(30))
        .assert()
        .code(0);
    save_through_mcp(mcp.path()).await;

    let read = |dir: &TempDir| std::fs::read_to_string(dir.path().join("r.yml")).expect("read");
    let from_core = read(&core);
    for saved in ["examples:", "status: 201", "x-fixed", "fixed"] {
        assert!(from_core.contains(saved), "{saved} missing:\n{from_core}");
    }
    assert_eq!(read(&cli), from_core, "the CLI's file differs from core's");
    assert_eq!(read(&mcp), from_core, "MCP's file differs from core's");
}
