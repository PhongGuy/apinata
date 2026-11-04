//! The spacing scale matches the design and the theme, and the extra icons load beside the
//! kit's default set.

use apinata_app::icons::{AppAssets, EXTRA};
use apinata_app::style::*;
use apinata_app::theme::THEME_JSON;
use gpui_kit::assets::{Assets, IconName};
use gpui_kit::{AssetSource as _, px};

#[test]
fn the_scale_matches_the_spec() {
    assert_eq!(
        [
            at_zoom(SPACE_1, 1.0),
            at_zoom(SPACE_2, 1.0),
            at_zoom(SPACE_3, 1.0),
            at_zoom(SPACE_4, 1.0),
            at_zoom(SPACE_5, 1.0)
        ],
        [px(4.), px(8.), px(12.), px(16.), px(24.)]
    );
    assert_eq!(
        (
            at_zoom(ROW_HEIGHT, 1.0),
            at_zoom(TITLE_BAR_HEIGHT, 1.0),
            at_zoom(STATUS_BAR_HEIGHT, 1.0)
        ),
        (px(28.), px(36.), px(24.))
    );
    assert_eq!(
        (TEXT_UI.to_pixels(px(13.)), TEXT_SMALL.to_pixels(px(13.))),
        (px(13.), px(12.))
    );
}

#[test]
fn the_theme_agrees_with_the_constants() {
    let set: serde_json::Value = serde_json::from_str(THEME_JSON).unwrap();
    for t in set["themes"].as_array().unwrap() {
        assert_eq!(px(t["radius"].as_f64().unwrap() as f32), RADIUS);
        assert_eq!(px(t["radius.lg"].as_f64().unwrap() as f32), POPOVER_RADIUS);
        assert_eq!(
            px(t["font.size"].as_f64().unwrap() as f32),
            TEXT_UI.to_pixels(px(13.))
        );
        assert_eq!(
            (t["font.family"].as_str(), t["mono_font.family"].as_str()),
            (Some("Inter"), Some("JetBrains Mono"))
        );
        assert!(
            t.get("mono_font.size").is_none(),
            "settings own the editor font size"
        );
    }
}

#[test]
fn extra_icons_load_and_the_defaults_still_do() {
    for icon in EXTRA {
        let path = icon.path();
        assert!(matches!(AppAssets.load(&path), Ok(Some(_))), "{path}");
        assert!(
            Assets.load(&path).is_err(),
            "{path} is already a default icon; drop it from EXTRA"
        );
    }
    assert!(matches!(
        AppAssets.load(&IconName::Close.path()),
        Ok(Some(_))
    ));
    assert!(!matches!(
        AppAssets.load("icons/no-such-icon.svg"),
        Ok(Some(_))
    ));
}

#[test]
fn listing_includes_both_sets() {
    let all = AppAssets.list("icons/").unwrap();
    assert!(
        all.iter()
            .any(|p| p.as_ref() == IconName::GitBranch.path().as_ref())
    );
    assert!(
        all.iter()
            .any(|p| p.as_ref() == IconName::Close.path().as_ref())
    );
}

#[test]
fn the_lock_icons_are_extras() {
    for icon in [IconName::Lock, IconName::LockOpen] {
        assert!(EXTRA.contains(&icon), "{icon:?}");
    }
}

#[test]
fn the_environment_editor_icon_is_an_extra() {
    assert!(EXTRA.contains(&IconName::SlidersHorizontal));
}

/// Lucide file stem for an `IconName` identifier: `CodeXml` is `code-xml`, `Columns2` is
/// `columns-2`.
fn icon_stem(ident: &str) -> String {
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in ident.chars() {
        let starts_word = c.is_ascii_uppercase()
            || (c.is_ascii_digit() && prev.is_some_and(|p| !p.is_ascii_digit()));
        if starts_word && prev.is_some() {
            out.push('-');
        }
        out.push(c.to_ascii_lowercase());
        prev = Some(c);
    }
    out
}

fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_stem_rule_matches_the_kit_paths() {
    for icon in EXTRA {
        let stem = icon_stem(&format!("{icon:?}"));
        assert_eq!(icon.path().as_ref(), format!("icons/{stem}.svg"));
    }
}

#[test]
fn every_icon_the_app_names_is_bundled() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let mut names = std::collections::BTreeSet::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for part in text.split("IconName::").skip(1) {
            let ident: String = part
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            if ident.starts_with(|c: char| c.is_ascii_uppercase()) {
                names.insert(ident);
            }
        }
    }
    assert!(names.len() > 30, "scan found only {} names", names.len());
    let missing: Vec<_> = names
        .iter()
        .filter(|n| {
            let path = format!("icons/{}.svg", icon_stem(n));
            !matches!(AppAssets.load(&path), Ok(Some(_)))
        })
        .collect();
    assert!(missing.is_empty(), "icons with no bundled SVG: {missing:?}");
}
