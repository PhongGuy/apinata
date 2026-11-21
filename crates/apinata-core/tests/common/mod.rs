//! Helpers shared by the variable, resolution and executor integration tests.
#![allow(dead_code)] // each test binary uses a different subset

use apinata_core::{
    Collection, ContextOptions, ExecOptions, Executor, Folder, RequestFile, ResolveError,
    ResolvedRequest, VarContext, resolve,
};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

pub fn write_tree(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    for (rel, text) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, text).expect("write");
    }
    dir
}

/// The request at `rel` under the collection root, searched through the whole tree.
pub fn request<'a>(c: &'a Collection, rel: &str) -> &'a RequestFile {
    fn walk<'a>(f: &'a Folder, target: &Path) -> Option<&'a RequestFile> {
        f.requests()
            .iter()
            .find(|r| r.path() == target)
            .or_else(|| f.folders().iter().find_map(|sub| walk(sub, target)))
    }
    walk(c.tree(), &c.root().join(rel)).unwrap_or_else(|| panic!("no request {rel}"))
}

/// Open `files` as a collection that must load cleanly, with no environment selected.
pub fn setup(files: &[(&str, &str)]) -> (TempDir, Collection, VarContext) {
    let dir = write_tree(files);
    let c = Collection::open(dir.path()).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    (dir, c, ctx)
}

pub fn at(c: &Collection, ctx: &VarContext, rel: &str) -> Result<ResolvedRequest, ResolveError> {
    resolve(request(c, rel), ctx)
}

pub fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// Resolve one request file with `method` and `url`; `under_http` is indented under `http:`
/// and `top` holds top-level blocks such as `settings:`.
pub fn resolved_one(method: &str, url: &str, under_http: &str, top: &str) -> ResolvedRequest {
    let text = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: {method}\n  url: \"{url}\"\n{under_http}{top}"
    );
    let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
    let (_dir, c, ctx) = setup(&[("opencollection.yml", root), ("r.yml", &text)]);
    at(&c, &ctx, "r.yml").expect("resolves")
}

/// An executor with default options.
pub fn executor() -> Executor {
    Executor::new(ExecOptions::new()).expect("client")
}
pub mod body_server;
pub mod potluck;
pub mod raw_server;
pub mod sse_server;
pub mod ws_server;

/// A port that refuses every connection and that no other process can take: the socket is
/// bound for the whole process but never listens.
pub fn dead_port() -> u16 {
    use socket2::{Domain, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).expect("socket");
    socket
        .bind(&std::net::SocketAddr::from(([127, 0, 0, 1], 0)).into())
        .expect("bind");
    let port = socket
        .local_addr()
        .expect("addr")
        .as_socket()
        .expect("ip")
        .port();
    std::mem::forget(socket);
    port
}

/// Drops a path's permissions for one test and puts the original mode back on drop, so a
/// failing assertion never leaves a temp dir the cleanup cannot remove. Bind it with
/// `let _g =` after the `TempDir` so it drops first.
#[cfg(unix)]
pub struct ModeGuard {
    path: std::path::PathBuf,
    original: u32,
}

#[cfg(unix)]
impl ModeGuard {
    /// Records the current mode, then applies `mode`. The guard exists before the chmod, so a
    /// panic right after still restores.
    pub fn set(path: impl AsRef<std::path::Path>, mode: u32) -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let path = path.as_ref().to_path_buf();
        let original = std::fs::metadata(&path).expect("stat").permissions().mode();
        let guard = Self { path, original };
        std::fs::set_permissions(&guard.path, std::fs::Permissions::from_mode(mode))
            .expect("chmod");
        guard
    }

    /// Puts the original mode back now; the drop does it again harmlessly.
    pub fn restore(&self) {
        use std::os::unix::fs::PermissionsExt as _;
        let _ =
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(self.original));
    }
}

#[cfg(unix)]
impl Drop for ModeGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

/// The minimal `opencollection.yml`.
pub const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

/// Write `files` as a tree and check it.
pub fn check_tree(
    files: &[(&str, &str)],
    opts: &apinata_core::CheckOptions,
) -> (TempDir, apinata_core::CheckReport) {
    let dir = write_tree(files);
    let report = apinata_core::check_collection(dir.path(), opts).expect("checks");
    (dir, report)
}

/// `parsed` with each credential the import moved into a secret variable put back in place and
/// the notes asking for those secrets removed, so a test can pin what a curl command maps to.
/// Secrets with a fixed name, such as the AWS secret key, stay as parsed.
pub fn reveal_curl(
    mut parsed: apinata_core::import::ParsedCurl,
) -> apinata_core::import::ParsedCurl {
    use apinata_core::AuthSpec;
    let moved = |source: &str| {
        [
            "curl auth.",
            "curl headers[",
            "curl url",
            "curl params[",
            "curl body[",
        ]
        .iter()
        .any(|prefix| source.starts_with(prefix))
    };
    let (revealed, kept) = std::mem::take(&mut parsed.request.secrets)
        .into_iter()
        .partition::<Vec<_>, _>(|s| moved(&s.source));
    parsed.request.secrets = kept;
    parsed.notes.retain(|n| {
        n.kind != apinata_core::import::NoteKind::NeedsInput
            || !revealed.iter().any(|s| s.source == n.source)
    });
    let put_back = |value: &mut String| {
        for s in &revealed {
            *value = value.replace(&format!("{{{{{}}}}}", s.name), &s.value);
        }
    };
    match &mut parsed.request.auth {
        AuthSpec::Basic { password, .. }
        | AuthSpec::Digest { password, .. }
        | AuthSpec::Ntlm { password, .. } => put_back(password),
        AuthSpec::Bearer { token } => put_back(token),
        _ => {}
    }
    for header in &mut parsed.request.headers {
        put_back(&mut header.value);
    }
    put_back(&mut parsed.request.url);
    for param in &mut parsed.request.params {
        put_back(&mut param.value);
    }
    parsed
}
