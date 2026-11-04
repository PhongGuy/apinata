//! Which installs the updater may replace, and the swap that replaces a standalone one.

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::{InstallKind, Os, Product, Reason, Staged, UpdateError};

/// Classifies the install `exe` belongs to, applying the rules in order. Pure: the
/// caller canonicalizes `exe`, passes `$APPIMAGE` as `appimage_env`, and sets `sibling_app`
/// when an `apinata-app[.exe]` sits beside `exe`.
pub fn install_kind(
    product: Product,
    os: Os,
    exe: &Path,
    appimage_env: Option<&Path>,
    sibling_app: bool,
) -> InstallKind {
    let text = exe.to_string_lossy();
    let bundle = mac_bundle(exe);

    // A path the caller failed to canonicalize could disguise a package-managed binary,
    // so it is never replaced.
    if !canonical(os, &text) {
        return InstallKind::NotOwned(Reason::Unrecognised);
    }
    if os == Os::Linux && exe.starts_with("/usr") && !exe.starts_with("/usr/local") {
        return InstallKind::NotOwned(Reason::SystemPackage);
    }
    match product {
        Product::Cli => {
            let in_appimage = os == Os::Linux && appimage_env.is_some();
            let in_bundle = os == Os::Macos && inside_app_bundle(exe);
            if in_appimage || in_bundle || sibling_app {
                InstallKind::NotOwned(Reason::ShippedWithApp)
            } else {
                InstallKind::Standalone(exe.to_path_buf())
            }
        }
        Product::App => match (os, appimage_env, bundle) {
            (Os::Macos, _, _) if text.starts_with("/Volumes/") => {
                InstallKind::NotOwned(Reason::ReadOnlyVolume)
            }
            (Os::Macos, _, _) if text.contains("/AppTranslocation/") => {
                InstallKind::NotOwned(Reason::Translocated)
            }
            (Os::Linux, Some(appimage), _) => InstallKind::AppImage(appimage.to_path_buf()),
            (Os::Macos, _, Some(bundle)) => InstallKind::MacApp(bundle),
            (Os::Windows, _, _) => InstallKind::WindowsNsis,
            _ => InstallKind::NotOwned(Reason::Unrecognised),
        },
    }
}

/// Whether `text` is absolute for `os` and free of `.` and `..` segments. Judged on the text
/// rather than the host's path rules, since `os` need not be the host.
fn canonical(os: Os, text: &str) -> bool {
    let (absolute, separators): (bool, &[char]) = match os {
        Os::Windows => {
            let b = text.as_bytes();
            let drive = b.len() >= 3
                && b[0].is_ascii_alphabetic()
                && b[1] == b':'
                && matches!(b[2], b'\\' | b'/');
            (drive || text.starts_with(r"\\"), &['\\', '/'])
        }
        _ => (text.starts_with('/'), &['/']),
    };
    absolute && !text.split(separators).any(|seg| seg == "." || seg == "..")
}

/// Whether any directory above `exe` is a `.app` bundle, at any depth.
fn inside_app_bundle(exe: &Path) -> bool {
    exe.ancestors().skip(1).any(|dir| {
        dir.file_name().is_some_and(|name| {
            name.to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".app")
        })
    })
}

/// The `X.app` directory when `exe` sits at `X.app/Contents/MacOS/<exe>`.
fn mac_bundle(exe: &Path) -> Option<PathBuf> {
    let named = |dir: &&Path, name: &str| dir.file_name() == Some(OsStr::new(name));
    let macos = exe.parent().filter(|d| named(d, "MacOS"))?;
    let contents = macos.parent().filter(|d| named(d, "Contents"))?;
    let app = contents.parent()?;
    let is_app = app.file_name()?.to_string_lossy().ends_with(".app");
    is_app.then(|| app.to_path_buf())
}

/// Replaces `target` with the verified `staged` file. On any failure `target` is unchanged and
/// the staged file is removed (best effort when its directory is read-only), except that a
/// staged path which is not a separate regular file beside `target` is refused and left in place.
pub fn install_standalone(staged: Staged, target: &Path) -> Result<(), UpdateError> {
    // `Staged.path` is a public field, so refuse anything but a regular file beside the target:
    // that is where `download` stages, and it keeps the swap a same-directory rename. The target
    // itself is refused, since failure cleanup would then delete it.
    let beside = staged.path.parent().is_some()
        && staged.path.parent() == target.parent()
        && staged.path != target;
    let regular = std::fs::symlink_metadata(&staged.path).is_ok_and(|m| m.is_file());
    if !beside || !regular {
        return Err(UpdateError::Io {
            path: staged.path,
            message: "the staged update is not a file beside the install target".into(),
        });
    }
    let result = swap(&staged.path, target);
    if result.is_err() {
        let _ = std::fs::remove_file(&staged.path);
    }
    result
}

/// Unix: give the staged file the target's permission bits (never setuid, setgid or sticky),
/// then one atomic `rename` over it.
#[cfg(unix)]
fn swap(staged: &Path, target: &Path) -> Result<(), UpdateError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(target)
        .map_err(|e| failed(target, target, e))?
        .permissions()
        .mode();
    std::fs::set_permissions(staged, std::fs::Permissions::from_mode(mode & 0o777))
        .map_err(|e| failed(staged, target, e))?;
    std::fs::rename(staged, target).map_err(|e| failed(target, target, e))
}

/// Windows cannot rename over a running executable: `self-replace` handles the running one,
/// and any other target is moved aside, then restored if the staged file cannot take its place.
#[cfg(windows)]
fn swap(staged: &Path, target: &Path) -> Result<(), UpdateError> {
    let running = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .ok()
        .zip(std::fs::canonicalize(target).ok())
        .is_some_and(|(exe, t)| exe == t);
    if running {
        self_replace::self_replace(staged).map_err(|e| failed(target, target, e))?;
        let _ = std::fs::remove_file(staged);
        return Ok(());
    }
    let mut old = target.as_os_str().to_owned();
    old.push(".old");
    let old = PathBuf::from(old);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(target, &old).map_err(|e| failed(target, target, e))?;
    std::fs::rename(staged, target).map_err(|e| {
        let _ = std::fs::rename(&old, target);
        failed(target, target, e)
    })?;
    let _ = std::fs::remove_file(&old);
    Ok(())
}

/// A permission failure means the target's directory cannot be written; anything else names
/// the path that failed.
pub(crate) fn failed(path: &Path, target: &Path, e: std::io::Error) -> UpdateError {
    if e.kind() == ErrorKind::PermissionDenied {
        let dir = target.parent().unwrap_or(target);
        return UpdateError::NotWritable(dir.to_path_buf());
    }
    UpdateError::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    }
}
