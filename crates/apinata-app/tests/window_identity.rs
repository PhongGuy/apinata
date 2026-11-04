//! The window's identity matches what the Linux packages install, so the desktop shell finds
//! the Apinata icon for it.

use apinata_app::app::window_options;
use apinata_app::identity::{APP_ID, window_icon};
use apinata_app::session::Session;
use std::path::Path;
use std::sync::Arc;

fn repo(rel: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join(rel)).unwrap()
}

#[test]
fn app_id_is_the_desktop_file_stem() {
    assert_eq!(APP_ID, "apinata-app");
    assert_eq!(
        window_options(&Session::default(), &[]).app_id.as_deref(),
        Some(APP_ID)
    );
}

#[test]
fn window_title_is_apinata() {
    let opts = window_options(&Session::default(), &[]);
    assert_eq!(opts.titlebar.and_then(|t| t.title), Some("Apinata".into()));
}

#[test]
fn packaging_checks_files_named_after_the_app_id() {
    let script = repo("scripts/package-linux.sh");
    assert!(script.contains(&format!("usr/share/applications/{APP_ID}.desktop")));
    assert!(script.contains(&format!(
        "usr/share/icons/hicolor/256x256/apps/{APP_ID}.png"
    )));
}

#[test]
fn the_main_binary_is_named_after_the_app_id() {
    // cargo-packager names the desktop file and icon after the main binary.
    let manifest = repo("crates/apinata-app/Cargo.toml");
    assert!(manifest.contains(&format!(r#"{{ path = "{APP_ID}", main = true }}"#)));
}

#[test]
fn the_desktop_template_names_the_app_id_as_window_class() {
    let template = repo("crates/apinata-app/packaging/apinata-app.desktop");
    assert!(
        template
            .lines()
            .any(|l| l == format!("StartupWMClass={APP_ID}"))
    );
}

#[test]
fn the_window_icon_is_the_committed_256_px_png() {
    let icon = window_icon().expect("the committed PNG decodes");
    let png =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icon/generated/png/256x256.png");
    let expected = image::load_from_memory(&std::fs::read(png).unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(icon.dimensions(), (256, 256));
    assert_eq!(icon.dimensions(), expected.dimensions());
    assert!(
        icon.as_raw() == expected.as_raw(),
        "pixels match the committed PNG"
    );
}

#[test]
fn the_icon_is_decoded_once_and_set_on_the_window() {
    let icon = window_icon().unwrap();
    assert!(Arc::ptr_eq(&icon, &window_icon().unwrap()), "decoded once");
    let opts = window_options(&Session::default(), &[]);
    assert!(Arc::ptr_eq(opts.icon.as_ref().unwrap(), &icon));
}
