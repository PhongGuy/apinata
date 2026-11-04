//! Removing one overlay entry: only that name goes, and the file value shows through again.

use apinata_core::{EnvOverlay, OverlayLayer};

#[test]
fn remove_drops_one_entry_in_each_layer_and_keeps_the_rest() {
    let o = EnvOverlay::new();
    for layer in [
        OverlayLayer::Environment,
        OverlayLayer::Collection,
        OverlayLayer::Global,
    ] {
        o.set_text(layer, "a", "1");
        o.set_text(layer, "b", "2");
        assert!(o.remove(layer, "a"));
        let left: Vec<_> = o
            .entries()
            .into_iter()
            .filter(|v| v.layer() == layer)
            .map(|v| v.name().to_owned())
            .collect();
        assert_eq!(left, ["b"], "{layer:?}");
    }
}

#[test]
fn remove_reports_whether_the_name_was_there() {
    let o = EnvOverlay::new();
    o.set_text(OverlayLayer::Global, "a", "1");
    assert!(!o.remove(OverlayLayer::Collection, "a"));
    assert!(!o.remove(OverlayLayer::Global, "missing"));
    assert!(o.remove(OverlayLayer::Global, "a"));
    assert!(!o.remove(OverlayLayer::Global, "a"));
    assert!(o.is_empty());
}
