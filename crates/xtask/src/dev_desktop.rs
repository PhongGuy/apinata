//! `cargo xtask dev-desktop`: a desktop entry and icon for the debug build, so a dev run gets
//! the same window identity (`app_id`, X11 class, icon) as an installed package. Linux only in
//! effect; it compiles everywhere.

use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;

/// The app's window id and desktop file stem; must equal `apinata_app::identity::APP_ID`,
/// which this crate cannot depend on.
pub const APP_ID: &str = "apinata-app";

const ICON_DIR: &str = "icons/hicolor/256x256/apps";

/// The display name of the dev build's entry.
const DEV_NAME: &str = "Apinata (dev)";

/// The dev desktop entry for `binary`: `desktop_entry_named` with Name "Apinata (dev)".
pub fn desktop_entry(binary: &Path) -> String {
    desktop_entry_named(binary, DEV_NAME)
}

/// A desktop entry for `binary` shown as `name`: Type, Name, Exec (quoted when needed),
/// Icon and StartupWMClass both APP_ID, Terminal=false, Categories=Development;
pub fn desktop_entry_named(binary: &Path, name: &str) -> String {
    let exec = exec_arg(&binary.to_string_lossy());
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nIcon={APP_ID}\n\
         StartupWMClass={APP_ID}\nTerminal=false\nCategories=Development;\n"
    )
}

/// Quotes `path` per the desktop entry spec's Exec rules, then applies the string-value escapes
/// that the spec unescapes first; `%` is doubled so it never reads as a field code.
fn exec_arg(path: &str) -> String {
    const RESERVED: &str = " \t\n\"'`$\\<>~|&;*?#()";
    let arg = if path.chars().any(|c| RESERVED.contains(c)) {
        let mut quoted = String::from("\"");
        for c in path.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                quoted.push('\\');
            }
            quoted.push(c);
        }
        quoted.push('"');
        quoted
    } else {
        path.to_owned()
    };
    arg.replace('%', "%%")
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
}

fn targets(data_home: &Path) -> [PathBuf; 2] {
    [
        data_home.join(format!("applications/{APP_ID}.desktop")),
        data_home.join(format!("{ICON_DIR}/{APP_ID}.png")),
    ]
}

/// Writes `<data_home>/applications/APP_ID.desktop` and copies `icon_png` to
/// `<data_home>/icons/hicolor/256x256/apps/APP_ID.png`, creating directories; returns both paths.
pub fn install(data_home: &Path, binary: &Path, icon_png: &Path) -> anyhow::Result<Vec<PathBuf>> {
    install_named(data_home, binary, icon_png, DEV_NAME)
}

/// `install` with the entry shown as `name`.
pub fn install_named(
    data_home: &Path,
    binary: &Path,
    icon_png: &Path,
    name: &str,
) -> anyhow::Result<Vec<PathBuf>> {
    binary
        .to_str()
        .with_context(|| format!("binary path is not UTF-8: {}", binary.display()))?;
    let [entry, png] = targets(data_home);
    for path in [&entry, &png] {
        let dir = path.parent().context("target has no directory")?;
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    fs::write(&entry, desktop_entry_named(binary, name))
        .with_context(|| format!("writing {}", entry.display()))?;
    fs::copy(icon_png, &png)
        .with_context(|| format!("copying {} to {}", icon_png.display(), png.display()))?;
    Ok(vec![entry, png])
}

/// Removes those two files if present; returns the ones removed. Nothing else is touched.
pub fn remove(data_home: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    for path in targets(data_home) {
        match fs::remove_file(&path) {
            Ok(()) => removed.push(path),
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err).with_context(|| format!("removing {}", path.display())),
        }
    }
    Ok(removed)
}

/// `xdg` if absolute, else `<home>/.local/share`; errors when neither gives a path.
pub fn data_home_from(xdg: Option<OsString>, home: Option<OsString>) -> anyhow::Result<PathBuf> {
    // The XDG base directory spec says a relative value is invalid and must be ignored.
    if let Some(xdg) = xdg.map(PathBuf::from).filter(|p| p.is_absolute()) {
        return Ok(xdg);
    }
    let home = home
        .filter(|h| !h.is_empty())
        .context("neither XDG_DATA_HOME nor HOME is set")?;
    Ok(PathBuf::from(home).join(".local/share"))
}

/// The entry at `<data_home>/applications/APP_ID.desktop` when it exists and is not the dev
/// entry, i.e. a per-user one written by `cargo xtask desktop`; an unreadable or nameless file
/// counts as absent.
pub fn installed_entry(data_home: &Path) -> Option<PathBuf> {
    let [entry, _] = targets(data_home);
    let text = fs::read_to_string(&entry).ok()?;
    let name = text.lines().find_map(|l| l.strip_prefix("Name="))?;
    (name != DEV_NAME).then_some(entry)
}

/// The subcommand: install (or remove) for `<target_directory>/debug/apinata-app` and the
/// committed 256 px icon, printing what changed; does nothing but say so when an installed
/// entry is present.
pub fn run(remove: bool) -> anyhow::Result<ExitCode> {
    let data_home = data_home_from(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))?;
    // The installed entry shares the dev entry's file name (Wayland ties it to the app_id), so
    // leave it alone rather than overwrite or delete it; checked before any build metadata.
    if let Some(path) = installed_entry(&data_home) {
        println!(
            "kept the installed entry {}; uninstall it to use a dev entry",
            path.display()
        );
        return Ok(ExitCode::SUCCESS);
    }
    let changed = if remove {
        let removed = self::remove(&data_home)?;
        if removed.is_empty() {
            println!("no dev desktop entry to remove");
        }
        removed
            .iter()
            .for_each(|p| println!("removed {}", p.display()));
        removed
    } else {
        let metadata = crate::cargo_metadata()?;
        let target = metadata["target_directory"]
            .as_str()
            .context("cargo metadata lacks target_directory")?;
        let binary = Path::new(target).join("debug").join(APP_ID);
        if !binary.exists() {
            eprintln!(
                "warning: {} does not exist yet; run `cargo build -p apinata-app` first",
                binary.display()
            );
        }
        let icon = crate::workspace_root().join("assets/icon/generated/png/256x256.png");
        let written = install(&data_home, &binary, &icon)?;
        written
            .iter()
            .for_each(|p| println!("wrote {}", p.display()));
        written
    };
    if !changed.is_empty() {
        println!(
            "hint: if the launcher does not pick this up, run `update-desktop-database` \
             or `kbuildsycoca6`"
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// The `desktop` subcommand: install (or remove) an entry named `name` for `binary` under an
/// explicit `data_home`, with the committed 256 px icon. The binary need not exist yet, since an
/// installer writes the entry before or after copying it.
pub fn run_explicit(
    data_home: &Path,
    binary: &Path,
    name: &str,
    remove: bool,
) -> anyhow::Result<ExitCode> {
    if remove {
        for p in self::remove(data_home)? {
            println!("removed {}", p.display());
        }
    } else {
        let icon = crate::workspace_root().join("assets/icon/generated/png/256x256.png");
        for p in install_named(data_home, binary, &icon, name)? {
            println!("wrote {}", p.display());
        }
    }
    Ok(ExitCode::SUCCESS)
}
