//! MCP server for Apinata: serves one OpenCollection to an AI agent over stdio.
//! Every tool calls `apinata-core`, so the agent sees what the CLI and app see.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod apply;
mod check;
mod cookies;
mod dto;
mod envs;
mod error;
mod examples;
mod gate;
mod generate;
mod git;
mod import_collection;
mod imports;
mod items;
mod mock;
mod oauth2;
mod options;
mod params;
mod policy;
mod prompts;
mod reads;
mod redact;
mod replace;
mod resources;
mod run;
mod scripts;
mod search;
mod send;
mod server;
mod tagged;
mod tree;
mod watch;
mod writes;

pub use error::McpError;
pub use options::McpOptions;
pub use server::ApinataServer;

use rmcp::ServiceExt;

/// Serve the collection named by `options` over stdin and stdout until the client disconnects.
pub async fn serve_stdio(options: McpOptions) -> Result<(), McpError> {
    let running = ApinataServer::new(options)?
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| McpError::Transport(e.to_string()))?;
    running
        .waiting()
        .await
        .map_err(|e| McpError::Transport(e.to_string()))?;
    Ok(())
}
