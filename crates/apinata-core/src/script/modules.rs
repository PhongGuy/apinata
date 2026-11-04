//! The script modules layer: the POSIX `path` module and `require` of collection files.

use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use rquickjs::{Ctx, Exception, Function, Object};
use serde_json::json;

use super::engine::Installer;
use super::host::HostState;
use crate::document::is_special_file;

const MODULES_JS: &str = include_str!("modules.js");

/// The largest module file `require` reads.
const MAX_MODULE_BYTES: u64 = 10 * 1024 * 1024;

/// Installs `require("path")`, the lazy `path` global and local `require`, with the collection
/// root as the working directory. Runs last, so it answers its names before every other layer.
pub(crate) fn installer(state: Arc<HostState>) -> Installer {
    Box::new(move |ctx| {
        let root = state.ctx.root().to_path_buf();
        let modules = Object::new(ctx.clone())?;
        modules.set("root", root.to_string_lossy().into_owned())?;
        let load_root = root.clone();
        let load = move |ctx: Ctx<'_>, name: String| load(&ctx, &load_root, &name);
        modules.set("load", Function::new(ctx.clone(), load)?)?;
        modules.set("resolve", Function::new(ctx.clone(), resolve)?)?;
        ctx.globals().set("__modules", modules)?;
        ctx.eval::<(), _>(MODULES_JS)
    })
}

/// Reads module `name` (relative to `root`, or absolute) as JSON `{source, dir}`. Both paths
/// are canonicalised before the containment check, so `..` and symlinks cannot leave the root.
fn load(ctx: &Ctx<'_>, root: &Path, name: &str) -> rquickjs::Result<String> {
    let mut file = root.join(name);
    if Path::new(name).extension().is_none() {
        file.as_mut_os_string().push(".js");
    }
    let missing = || Exception::throw_message(ctx, &format!("Cannot find module {name}"));
    let (Ok(canonical_root), Ok(canonical)) = (root.canonicalize(), file.canonicalize()) else {
        return Err(missing());
    };
    if !canonical.starts_with(&canonical_root) {
        return Err(Exception::throw_message(
            ctx,
            "Access to files outside of the collectionPath is not allowed.",
        ));
    }
    let unreadable =
        || Exception::throw_message(ctx, &format!("Error loading local module {name}"));
    if is_special_file(&canonical) {
        return Err(unreadable());
    }
    // Read one byte past the cap, so a file that grows after any size check still fails here.
    let mut bytes = Vec::new();
    let read =
        File::open(&canonical).and_then(|f| f.take(MAX_MODULE_BYTES + 1).read_to_end(&mut bytes));
    if read.is_err() || bytes.len() as u64 > MAX_MODULE_BYTES {
        return Err(unreadable());
    }
    let dir = root.join(name);
    let dir = normalise(dir.parent().unwrap_or(root));
    Ok(json!({
        "source": String::from_utf8_lossy(&bytes),
        "dir": dir.to_string_lossy(),
    })
    .to_string())
}

/// `dir.join(name)` with `.` dropped and `..` popping its parent, without touching the disk.
fn resolve(name: String, dir: String) -> String {
    normalise(&Path::new(&dir).join(name))
        .to_string_lossy()
        .into_owned()
}

fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}
