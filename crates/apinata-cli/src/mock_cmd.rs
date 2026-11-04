//! `apinata mock`: open the collection as `apinata run` does and serve its saved examples
//! until ctrl-c. Hits are logged to stdout; warnings go to stderr.

use std::io::Write;
use std::process::ExitCode;

use apinata_core::mock::{self, MockError, MockOptions};

use crate::run::{global_options, open_at};
use crate::{MockArgs, collection_arg};

/// Exit code for any setup failure: a refused or busy address, a collection that does not
/// open, a runtime that does not start.
const EXIT_USAGE: u8 = 3;

/// Serve until ctrl-c: 0 then, 3 when it cannot start serving.
pub fn run(args: &MockArgs) -> ExitCode {
    // The loopback check precedes opening anything, so a refusal costs no work.
    if !args.allow_non_loopback && !args.bind.is_loopback() {
        eprintln!(
            "error: {} is not a loopback address; pass --allow-non-loopback to expose the examples to the network",
            args.bind
        );
        return ExitCode::from(EXIT_USAGE);
    }
    let target = global_options(
        args.global_env.as_deref(),
        args.workspace.as_deref(),
        &args.global_env_var,
    )
    .and_then(|(options, secrets)| {
        let path = collection_arg::require(
            args.path.as_deref(),
            args.collection.as_deref(),
            "`apinata mock <PATH>` or `apinata mock --collection <DIR>`",
        )?;
        collection_arg::with_hint(
            open_at(&path, args.env.as_deref(), &args.env_var, &secrets, options),
            "pass the collection as `apinata mock <PATH>` or `apinata mock --collection <DIR>`",
        )
    });
    let target = match target {
        Ok(target) => target,
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    runtime.block_on(async {
        // Hit lines ignore write errors: a closed stdout must not stop a server that is still serving.
        let options = MockOptions::new()
            .bind(args.bind)
            .port(args.port)
            .delay_ms(args.delay)
            .allow_non_loopback(args.allow_non_loopback)
            .on_hit(|hit| {
                let _ = writeln!(std::io::stdout().lock(), "{}", hit.line());
            });
        let handle = match mock::start(&target.collection, &target.ctx, options).await {
            Ok(handle) => handle,
            Err(e @ MockError::NonLoopbackBind) => {
                eprintln!("error: {e}; pass --allow-non-loopback");
                return ExitCode::from(EXIT_USAGE);
            }
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(EXIT_USAGE);
            }
        };
        for warning in handle.warnings() {
            eprintln!("warning: {warning}");
        }
        if args.allow_non_loopback {
            eprintln!(
                "warning: listening on {}, so anyone who can reach it can read the saved examples",
                handle.addr()
            );
        }
        // Register the interrupt handler before announcing readiness: `ctrl_c()` only installs it
        // when first polled, so a signal sent right after the line would otherwise kill the process.
        #[cfg(unix)]
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).ok();
        // A broken pipe means nobody reads stdout, which must not stop the server; any other
        // write error still ends the run with exit 3 before the first request is served.
        if let Err(e) = announce(handle.addr())
            && e.kind() != std::io::ErrorKind::BrokenPipe
        {
            eprintln!("error: cannot write output: {e}");
            handle.stop().await;
            return ExitCode::from(EXIT_USAGE);
        }
        #[cfg(unix)]
        match interrupt.as_mut() {
            Some(signal) => {
                signal.recv().await;
            }
            None => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
        #[cfg(not(unix))]
        let _ = tokio::signal::ctrl_c().await;
        handle.stop().await;
        ExitCode::SUCCESS
    })
}

/// Write the readiness line, returning the write error instead of exiting.
fn announce(addr: std::net::SocketAddr) -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "Listening on http://{addr}").and_then(|()| stdout.flush())
}
