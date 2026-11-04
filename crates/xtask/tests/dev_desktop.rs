//! `cargo xtask dev-desktop` gives a dev build the same desktop identity as a package.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use xtask::dev_desktop::{
    APP_ID, data_home_from, desktop_entry, install, install_named, installed_entry, remove,
};

fn icon() -> PathBuf {
    xtask::workspace_root().join("assets/icon/generated/png/256x256.png")
}

#[test]
fn entry_points_at_the_binary_and_names_the_window_class() {
    let e = desktop_entry(Path::new("/w/target/debug/apinata-app"));
    let lines: Vec<&str> = e.lines().collect();
    assert_eq!(lines[0], "[Desktop Entry]");
    for want in [
        "Type=Application",
        "Name=Apinata (dev)",
        "Exec=/w/target/debug/apinata-app",
        "Icon=apinata-app",
        "StartupWMClass=apinata-app",
        "Terminal=false",
        "Categories=Development;",
    ] {
        assert!(lines.contains(&want), "{want} missing from:\n{e}");
    }
}

// Exec quoting backslash-escapes `"`, `` ` ``, `$` and `\`, and `%` doubles; the string-value
// layer the spec unescapes first then doubles every backslash once more.
#[test]
fn exec_escapes_reserved_characters_and_percent() {
    let e = desktop_entry(Path::new("/a\"$`\\b/50%/apinata-app"));
    assert!(
        e.lines()
            .any(|l| l == r#"Exec="/a\\"\\$\\`\\\\b/50%%/apinata-app""#),
        "{e}"
    );
}

#[test]
fn a_binary_path_with_spaces_is_quoted() {
    let e = desktop_entry(Path::new("/my dir/apinata-app"));
    assert!(
        e.lines().any(|l| l == r#"Exec="/my dir/apinata-app""#),
        "{e}"
    );
}

#[test]
fn install_writes_the_entry_and_icon_and_remove_undoes_it() {
    let tmp = tempfile::tempdir().unwrap();
    let share = tmp.path().join("share");
    let bin = tmp.path().join("apinata-app");
    let entry = share.join("applications/apinata-app.desktop");
    let png = share.join("icons/hicolor/256x256/apps/apinata-app.png");
    assert_eq!(
        install(&share, &bin, &icon()).unwrap(),
        vec![entry.clone(), png.clone()]
    );
    assert_eq!(fs::read_to_string(&entry).unwrap(), desktop_entry(&bin));
    assert_eq!(fs::read(&png).unwrap(), fs::read(icon()).unwrap());
    install(&share, &bin, &icon()).expect("a second install overwrites in place");
    assert_eq!(remove(&share).unwrap(), vec![entry.clone(), png.clone()]);
    assert!(!entry.exists() && !png.exists());
    assert!(remove(&share).unwrap().is_empty(), "remove is idempotent");
}

#[test]
fn remove_leaves_other_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let other = tmp.path().join("applications/other.desktop");
    fs::create_dir_all(other.parent().unwrap()).unwrap();
    fs::write(&other, "[Desktop Entry]\n").unwrap();
    install(tmp.path(), Path::new("/b/apinata-app"), &icon()).unwrap();
    remove(tmp.path()).unwrap();
    assert!(other.exists());
}

#[test]
fn data_home_follows_the_xdg_rules() {
    let s = |v: &str| Some(OsString::from(v));
    assert_eq!(
        data_home_from(s("/x/share"), s("/h")).unwrap(),
        PathBuf::from("/x/share")
    );
    assert_eq!(
        data_home_from(s("relative"), s("/h")).unwrap(),
        PathBuf::from("/h/.local/share")
    );
    assert_eq!(
        data_home_from(s(""), s("/h")).unwrap(),
        PathBuf::from("/h/.local/share")
    );
    assert_eq!(
        data_home_from(None, s("/h")).unwrap(),
        PathBuf::from("/h/.local/share")
    );
    assert!(data_home_from(None, None).is_err());
}

#[test]
fn app_id_matches_the_app() {
    let src =
        fs::read_to_string(xtask::workspace_root().join("crates/apinata-app/src/identity.rs"))
            .unwrap();
    assert!(src.contains(&format!("pub const APP_ID: &str = \"{APP_ID}\";")));
}

#[test]
fn a_named_entry_differs_from_the_dev_entry_only_in_its_name() {
    let bin = std::path::Path::new("/opt/x/apinata-app");
    let named = xtask::dev_desktop::desktop_entry_named(bin, "Apinata");
    assert!(named.contains("\nName=Apinata\n"));
    assert_eq!(
        named.replace("Name=Apinata\n", "Name=Apinata (dev)\n"),
        xtask::dev_desktop::desktop_entry(bin)
    );
}

#[test]
fn the_desktop_subcommand_installs_and_removes_under_the_given_data_home() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path().join("share");
    let run = |extra: &[&str]| {
        let out = std::process::Command::new(env!("CARGO"))
            .args(["xtask", "desktop", "--data-home"])
            .arg(&home)
            .args(["--binary", "/opt/apinata/bin/apinata-app"])
            .args(extra)
            .current_dir(xtask::workspace_root())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&[]);
    let entry = std::fs::read_to_string(home.join("applications/apinata-app.desktop")).unwrap();
    assert!(
        entry.contains("\nName=Apinata\n")
            && entry.contains("\nExec=/opt/apinata/bin/apinata-app\n")
    );
    assert!(
        home.join("icons/hicolor/256x256/apps/apinata-app.png")
            .is_file()
    );
    run(&["--remove"]);
    assert!(!home.join("applications/apinata-app.desktop").exists());
    assert!(
        !home
            .join("icons/hicolor/256x256/apps/apinata-app.png")
            .exists()
    );
}

