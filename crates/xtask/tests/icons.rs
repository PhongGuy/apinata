use std::path::PathBuf;
use std::sync::OnceLock;

use xtask::icons::{
    Export, ICO_SIZES, PNG_SIZES, icon_dir, render_exports, source_for, stale, write,
};

/// Rendering is slow in a debug build, so the tests share one set.
fn exports() -> &'static [Export] {
    static EXPORTS: OnceLock<Vec<Export>> = OnceLock::new();
    EXPORTS.get_or_init(|| render_exports(&icon_dir()).expect("icons render"))
}

fn export(path: &str) -> &'static Export {
    exports()
        .iter()
        .find(|e| e.path == std::path::Path::new(path))
        .expect(path)
}

#[test]
fn each_size_uses_the_svg_claude_md_assigns() {
    assert_eq!(source_for(16), "apinata-16.svg");
    for size in [24, 32, 48] {
        assert_eq!(source_for(size), "apinata-32.svg", "{size}");
    }
    for size in [64, 128, 1024] {
        assert_eq!(source_for(size), "apinata.svg", "{size}");
    }
}

#[test]
fn exports_are_the_png_set_ico_and_icns() {
    let mut want: Vec<PathBuf> = PNG_SIZES
        .iter()
        .map(|s| PathBuf::from(format!("png/{s}x{s}.png")))
        .collect();
    want.push("apinata.ico".into());
    want.push("apinata.icns".into());
    let got: Vec<PathBuf> = exports().iter().map(|e| e.path.clone()).collect();
    assert_eq!(got, want);
}

#[test]
fn pngs_have_their_size_and_visible_pixels() {
    for size in PNG_SIZES {
        let e = export(&format!("png/{size}x{size}.png"));
        let pixmap = resvg::tiny_skia::Pixmap::decode_png(&e.bytes).expect("valid png");
        assert_eq!((pixmap.width(), pixmap.height()), (size, size));
        assert!(
            pixmap.pixels().iter().any(|p| p.alpha() == 255),
            "{size} has no opaque pixel"
        );
    }
}

#[test]
fn ico_holds_every_ico_size() {
    let dir = ico::IconDir::read(std::io::Cursor::new(&export("apinata.ico").bytes)).unwrap();
    let widths: Vec<u32> = dir.entries().iter().map(|e| e.width()).collect();
    assert_eq!(widths, ICO_SIZES);
    for entry in dir.entries() {
        entry.decode().expect("entry decodes");
    }
}

#[test]
fn icns_holds_ten_slots_up_to_1024() {
    let family =
        icns::IconFamily::read(std::io::Cursor::new(&export("apinata.icns").bytes)).unwrap();
    assert_eq!(family.available_icons().len(), 10);
    let big = family
        .get_icon_with_type(icns::IconType::RGBA32_512x512_2x)
        .unwrap();
    assert_eq!((big.width(), big.height()), (1024, 1024));
}

#[test]
fn rendering_is_deterministic() {
    assert_eq!(render_exports(&icon_dir()).unwrap(), exports());
}

#[test]
fn stale_reports_changed_missing_and_extra_files() {
    let dir = tempfile::tempdir().unwrap();
    let generated = dir.path().join("generated");
    write(&generated, exports()).unwrap();
    assert!(stale(&generated, exports()).unwrap().is_empty());

    std::fs::write(generated.join("apinata.ico"), b"hand edit").unwrap();
    std::fs::remove_file(generated.join("png/16x16.png")).unwrap();
    std::fs::write(generated.join("png/stray.png"), b"x").unwrap();
    assert_eq!(
        stale(&generated, exports()).unwrap(),
        [
            PathBuf::from("apinata.ico"),
            "png/16x16.png".into(),
            "png/stray.png".into()
        ]
    );

    write(&generated, exports()).unwrap();
    assert!(
        stale(&generated, exports()).unwrap().is_empty(),
        "write removes strays"
    );
}

// Raster output is pinned for the platform CI checks on; other targets may round differently.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn committed_exports_are_current() {
    let stale = stale(&icon_dir().join("generated"), exports()).unwrap();
    assert!(
        stale.is_empty(),
        "run `cargo xtask icons` and commit: {stale:?}"
    );
}
