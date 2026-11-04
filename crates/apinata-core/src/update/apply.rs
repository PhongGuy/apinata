//! Staging an app update beside its install, and handing the verified file to the install.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::install::failed;
use super::{
    Available, Format, InstallKind, Reason, Relaunch, Staged, UpdateError, download,
    install_standalone,
};

/// Where an app update for `kind` is staged: beside the AppImage or `.app` bundle, so the
/// swap is a same-directory rename, or the system temp directory for the NSIS installer.
/// Kinds the app does not install are [`UpdateError::NotOwned`].
pub fn app_staging_dir(kind: &InstallKind) -> Result<PathBuf, UpdateError> {
    let unrecognised = UpdateError::NotOwned(Reason::Unrecognised);
    match kind {
        InstallKind::AppImage(p) | InstallKind::MacApp(p) => p
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .ok_or(unrecognised),
        InstallKind::WindowsNsis => Ok(std::env::temp_dir()),
        InstallKind::NotOwned(r) => Err(UpdateError::NotOwned(*r)),
        _ => Err(unrecognised),
    }
}

/// Downloads and verifies `a` into the staging directory for `kind`. A release whose format
/// does not fit the install is [`UpdateError::ManifestInvalid`] before any request is made.
pub async fn stage_app(
    kind: &InstallKind,
    a: &Available,
    progress: impl FnMut(u64, u64),
) -> Result<Staged, UpdateError> {
    let fits = matches!(
        (kind, a.format),
        (InstallKind::AppImage(_), Format::AppImage)
            | (InstallKind::MacApp(_), Format::App)
            | (InstallKind::WindowsNsis, Format::Nsis)
    );
    if !fits {
        return Err(UpdateError::ManifestInvalid(format!(
            "a {:?} release does not fit this install",
            a.format
        )));
    }
    download(a, &app_staging_dir(kind)?, progress).await
}

/// Installs the verified `staged` release for `kind` and says how the new app starts. Nothing
/// is started here. On failure, or for a kind the app does not install, the staged file is removed.
pub fn install_app(kind: &InstallKind, staged: Staged) -> Result<Relaunch, UpdateError> {
    match kind {
        InstallKind::AppImage(p) => {
            install_standalone(staged, p)?;
            Ok(Relaunch::Restart(p.clone()))
        }
        InstallKind::WindowsNsis => {
            // Windows runs only a file named like an executable, so the installer gets an
            // `.exe` name beside where it was staged; `/P /R` is passive and relaunches.
            let regular = std::fs::symlink_metadata(&staged.path).is_ok_and(|m| m.is_file());
            if !regular {
                // Removing a link removes only the link; a directory is never removed here.
                let _ = std::fs::remove_file(&staged.path);
                return Err(UpdateError::Io {
                    path: staged.path,
                    message: "the staged update is not a regular file".into(),
                });
            }
            let name = format!("apinata-update-{}-setup.exe", staged.version);
            let program = staged.path.with_file_name(name);
            std::fs::rename(&staged.path, &program).map_err(|e| {
                let _ = std::fs::remove_file(&staged.path);
                failed(&program, &program, e)
            })?;
            let args = vec![OsString::from("/P"), OsString::from("/R")];
            Ok(Relaunch::Installer { program, args })
        }
        InstallKind::MacApp(bundle) => {
            install_bundle(&staged, bundle)?;
            Ok(Relaunch::Restart(bundle.clone()))
        }
        other => {
            let _ = std::fs::remove_file(&staged.path);
            Err(app_staging_dir(other)
                .err()
                .unwrap_or(UpdateError::NotOwned(Reason::Unrecognised)))
        }
    }
}

/// Extracts the staged `.app` tarball beside `bundle` and swaps it in under the bundle's own
/// name. Any failure removes the work directory and the staged file and leaves `bundle` as it was.
fn install_bundle(staged: &Staged, bundle: &Path) -> Result<(), UpdateError> {
    let parent = bundle.parent().filter(|d| !d.as_os_str().is_empty());
    let beside = parent.is_some() && staged.path.parent() == parent && staged.path != bundle;
    let regular = std::fs::symlink_metadata(&staged.path).is_ok_and(|m| m.is_file());
    let Some(parent) = parent.filter(|_| beside && regular) else {
        let _ = std::fs::remove_file(&staged.path);
        return Err(UpdateError::Io {
            path: staged.path.clone(),
            message: "the staged update is not a file beside the install target".into(),
        });
    };
    let work = parent.join(format!(".apinata-update-{}", staged.version));
    let aside = parent.join(format!(".apinata-old-{}.app", staged.version));
    // The aside copy is spare only while a bundle is in place; if an earlier swap failed in
    // both directions it is the only original left, so it is never removed then.
    let in_place = || std::fs::symlink_metadata(bundle).is_ok_and(|m| m.is_dir());
    let result = extract_bundle(&staged.path, &work, bundle).and_then(|new| {
        if in_place() {
            remove_tree(&aside);
        }
        swap_dirs(&new, bundle, &aside)
    });
    if in_place() {
        remove_tree(&aside);
    }
    remove_tree(&work);
    let _ = std::fs::remove_file(&staged.path);
    result
}

