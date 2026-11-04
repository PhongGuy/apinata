//! `bru.sleep` and `setTimeout`: an async host wait on tokio's clock, wrapped by `timers.js`.

use std::time::Duration;

use rquickjs::Function;
use rquickjs::prelude::Async;

use super::engine::Installer;

const TIMERS_JS: &str = include_str!("timers.js");

/// The longest single wait in milliseconds, a day; the phase deadline ends any sleep before it.
const MAX_WAIT_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;

/// Installs `__sleep` and `timers.js`, which builds `bru.sleep` and `setTimeout` over it.
pub(crate) fn installer() -> Installer {
    Box::new(|ctx| {
        let sleep = |ms: f64| async move {
            tokio::time::sleep(wait(ms)).await;
            Ok::<_, rquickjs::Error>(())
        };
        ctx.globals()
            .set("__sleep", Function::new(ctx.clone(), Async(sleep))?)?;
        ctx.eval::<(), _>(TIMERS_JS)
    })
}

/// The wait for `ms` milliseconds: NaN or negative is none, and the rest is capped at a day so
/// the `Duration` never overflows, Infinity included.
pub(super) fn wait(ms: f64) -> Duration {
    if ms > 0.0 {
        Duration::from_secs_f64(ms.min(MAX_WAIT_MS) / 1000.0)
    } else {
        Duration::ZERO
    }
}
