//! Failures that stop the MCP server from starting or serving.

/// Why the server could not open its collection or keep its transport running.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum McpError {
    /// The collection at the configured root failed to open.
    #[error(transparent)]
    Open(#[from] apinata_core::OpenError),
    /// The stdio transport failed to start or broke while serving.
    #[error("MCP transport: {0}")]
    Transport(String),
    /// The HTTP client used for sends could not be built.
    #[error("HTTP client: {0}")]
    Client(String),
}
