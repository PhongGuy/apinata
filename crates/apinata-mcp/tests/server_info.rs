//! The `initialize` answer names this server and tells the agent how to start.

use crate::common;

use common::{ROOT, TestServer};

#[tokio::test]
async fn initialize_reports_apinata_and_its_version_with_instructions() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let (info, instructions) = server.server_info();
    assert_eq!(info.name, "apinata");
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    let instructions = instructions.expect("instructions");
    assert!(instructions.contains("list_items"), "{instructions}");
}