/// Runs `cargo xtask dev-desktop` with `extra` args, its data home and HOME both under `tmp`.
fn dev_desktop(tmp: &Path, extra: &[&str]) -> String {
    let out = std::process::Command::new(env!("CARGO"))
        .args(["xtask", "dev-desktop"])
        .args(extra)
        .env("XDG_DATA_HOME", tmp.join("share"))
        .env("HOME", tmp.join("home"))
        .current_dir(xtask::workspace_root())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn only_an_entry_with_another_name_counts_as_installed() {
    let tmp = tempfile::tempdir().unwrap();
    let share = tmp.path();
    assert_eq!(installed_entry(share), None, "no entry at all");
    install(share, Path::new("/w/target/debug/apinata-app"), &icon()).unwrap();
    assert_eq!(
        installed_entry(share),
        None,
        "the dev entry is not installed"
    );
    let bin = Path::new("/usr/local/bin/apinata-app");
    install_named(share, bin, &icon(), "Apinata").unwrap();
    assert_eq!(
        installed_entry(share),
        Some(share.join("applications/apinata-app.desktop"))
    );
}

#[test]
fn dev_desktop_leaves_an_installed_entry_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let share = tmp.path().join("share");
    let bin = Path::new("/usr/local/bin/apinata-app");
    install_named(&share, bin, &icon(), "Apinata").unwrap();
    let entry = share.join("applications/apinata-app.desktop");
    let installed = fs::read_to_string(&entry).unwrap();

    let said = dev_desktop(tmp.path(), &[]);
    assert_eq!(fs::read_to_string(&entry).unwrap(), installed, "{said}");
    assert!(said.contains(&entry.display().to_string()), "{said}");

    dev_desktop(tmp.path(), &["--remove"]);
    assert_eq!(fs::read_to_string(&entry).unwrap(), installed);
    assert!(
        share
            .join("icons/hicolor/256x256/apps/apinata-app.png")
            .is_file(),
        "the installed icon stays too"
    );
    assert!(!tmp.path().join("home").exists(), "nothing went to HOME");
}
