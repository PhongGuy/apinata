//! The vendored moment, lodash, crypto-js, tv4, buffer and ajv behind lazy globals and `require`, evaluated on first use.

use rquickjs::{Ctx, Function, Value};

use super::chai::options;
use super::engine::Installer;

const LIBS_JS: &str = include_str!("libs.js");

/// Installs `__loadLib` and the script that wraps it into the lazy library globals.
pub(crate) fn installer() -> Installer {
    Box::new(|ctx| {
        ctx.globals()
            .set("__loadLib", Function::new(ctx.clone(), load)?)?;
        ctx.globals()
            .set("__proxyTarget", Function::new(ctx.clone(), proxy_target)?)?;
        ctx.eval::<(), _>(LIBS_JS)
    })
}

/// The target behind a Proxy, read without running a trap: `undefined` for anything else and `null`
/// for a revoked proxy (`false` for a callable one, whose target cannot be read), so the inspect text can show what Node shows.
pub(super) fn proxy_target<'js>(ctx: Ctx<'js>, value: Value<'js>) -> Value<'js> {
    // The engine types a callable proxy as a function, so it cannot be read as a proxy; say only that it is one.
    let callable = value.is_proxy();
    let Some(proxy) = value.into_proxy() else {
        return if callable { Value::new_bool(ctx, false) } else { Value::new_undefined(ctx) };
    };
    match proxy.target() {
        Ok(target) if !target.as_value().is_null() => target.into_value(),
        _ => {
            // A revoked proxy leaves its exception pending; clear it so the script sees none.
            let _ = ctx.catch();
            Value::new_null(ctx)
        }
    }
}

/// Evaluates one vendored UMD file under a CommonJS `module` and returns its export. Each file
/// runs in sloppy mode under its own name and is handed none of the globals it probes, so it
/// always takes the CommonJS branch.
fn load<'js>(ctx: Ctx<'js>, name: String) -> rquickjs::Result<Value<'js>> {
    let (source, file) = match name.as_str() {
        "moment" => (
            include_str!("vendor/moment-2.30.1.min.js"),
            "moment-2.30.1.min.js",
        ),
        "lodash" => (
            include_str!("vendor/lodash-4.17.21.min.js"),
            "lodash-4.17.21.min.js",
        ),
        "crypto-js" => (
            include_str!("vendor/crypto-js-4.2.0.js"),
            "crypto-js-4.2.0.js",
        ),
        "tv4" => (include_str!("vendor/tv4-1.3.0.js"), "tv4-1.3.0.js"),
        "buffer" => (
            include_str!("vendor/buffer-5.7.1.min.js"),
            "buffer-5.7.1.min.js",
        ),
        "ajv" => (
            include_str!("vendor/ajv-8.20.0.min.js"),
            "ajv-8.20.0.min.js",
        ),
        _ => return Err(rquickjs::Error::new_from_js("string", "library")),
    };
    let wrapped = format!(
        "(function (module, exports, define, window, global, self, require) {{\n{source}\nreturn module.exports;\n}})({{ exports: {{}} }}, {{}})"
    );
    ctx.eval_with_options(wrapped, options(file))
}