/// Unpacks the gzip tarball at `archive` into a fresh `work` directory and returns the bundle
/// it holds: the first entry's top-level directory. Archive errors name the staged file.
fn extract_bundle(archive: &Path, work: &Path, bundle: &Path) -> Result<PathBuf, UpdateError> {
    remove_tree(work);
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut dir = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut dir, 0o700);
    dir.create(work).map_err(|e| failed(work, bundle, e))?;

    let bad = |message: String| UpdateError::Io {
        path: archive.to_path_buf(),
        message,
    };
    let file = std::fs::File::open(archive).map_err(|e| bad(e.to_string()))?;
    // Permissions are not preserved, so tar keeps only 0o777 and drops setuid bits; the mask
    // also clears group and other write, so no installed file is writable by others.
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    tar.set_mask(0o022);
    let mut top: Option<OsString> = None;
    for entry in tar.entries().map_err(|e| bad(e.to_string()))? {
        let mut entry = entry.map_err(|e| bad(e.to_string()))?;
        check_entry(&entry, &mut top).map_err(|m| bad(m.into()))?;
        if !entry.unpack_in(work).map_err(|e| bad(e.to_string()))? {
            return Err(bad("the update has a path outside its bundle".into()));
        }
    }
    top.map(|t| work.join(t))
        .filter(|t| std::fs::symlink_metadata(t).is_ok_and(|m| m.is_dir()))
        .ok_or_else(|| bad("the update holds no bundle directory".into()))
}

/// Refuses an entry that could write outside the bundle or replace it: tar's `unpack_in`
/// strips a leading `/`, skips `..` silently and never checks a link's target. The first
/// entry must be the `*.app` directory, recorded in `top`; every later entry lies inside it.
fn check_entry<R: std::io::Read>(
    entry: &tar::Entry<'_, R>,
    top: &mut Option<OsString>,
) -> Result<(), &'static str> {
    use std::path::Component;
    // Every component plain, so a path or link target can only go down.
    let normal = |p: &Path| -> Option<Vec<OsString>> {
        let parts: Option<Vec<OsString>> = p
            .components()
            .map(|c| match c {
                Component::Normal(n) => Some(n.to_os_string()),
                _ => None,
            })
            .collect();
        parts.filter(|v| !v.is_empty())
    };
    let inside = |parts: &[OsString], top: &Option<OsString>| {
        parts.len() > 1 && top.as_ref() == parts.first()
    };
    let path = entry
        .path()
        .map_err(|_| "the update has an unreadable path")?;
    let parts = normal(&path).ok_or("the update has a path outside its bundle")?;
    let kind = entry.header().entry_type();
    if top.is_none() {
        let name = parts[0].to_string_lossy();
        if !kind.is_dir() || parts.len() != 1 || !name.ends_with(".app") {
            return Err("the update does not start with an app bundle directory");
        }
        *top = Some(parts[0].clone());
        return Ok(());
    }
    if !inside(&parts, top) {
        return Err("the update has a path outside its bundle");
    }
    if kind.is_dir() || kind.is_file() {
        return Ok(());
    }
    if !(kind.is_symlink() || kind.is_hard_link()) {
        return Err("the update has an entry that is not a file, directory or link");
    }
    let target = entry
        .link_name()
        .ok()
        .flatten()
        .ok_or("the update has a link without a target")?;
    // A symlink resolves from its own directory, a hard link from the archive root.
    // Symlink targets with `..` are refused on purpose; a bundle with frameworks linking
    // upward would need this rule revisited.
    let ok = match normal(&target) {
        Some(_) if kind.is_symlink() => true,
        Some(t) => inside(&t, top),
        None => false,
    };
    if ok {
        Ok(())
    } else {
        Err("the update has a link that leaves its bundle")
    }
}

/// Moves `target` to `aside` and `new` into its place. If `new` cannot take the place, the
/// original goes back, so `target` is never left missing; the error names `target`.
fn swap_dirs(new: &Path, target: &Path, aside: &Path) -> Result<(), UpdateError> {
    std::fs::rename(target, aside).map_err(|e| failed(target, target, e))?;
    std::fs::rename(new, target).map_err(|e| {
        let _ = std::fs::rename(aside, target);
        failed(target, target, e)
    })
}

/// Best-effort removal of `path` without following a link at it.
fn remove_tree(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => {
            allow_owner_write(path, &m);
            drop(std::fs::remove_dir_all(path));
        }
        Ok(_) => drop(std::fs::remove_file(path)),
        Err(_) => {}
    }
}

/// Gives every real directory under `dir` owner write, so an update carrying a read-only
/// directory can still be emptied. Links are never followed; `meta` is `dir`'s own lstat.
fn allow_owner_write(dir: &Path, meta: &std::fs::Metadata) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        if mode & 0o700 != 0o700 {
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode | 0o700));
        }
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if let Ok(m) = std::fs::symlink_metadata(entry.path())
                && m.is_dir()
            {
                allow_owner_write(&entry.path(), &m);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = (dir, meta);
}

/// Starts the installer without waiting for it; a failure to start is [`UpdateError::Io`].
pub fn start_installer(program: &Path, args: &[OsString]) -> Result<(), UpdateError> {
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(drop)
        .map_err(|e| UpdateError::Io {
            path: program.to_path_buf(),
            message: e.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::swap_dirs;

    #[test]
    fn a_failed_swap_restores_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let (bundle, aside) = (dir.path().join("Apinata.app"), dir.path().join("old.app"));
        std::fs::create_dir(&bundle).unwrap();
        std::fs::write(bundle.join("marker"), b"original").unwrap();
        let missing = dir.path().join("work/Apinata.app");
        assert!(swap_dirs(&missing, &bundle, &aside).is_err());
        assert_eq!(std::fs::read(bundle.join("marker")).unwrap(), b"original");
        assert!(!aside.exists());
    }
}
