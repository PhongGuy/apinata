//! The vendored chai behind `expect`, `assert` and `require`, evaluated on first use.

use rquickjs::context::EvalOptions;
use rquickjs::{Ctx, Function, Object};

use super::engine::Installer;

const CHAI: &str = include_str!("vendor/chai-4.5.0.js");
const EXTENSIONS: &str = include_str!("vendor/chai-extensions.js");
const CHAI_JS: &str = include_str!("chai.js");

/// Installs the lazy `expect` and `assert` globals and `require`.
pub(crate) fn installer() -> Installer {
    Box::new(|ctx| {
        ctx.globals()
            .set("__loadChai", Function::new(ctx.clone(), load)?)?;
        ctx.eval::<(), _>(CHAI_JS)
    })
}

/// Evaluates chai and its extensions, returning the chai export. The bundle runs in sloppy mode
/// under its own file name, so its frames never read as the script's lines. Its loader is handed
/// a CommonJS `module` and sees no other globals it probes, so a script's own cannot capture it.
fn load<'js>(ctx: Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    let source = format!(
        "(function (module, exports, define, window, global, self, require) {{\n{CHAI}\nreturn module.exports;\n}})({{ exports: {{}} }}, {{}})"
    );
    let chai: Object = ctx.eval_with_options(source, options("chai-4.5.0.js"))?;
    let extend: Function = ctx.eval_with_options(EXTENSIONS, options("chai-extensions.js"))?;
    extend.call::<_, ()>((chai.clone(),))?;
    Ok(chai)
}

pub(super) fn options(file: &str) -> EvalOptions {
    let mut options = EvalOptions::default();
    options.strict = false;
    options.filename = Some(file.to_owned());
    options
}
