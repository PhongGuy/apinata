//! Opening a response body in the system's default app: the body goes to a fresh 0600 file in
//! this process's private temp directory, then to the injected opener. An HTML body goes as an
//! inert `.html` page holding it in a sandboxed srcdoc frame (see [`inert_html`]), an SVG body
//! as a data: image in such a page. Only `main.rs` installs the real opener, so tests never
//! launch anything.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::{App, AppContext as _, Global, Task};

/// Opens a file in its default app; only `main.rs` installs `App::open_with_system`.
pub type FileOpener = Arc<dyn Fn(&Path, &mut App)>;

/// The name prefix of each process's private directory under the base.
pub const OPEN_DIR_PREFIX: &str = "apinata-open-";

/// The Content-Security-Policy every inert browser copy carries: no script, fetch, subresource,
/// frame, form target or `<base>`; inline CSS and data: images still render. `frame-src 'none'`
/// blocks every fetched frame, and a srcdoc frame is never fetched.
pub const INERT_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; frame-src 'none'; form-action 'none'; base-uri 'none'";

// Byte 0 of every inert page. Ahead of any body markup the policy lands in the implied head
// before a body script can run, and the charset meta wins the prescan so a body cannot switch
// the decoder. The outer page has no doctype (quirks mode); a framed body keeps its own.
fn inert_head() -> String {
    format!(
        r#"<meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="{INERT_CSP}">"#
    )
}

/// An HTML body as a page whose scripts, network loads, forms and navigation are blocked: the
/// inert head, then a full-tab `<iframe sandbox="">` whose srcdoc is `body`, attribute-escaped.
/// The frame inherits the head's policy, and its empty sandbox stops scripts, refresh, forms,
/// popups and navigation of the tab; frame-src stops it navigating itself.
pub fn inert_html(body: &str) -> String {
    format!(
        r#"{}<title>HTML response</title><style>html,body{{margin:0;height:100%}}iframe{{display:block;border:0;width:100%;height:100%}}</style><iframe sandbox="" title="HTML response" srcdoc="{}"></iframe>"#,
        inert_head(),
        srcdoc_escape(body)
    )
}

// Only `"` ends the attribute value and `&` starts a reference; escaping the other three too
// keeps no body tag in the file. One pass, so no inserted reference is escaped again.
fn srcdoc_escape(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    for c in body.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// An SVG body as an inert HTML page showing it as a data: image, where SVG runs no script and
/// loads nothing.
pub fn inert_svg_page(svg: &[u8]) -> String {
    use base64::Engine as _;
    let data = base64::engine::general_purpose::STANDARD.encode(svg);
    format!(
        r#"{}<title>SVG response</title><img alt="SVG response" src="data:image/svg+xml;base64,{data}">"#,
        inert_head()
    )
}

/// A sibling directory older than this is pruned even when its owner looks alive, which
/// covers a reused process id and an instance that has not opened anything for a day.
const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Default)]
struct OpenExternal {
    opener: Option<FileOpener>,
    base: PathBuf,
    dir: Option<SessionDir>,
}

// The session directory and the identity it was created with: its name is visible in a
// shared temp dir, so a same-named directory made after a prune must not be trusted.
#[derive(Clone)]
struct SessionDir {
    path: PathBuf,
    identity: Identity,
}

// Device, inode and owner uid: an inode reused by another user's directory never matches.
type Identity = Option<(u64, u64, u32)>;

impl Global for OpenExternal {}

/// Sets the opener, creates this session's private directory under `base` (mode 0700,
/// unpredictable name) and removes older `apinata-open-*` directories under `base` owned by
/// the current uid. Without a directory (creation failed) nothing is installed.
pub fn install(opener: FileOpener, base: PathBuf, cx: &mut App) {
    install_with_liveness(opener, base, process_alive, cx);
}

/// `install` with an explicit check of whether a process id is running. A sibling directory
/// is pruned only when its owner is not running or it is older than a day, so a second
/// instance never removes the files a running one handed to an app.
pub fn install_with_liveness(
    opener: FileOpener,
    base: PathBuf,
    alive: fn(u32) -> bool,
    cx: &mut App,
) {
    let Ok(dir) = create_session_dir(&base) else {
        return;
    };
    let state = cx.default_global::<OpenExternal>();
    state.opener = Some(opener);
    state.base = base.clone();
    let mine = dir.path.clone();
    state.dir = Some(dir);
    cx.background_spawn(async move { prune(&base, &mine, alive) })
        .detach();
}

/// Whether an opener is installed.
pub fn installed(cx: &App) -> bool {
    cx.try_global::<OpenExternal>()
        .is_some_and(|s| s.opener.is_some())
}

/// Hands `path` to the installed opener; `false` when none is installed.
pub fn open_path(path: &Path, cx: &mut App) -> bool {
    let Some(opener) = cx
        .try_global::<OpenExternal>()
        .and_then(|s| s.opener.clone())
    else {
        return false;
    };
    opener(path, cx);
    true
}

/// This session's private directory, once installed.
pub fn session_dir(cx: &App) -> Option<PathBuf> {
    Some(cx.try_global::<OpenExternal>()?.dir.as_ref()?.path.clone())
}

/// The file extension for a Content-Type: pdf, png, jpg, gif, webp, svg, html, zip, gz, mp3,
/// mp4, wav, ogg, woff2, wasm by media type (parameters and case ignored), else "bin".
pub fn extension(content_type: &str) -> &'static str {
    let media = content_type.split(';').next().unwrap_or_default().trim();
    match media.to_ascii_lowercase().as_str() {
        "application/pdf" => "pdf",
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "text/html" => "html",
        "application/zip" | "application/x-zip-compressed" => "zip",
        "application/gzip" | "application/x-gzip" => "gz",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "video/mp4" | "audio/mp4" => "mp4",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/ogg" | "video/ogg" | "application/ogg" => "ogg",
        "font/woff2" => "woff2",
        "application/wasm" => "wasm",
        _ => "bin",
    }
}

