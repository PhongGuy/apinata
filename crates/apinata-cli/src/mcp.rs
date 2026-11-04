//! `apinata mcp`: find the collection as `apinata run` does and serve it over stdio.
//! Stdout carries the protocol, so every message here goes to stderr.

use std::process::ExitCode;

use std::sync::Arc;

use anyhow::{Context, bail};
use apinata_core::TokenCache;
use apinata_mcp::{McpError, McpOptions};

use crate::network;
use crate::run::{find_root, resolve_global};
use crate::{McpArgs, collection_arg};

/// Exit code when the server cannot start: no collection, a bad option, a failed setup.
const EXIT_SETUP: u8 = 3;
/// Exit code when the stdio transport fails to start or breaks while serving.
const EXIT_TRANSPORT: u8 = 2;

/// Serve until the client disconnects: 0 then, 3 when it cannot start, 2 when the transport fails.
pub fn serve(args: &McpArgs) -> ExitCode {
    let options = match options(args) {
        Ok(options) => options,
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::from(EXIT_SETUP);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(EXIT_SETUP);
        }
    };
    match runtime.block_on(apinata_mcp::serve_stdio(options)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e @ (McpError::Open(_) | McpError::Client(_))) => {
            eprintln!("error: {e}");
            ExitCode::from(EXIT_SETUP)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(EXIT_TRANSPORT)
        }
    }
}

/// Resolve the collection root from `--collection` or the working directory and carry the flags.
fn options(args: &McpArgs) -> anyhow::Result<McpOptions> {
    let named = collection_arg::pick(args.path.as_deref(), args.collection.as_deref())?;
    let start = match named {
        Some(path) => path,
        None => std::env::current_dir().context("cannot read the working directory")?,
    };
    let start = std::fs::canonicalize(&start)
        .with_context(|| format!("cannot open {}", start.display()))?;
    let root = collection_arg::with_hint(
        find_root(&start),
        &collection_arg::optional_hint("apinata mcp [COLLECTION]"),
    )?;
    let mut options = McpOptions::new(root)
        .insecure(args.insecure)
        .http_version(args.network.http_version)
        .network(network::settings(&args.network))
        .client_certificates(network::client_certificates(&args.network)?);
    if let Some(env) = &args.env {
        options = options.env(env);
    }
    // The server already treats every override as sensitive, so the origin flag is not needed.
    for v in &args.env_var {
        options = options.env_var(&v.key, &v.value);
    }
    // A mistyped global name fails at launch, before the client sees a server.
    if let Some(name) = &args.global_env {
        let (globals, name) = resolve_global(name, args.workspace.as_deref())?;
        let name = &name;
        if globals.environment(name).is_none() {
            let file = globals.dir().join(format!("{name}.yml"));
            match globals.load_errors().iter().find(|e| e.path() == file) {
                Some(e) => bail!(
                    "global environment `{name}` failed to load: {}",
                    e.message()
                ),
                None => bail!("{}", globals.environment_not_found(name)),
            }
        }
        options = options.global_env(Arc::new(globals), name);
    }
    for v in &args.global_env_var {
        options = options.global_env_var(&v.key, &v.value);
    }
    // Tokens are shared with `apinata run` through the same encrypted file.
    if let Some(path) = TokenCache::default_path() {
        options = options.token_cache_file(path);
    }
    Ok(options)
}
