//! `apinata gen`: resolve one request file and print it as code, without sending it.

use std::collections::BTreeSet;
use std::process::ExitCode;

use anyhow::{Context, anyhow};
use apinata_core::{
    AuthorizeOptions, CancelHandle, ContextOptions, ExecOptions, Executor, ResolveError,
    ResolvedRequest, generate,
};

use crate::{EXIT_USAGE, GenArgs, run};

/// Print the request at `args.path` as `args.target` code; every failure exits 3.
pub fn run(args: &GenArgs) -> ExitCode {
    match resolved(args) {
        Ok(req) => {
            for warning in req.warnings() {
                eprintln!("warning: {warning}");
            }
            outln!("{}", generate(&req, args.target));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// The one request file at `args.path`, resolved against the chosen environment.
fn resolved(args: &GenArgs) -> anyhow::Result<ResolvedRequest> {
    let path = std::fs::canonicalize(&args.path)
        .with_context(|| format!("cannot open {}", args.path.display()))?;
    if path.is_dir() {
        return Err(anyhow!("gen takes one request file"));
    }
    let target = run::open_at(
        &path,
        args.env.as_deref(),
        &args.env_var,
        &BTreeSet::new(),
        ContextOptions::new(),
    )?;
    let Some(file) = target.plan.requests().next() else {
        let error = target.plan.load_errors().first();
        return Err(anyhow!(
            "{}",
            error.map_or("the file holds no request", |e| e.message())
        ));
    };
    match apinata_core::resolve(file, &target.ctx) {
        // A cached OAuth 2.0 token may exist on disk but not yet in memory; load it from the
        // cache only, so generating code never fetches a token, then resolve once more.
        Err(ResolveError::OAuth2NoToken { .. }) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let executor = Executor::new(ExecOptions::new())?;
            runtime.block_on(executor.prepare_auth(
                file,
                &target.ctx,
                &AuthorizeOptions::new().cache_only(true),
                &CancelHandle::new(),
            ))?;
            Ok(apinata_core::resolve(file, &target.ctx)?)
        }
        result => Ok(result?),
    }
}