/// Writes `bytes` to a new 0600 file in the session directory off the main thread, then
/// calls the opener with its path. A session directory removed meanwhile (another instance
/// pruned it as stale) is replaced by a fresh one under the same base, never by the old name.
/// `Ok(())` without an opener; `Err` holds the write error for display. Raw bytes are never
/// named as a page a browser would run (html and svg become bin); see [`open_page`].
pub fn open_bytes(
    bytes: impl AsRef<[u8]> + Send + 'static,
    content_type: &str,
    cx: &mut App,
) -> Task<Result<(), String>> {
    let ext = match extension(content_type) {
        "html" | "svg" => "bin",
        ext => ext,
    };
    open_made(ext, move || bytes, cx)
}

/// [`open_bytes`] for a page `make` builds off the main thread (where redaction and wrapping
/// cost nothing on the UI), always written as a `.html` file.
pub fn open_page(
    make: impl FnOnce() -> String + Send + 'static,
    cx: &mut App,
) -> Task<Result<(), String>> {
    open_made("html", make, cx)
}

// The shared write-then-open path: `make` runs on the background thread that writes its output.
fn open_made<B: AsRef<[u8]>>(
    ext: &'static str,
    make: impl FnOnce() -> B + Send + 'static,
    cx: &mut App,
) -> Task<Result<(), String>> {
    let state = cx.try_global::<OpenExternal>();
    let (Some(opener), Some(dir)) = (
        state.and_then(|s| s.opener.clone()),
        state.and_then(|s| s.dir.clone()),
    ) else {
        return Task::ready(Ok(()));
    };
    let base = state.map(|s| s.base.clone()).unwrap_or_default();
    let write = cx.background_spawn(async move {
        let dir = match std::fs::symlink_metadata(&dir.path) {
            Ok(meta) if meta.is_dir() && identity(&meta) == dir.identity => dir,
            _ => create_session_dir(&base)?,
        };
        write_private(&dir.path, ext, make().as_ref()).map(|path| (dir, path))
    });
    cx.spawn(async move |cx| {
        let (dir, path) = write.await.map_err(|e| e.to_string())?;
        cx.update(|cx| {
            cx.default_global::<OpenExternal>().dir = Some(dir);
            opener(&path, cx)
        });
        Ok(())
    })
}

// The process id leads the random part of the name, so a later instance can tell whose
// directory a sibling is without a lock file.
fn create_session_dir(base: &Path) -> std::io::Result<SessionDir> {
    let mut builder = tempfile::Builder::new();
    let prefix = format!("{OPEN_DIR_PREFIX}{}-", std::process::id());
    builder.prefix(&prefix);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let path = builder.tempdir_in(base)?.keep();
    let identity = identity(&std::fs::symlink_metadata(&path)?);
    Ok(SessionDir { path, identity })
}

#[cfg(unix)]
fn identity(meta: &std::fs::Metadata) -> Identity {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino(), meta.uid()))
}

// Without a device, inode and owner the check falls back to "still a directory"; the temp dir there
// is per user.
#[cfg(not(unix))]
fn identity(_: &std::fs::Metadata) -> Identity {
    None
}

fn write_private(dir: &Path, ext: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    use std::io::Write as _;
    let suffix = format!(".{ext}");
    let mut file = tempfile::Builder::new()
        .prefix("body-")
        .suffix(&suffix)
        .tempfile_in(dir)?;
    file.write_all(bytes)?;
    let (_, path) = file.keep().map_err(|e| e.error)?;
    Ok(path)
}

// Removes sibling open directories this user owns whose owner process is gone (or cannot be
// read from the name) or that are stale. Symlinks and anything else are never followed or
// touched, and every error is ignored.
fn prune(base: &Path, mine: &Path, alive: fn(u32) -> bool) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let Ok(own) = std::fs::symlink_metadata(mine) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let Some(rest) = name.to_str().and_then(|n| n.strip_prefix(OPEN_DIR_PREFIX)) else {
            continue;
        };
        // Names, not paths: `read_dir` keeps a relative base relative, `tempfile` does not.
        if Some(name.as_os_str()) == mine.file_name() {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() || !same_owner(&meta, &own) {
            continue;
        }
        let owner = rest.split('-').next().and_then(|p| p.parse::<u32>().ok());
        let stale = meta
            .modified()
            .ok()
            .and_then(|m| SystemTime::now().duration_since(m).ok())
            .is_some_and(|age| age > STALE_AFTER);
        if stale || !owner.is_some_and(alive) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

#[cfg(unix)]
fn same_owner(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.uid() == b.uid()
}

#[cfg(not(unix))]
fn same_owner(_: &std::fs::Metadata, _: &std::fs::Metadata) -> bool {
    true
}

// Linux answers through /proc; elsewhere every owner counts as alive, so only age prunes.
fn process_alive(pid: u32) -> bool {
    if cfg!(target_os = "linux") {
        Path::new("/proc").join(pid.to_string()).exists()
    } else {
        true
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn identity_includes_the_owner() {
        let dir = tempfile::tempdir().unwrap();
        let meta = std::fs::symlink_metadata(dir.path()).unwrap();
        assert_eq!(identity(&meta), Some((meta.dev(), meta.ino(), meta.uid())));
    }
}
