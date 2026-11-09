//! WCAG 2.x contrast over every text-on-fill pair on the contrast list, in both
//! shipped themes (the two Apinata themes at AA, the two high-contrast ones at 7:1), read from the theme JSON as shipped.

use apinata_app::theme::{
    HC_DARK, HC_LIGHT, HIGH_CONTRAST_JSON, Status, THEME_JSON, status_fill_hex,
};
use gpui_kit::component::ThemeMode;
use serde_json::Value;

const STATUSES: [(&str, Status); 4] = [
    ("success", Status::Success),
    ("info", Status::Info),
    ("warning", Status::Warning),
    ("danger", Status::Danger),
];

fn channel(c: f64) -> f64 {
    if c <= 0.03928 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn lum(hex: &str) -> f64 {
    let h = hex.trim_start_matches('#');
    let c: Vec<f64> = (0..3)
        .map(|i| channel(u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap() as f64 / 255.0))
        .collect();
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn ratio(a: &str, b: &str) -> f64 {
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn themes() -> Vec<(ThemeMode, Value)> {
    [THEME_JSON, HIGH_CONTRAST_JSON]
        .into_iter()
        .flat_map(|json| {
            let set: Value = serde_json::from_str(json).unwrap();
            set["themes"].as_array().unwrap().clone()
        })
        .map(|t| {
            (
                if t["mode"] == "dark" {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                },
                t,
            )
        })
        .collect()
}

/// The text contrast floor for `t`: 7:1 for the high-contrast themes, AA otherwise.
fn floor(t: &Value) -> f64 {
    match t["name"].as_str().unwrap() {
        HC_LIGHT | HC_DARK => 7.0,
        _ => 4.5,
    }
}

fn col(t: &Value, key: &str) -> String {
    t["colors"][key]
        .as_str()
        .unwrap_or_else(|| panic!("no {key}"))
        .to_string()
}

// (text, fill) pairs, labelled, per the contrast list.
fn text_pairs(mode: ThemeMode, t: &Value) -> Vec<(String, String, String)> {
    // The selection is drawn translucent, so its pair is measured on the composited fill.
    let selected = selection_over(t, "background");
    let fills = [
        ("background", col(t, "background")),
        ("surface", col(t, "sidebar.background")),
        ("raised", col(t, "popover.background")),
        ("hover", col(t, "accent.background")),
        ("selection", selected),
    ];
    let mut p = Vec::new();
    let mut add = |label: String, a: String, b: String| p.push((label, a, b));
    for (name, fill) in &fills {
        add(
            format!("foreground on {name}"),
            col(t, "foreground"),
            fill.clone(),
        );
        // Muted text is never drawn on the selection: a focused locked input uses foreground.
        if *name != "selection" {
            add(
                format!("muted on {name}"),
                col(t, "muted.foreground"),
                fill.clone(),
            );
        }
    }
    // Input selection over the editable and the locked field fill.
    for field in ["secondary.background", "muted.background"] {
        add(
            format!("foreground on selection over {field}"),
            col(t, "foreground"),
            selection_over(t, field),
        );
    }
    for (name, fill) in &fills[..3] {
        add(
            format!("brand on {name}"),
            col(t, "primary.background"),
            fill.clone(),
        );
        for (s, _) in STATUSES {
            add(
                format!("{s} on {name}"),
                col(t, &format!("{s}.background")),
                fill.clone(),
            );
        }
    }
    for key in [
        "primary.background",
        "primary.hover.background",
        "primary.active.background",
    ] {
        add(
            format!("on-brand on {key}"),
            col(t, "primary.foreground"),
            col(t, key),
        );
    }
    for (s, status) in STATUSES {
        let fill = status_fill_hex(status, mode).to_string();
        add(
            format!("on-status on {s}"),
            col(t, &format!("{s}.foreground")),
            col(t, &format!("{s}.background")),
        );
        add(
            format!("{s} on its fill"),
            col(t, &format!("{s}.background")),
            fill.clone(),
        );
        add(
            format!("foreground on {s} fill"),
            col(t, "foreground"),
            fill,
        );
    }
    p
}

#[test]
fn every_text_pair_reaches_aa() {
    let mut fails = Vec::new();
    for (mode, t) in themes() {
        let pairs = text_pairs(mode, &t);
        assert_eq!(
            pairs.len(),
            41,
            "the 40 listed pairs, less muted on the selection, plus 2 field selections"
        );
        for (label, a, b) in pairs {
            let r = ratio(&a, &b);
            if r < floor(&t) {
                fails.push(format!("{}: {label} {a} on {b} = {r:.2}", t["name"]));
            }
        }
    }
    assert!(fails.is_empty(), "{fails:#?}");
}

#[test]
fn syntax_colours_reach_aa_on_the_editor() {
    let mut fails = Vec::new();
    for (_, t) in themes() {
        let h = &t["highlight"];
        for bg in ["editor.background", "editor.active_line.background"] {
            let bg = h[bg].as_str().unwrap();
            for (k, style) in h["syntax"].as_object().unwrap() {
                let c = style["color"].as_str().unwrap();
                if ratio(c, bg) < floor(&t) {
                    fails.push(format!("{} {k} {c} on {bg}", t["name"]));
                }
            }
            let n = h["editor.line_number"].as_str().unwrap();
            if ratio(n, bg) < floor(&t) {
                fails.push(format!("{} line number on {bg}", t["name"]));
            }
        }
    }
    assert!(fails.is_empty(), "{fails:#?}");
}

#[test]
fn the_focus_ring_reaches_three_to_one() {
    for (_, t) in themes() {
        for key in ["background", "sidebar.background", "popover.background"] {
            let r = ratio(&col(&t, "ring"), &col(&t, key));
            assert!(r >= 3.0, "{} ring on {key} = {r:.2}", t["name"]);
        }
    }
}

/// The kit's keyboard focus band alpha (gpui-component 0.9.0 `FOCUS_RING_OPACITY`, styled.rs).
const KIT_FOCUS_BAND_ALPHA: f64 = 0.5;

/// Every opaque surface a focus ring can sit on; the editable input fill is `secondary.background`.
const RING_SURFACES: [&str; 11] = [
    "background",
    "popover.background",
    "secondary.background",
    "muted.background",
    "sidebar.background",
    "tab_bar.background",
    "list.even.background",
    "accent.background",
    "list.hover.background",
    "list.active.background",
    "button.background",
];

/// In the high-contrast themes the kit's half-alpha band, composited over each surface, clears
/// 3:1, and the opaque ring the app's own rings and the focused border paint clears 4.5:1.
#[test]
fn high_contrast_focus_rings_clear_their_floors_on_every_surface() {
    let mut bad = Vec::new();
    for (_, t) in themes() {
        let name = t["name"].as_str().unwrap();
        if name != HC_LIGHT && name != HC_DARK {
            continue;
        }
        let ring = col(&t, "ring");
        for key in RING_SURFACES {
            let fill = &col(&t, key);
            let band = ratio(&blend(&ring, fill, KIT_FOCUS_BAND_ALPHA), fill);
            let opaque = ratio(&ring, fill);
            println!("{name} {key}: band {band:.2}, opaque {opaque:.2}");
            if band < 3.0 {
                bad.push(format!("{name}: focus band on {key} = {band:.2}"));
            }
            if opaque < 4.5 {
                bad.push(format!("{name}: ring on {key} = {opaque:.2}"));
            }
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn status_fills_match_the_spec() {
    let dark: Vec<_> = STATUSES
        .iter()
        .map(|(_, s)| status_fill_hex(*s, ThemeMode::Dark))
        .collect();
    let light: Vec<_> = STATUSES
        .iter()
        .map(|(_, s)| status_fill_hex(*s, ThemeMode::Light))
        .collect();
    assert_eq!(dark, ["#173322", "#14283D", "#3A2F0E", "#3D1A1E"]);
    assert_eq!(light, ["#E3F4E6", "#E1EEFA", "#FBF0D2", "#FDEDED"]);
}

#[test]
fn the_selection_edge_reaches_three_to_one_on_the_selected_fill() {
    for (_, t) in themes() {
        // The focused edge is the ring colour, the unfocused one the muted text colour.
        for edge in ["ring", "muted.foreground"] {
            let r = ratio(&col(&t, edge), &col(&t, "list.active.background"));
            assert!(
                r >= 3.0,
                "{} {edge} on list.active.background = {r:.2}",
                t["name"]
            );
        }
    }
}

#[test]
fn the_segmented_selection_is_visible() {
    // The raised fill with full foreground text carries the selected segment, never the
    // brand or focus ring; the 3:1 control outline still holds on the track and the fill.
    for (_, t) in themes() {
        let fill = ratio(&col(&t, "popover.background"), &col(&t, "muted.background"));
        assert!(
            fill >= 1.08,
            "{} raised segment on the track = {fill:.2}",
            t["name"]
        );
        let text = ratio(&col(&t, "foreground"), &col(&t, "popover.background"));
        assert!(
            text >= floor(&t),
            "{} foreground on raised = {text:.2}",
            t["name"]
        );
        let edge = col(&t, "input.border");
        for under in ["muted.background", "popover.background"] {
            let r = ratio(&edge, &col(&t, under));
            assert!(r >= 3.0, "{} segment edge on {under} = {r:.2}", t["name"]);
        }
        for loud in ["primary.background", "ring"] {
            assert_ne!(edge, col(&t, loud), "{} segment edge is {loud}", t["name"]);
        }
    }
}

#[test]
fn the_selected_row_fill_stands_out() {
    for (_, t) in themes() {
        let fill = col(&t, "list.active.background");
        let back = ratio(&fill, &col(&t, "background"));
        assert!(
            back >= 1.25,
            "{} selected fill on background = {back:.2}",
            t["name"]
        );
        let raised = ratio(&fill, &col(&t, "popover.background"));
        assert!(
            raised >= 1.15,
            "{} selected fill on raised = {raised:.2}",
            t["name"]
        );
        for text in ["foreground", "muted.foreground"] {
            let r = ratio(&col(&t, text), &fill);
            assert!(
                r >= floor(&t),
                "{} {text} on the selected fill = {r:.2}",
                t["name"]
            );
        }
    }
}

#[test]
fn control_outlines_reach_three_to_one() {
    for (_, t) in themes() {
        let mode = t["mode"].as_str().unwrap().to_string();
        for fill in ["background", "sidebar.background", "popover.background"] {
            let r = ratio(&col(&t, "input.border"), &col(&t, fill));
            assert!(r >= 3.0, "{mode}: input.border on {fill} is {r:.2}:1");
        }
    }
}

fn hex_byte(hex: &str, i: usize) -> f64 {
    let h = hex.trim_start_matches('#');
    u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap() as f64
}

/// `top` (`#RRGGBBAA`) painted over the opaque `under`, as `#RRGGBB`.
fn over(top: &str, under: &str) -> String {
    blend(top, under, hex_byte(top, 3) / 255.0)
}

/// The colour of `top` (alpha ignored) at alpha `a` over the opaque `under`, blended in sRGB.
fn blend(top: &str, under: &str, a: f64) -> String {
    let mixed: String = (0..3)
        .map(|i| {
            format!(
                "{:02X}",
                (hex_byte(top, i) * a + hex_byte(under, i) * (1.0 - a)).round() as u8
            )
        })
        .collect();
    format!("#{mixed}")
}

/// `selection.background` as the kit draws it over the `field` fill: alpha capped at 0.3.
fn selection_over(t: &Value, field: &str) -> String {
    let sel = col(t, "selection.background");
    let alpha = if sel.len() == 9 {
        u8::from_str_radix(&sel[7..9], 16).unwrap()
    } else {
        255
    };
    let alpha = alpha.min((0.3_f64 * 255.0).round() as u8);
    over(&format!("{}{alpha:02X}", &sel[..7]), &col(t, field))
}

/// The input selection as drawn: the kit caps its alpha at 0.3, so the highlight is that
/// share of `selection.background` over the field fill. Foreground text must hold the text
/// floor on that composite. Muted text is not measured: a focused locked input draws foreground.
#[test]
fn the_input_selection_is_visible_on_both_field_fills() {
    let mut bad = Vec::new();
    let mut seen = 0;
    for (mode, t) in themes() {
        seen += 1;
        let name = t["name"].as_str().unwrap();
        // Best reachable fill-to-field ratio per theme, with a little slack for rounding.
        let edge_floor = match (name, mode) {
            (HC_DARK, _) => 2.1,
            (_, ThemeMode::Dark) => 2.6,
            _ => 2.0,
        };
        for field in ["secondary.background", "muted.background", "background"] {
            let fill = col(&t, field);
            let drawn = selection_over(&t, field);
            let edge = ratio(&drawn, &fill);
            let text = ratio(&col(&t, "foreground"), &drawn);
            if edge < edge_floor {
                bad.push(format!("{name}: selection on {field} = {edge:.2}"));
            }
            if text < floor(&t) {
                bad.push(format!("{name}: selected text on {field} = {text:.2}"));
            }
        }
    }
    assert_eq!(seen, 4);
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn variable_tints_keep_text_readable_and_stand_out() {
    use apinata_app::theme::{VarKind, var_tint_hex};
    let mut fails = Vec::new();
    for (mode, t) in themes() {
        let h = &t["highlight"];
        let field = col(&t, "secondary.background");
        let editor = h["editor.background"].as_str().unwrap().to_string();
        let code_texts = [
            h["editor.foreground"].as_str().unwrap().to_string(),
            h["syntax"]["string"]["color"].as_str().unwrap().to_string(),
        ];
        let tints: Vec<&str> = VarKind::ALL
            .iter()
            .map(|k| var_tint_hex(*k, mode))
            .collect();
        let mut distinct = tints.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), 4, "{} tints differ", t["name"]);
        for tint in tints {
            // A single-line field paints the tint over its text.
            let fill = over(tint, &field);
            let text = over(tint, &col(&t, "foreground"));
            let mut check = |label: &str, r: f64, min: f64| {
                if r < min {
                    fails.push(format!("{} {tint} {label} = {r:.2}", t["name"]));
                }
            };
            check("field text", ratio(&text, &fill), floor(&t));
            check("field fill", ratio(&fill, &field), 1.2);
            // The body editor paints it behind its text.
            let fill = over(tint, &editor);
            for c in &code_texts {
                check(&format!("code {c}"), ratio(c, &fill), floor(&t));
            }
            check("code fill", ratio(&fill, &editor), 1.2);
        }
    }
    assert!(fails.is_empty(), "{fails:#?}");
}

// Coloured method tags show on the window, the sidebar, a hovered or selected row, either tab
// fill and a popover. The input selection is translucent, so its own test composites it.
#[test]
fn method_colours_reach_aa_where_they_show() {
    use apinata_app::ui::{StatusClass, method_class};
    let mut fails = Vec::new();
    for (_, t) in themes() {
        for m in ["GET", "POST", "PUT", "PATCH", "DELETE"] {
            let key = match method_class(m) {
                StatusClass::Success => "success.background",
                StatusClass::Info => "info.background",
                StatusClass::Warning => "warning.background",
                StatusClass::Danger => "danger.background",
                StatusClass::Muted => panic!("{m} is coloured"),
            };
            for fill in [
                "background",
                "sidebar.background",
                "list.hover.background",
                "list.active.background",
                "tab.background",
                "tab.active.background",
                "popover.background",
            ] {
                let r = ratio(&col(&t, key), &col(&t, fill));
                if r < floor(&t) {
                    fails.push(format!("{} {m} on {fill} = {r:.2}", t["name"]));
                }
            }
        }
    }
    assert!(fails.is_empty(), "{fails:#?}");
}

#[test]
fn both_sets_are_checked_and_high_contrast_foreground_reaches_seven() {
    let all = themes();
    let names: Vec<_> = all
        .iter()
        .map(|(_, t)| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 4, "{names:?}");
    for want in [HC_LIGHT, HC_DARK] {
        assert!(names.contains(&want), "{want} missing from {names:?}");
    }
    for (_, t) in all {
        if floor(&t) < 7.0 {
            continue;
        }
        for (a, b) in [
            ("foreground", "background"),
            ("muted.foreground", "background"),
        ] {
            let r = ratio(&col(&t, a), &col(&t, b));
            assert!(r >= 7.0, "{} {a} on {b} = {r:.2}", t["name"]);
        }
    }
}

/// Placeholders and disabled fields as the kit paints them: a placeholder is `muted.foreground`
/// as is, a disabled table cell is `muted.foreground` at the kit's alpha over `muted.background`.
/// The high-contrast themes keep both readable and apart from entered text.
#[test]
fn high_contrast_placeholders_and_disabled_text_stay_readable() {
    use apinata_app::theme::disabled_text_alpha;
    let mut fails = Vec::new();
    for (_, t) in themes() {
        let name = t["name"].as_str().unwrap();
        if ![HC_LIGHT, HC_DARK].contains(&name) {
            assert_eq!(
                disabled_text_alpha(name),
                0.5,
                "{name} keeps the kit's dimming"
            );
            continue;
        }
        let muted = col(&t, "muted.foreground");
        for fill in ["background", "secondary.background", "muted.background"] {
            let r = ratio(&muted, &col(&t, fill));
            if r < 7.0 {
                fails.push(format!("{name}: placeholder on {fill} = {r:.2}"));
            }
        }
        if muted.eq_ignore_ascii_case(&col(&t, "foreground")) {
            fails.push(format!("{name}: placeholder matches entered text"));
        }
        let alpha = format!("{:02X}", (disabled_text_alpha(name) * 255.0).round() as u8);
        let dimmed = over(&format!("{muted}{alpha}"), &col(&t, "muted.background"));
        let r = ratio(&dimmed, &col(&t, "muted.background"));
        if r < 4.5 {
            fails.push(format!("{name}: disabled text {dimmed} on muted = {r:.2}"));
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

/// WCAG contrast of two opaque colours.
fn hsla_ratio(a: gpui_kit::Hsla, b: gpui_kit::Hsla) -> f64 {
    let lum = |c: gpui_kit::Hsla| {
        let c = c.to_rgb();
        0.2126 * channel(c.r as f64) + 0.7152 * channel(c.g as f64) + 0.0722 * channel(c.b as f64)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// The chosen collection button's label, as the view draws it, on its fill in every bundled
/// theme; the high-contrast pair is held to the same 4.5:1 floor.
#[gpui_kit::test]
fn the_chosen_collection_label_reaches_aa_in_every_theme(cx: &mut gpui_kit::TestAppContext) {
    use apinata_app::settings::ThemeChoice;
    use apinata_app::theme::{DARK, LIGHT, chosen_label_colors};
    use gpui_kit::component::Theme;
    let _s = crate::support::started(cx, &crate::support::two_requests());
    let (mut seen, mut low) = (Vec::new(), Vec::new());
    for (choice, light, dark) in [
        (ThemeChoice::Dark, LIGHT, DARK),
        (ThemeChoice::Dark, LIGHT, HC_DARK),
        (ThemeChoice::Light, LIGHT, DARK),
        (ThemeChoice::Light, HC_LIGHT, DARK),
    ] {
        cx.update(|cx| {
            let s = cx.global::<apinata_app::app::AppState>().settings.clone();
            s.update(cx, |p, cx| {
                p.update(cx, |s| {
                    s.theme = choice;
                    s.light_theme = Some(light.to_string());
                    s.dark_theme = Some(dark.to_string());
                })
            });
        });
        cx.run_until_parked();
        let (name, text, fill) = cx.update(|cx| {
            let t = Theme::global(cx);
            let (text, fill) = chosen_label_colors(t);
            assert_eq!(text, t.foreground, "the label is the normal foreground");
            (t.theme_name().to_string(), text, fill)
        });
        seen.push(name.clone());
        let r = hsla_ratio(text, fill);
        if r < 4.5 {
            low.push(format!("{name} chosen label = {r:.2}"));
        }
    }
    assert!(low.is_empty(), "{low:#?}");
    assert_eq!(seen.len(), 4, "{seen:?}");
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 4, "four distinct themes: {seen:?}");
}

/// The commit message while a commit runs, as the open dialog draws it. The view locks the
/// textarea read-only in the high-contrast themes, where the kit paints plain `foreground`
/// on `input_background()` over the dialog's `background`; both come from the live theme, and
/// the lock state is read off the rendered view, so a regression to the dimmed disabled look
/// fails here.
#[gpui_kit::test]
fn the_busy_commit_message_reaches_aa_in_high_contrast(cx: &mut gpui_kit::TestAppContext) {
    use crate::git_support::Sandbox;
    use crate::h::*;
    use crate::support::*;
    use gpui_kit::AppContext as _;
    use gpui_kit::component::ActiveTheme as _;
    let mut low = Vec::new();
    for name in [HC_LIGHT, HC_DARK] {
        let sb = Sandbox::new();
        let col = changed(&sb);
        let s = git_started(cx, &sb, &col);
        use_theme(cx, name);
        open_on(cx, &s, col.path());
        view(cx).update(cx, |v, cx| {
            v.toggle(0, cx);
            assert!(v.busy(), "{name}: a commit is running");
        });
        // A bare render, so the work cannot finish before the frame is drawn.
        cx.update_window(s.win, |_, w, cx| {
            use gpui_kit::test::TestWindowExt as _;
            w.render_frame(cx)
        })
        .unwrap();
        let lock = view(cx).read_with(cx, |v, cx| v.message_lock(cx));
        assert_eq!(lock, (true, false), "{name}: read-only, not dimmed");
        let (text, fill) = cx.update(|cx| {
            let t = cx.theme();
            (t.foreground, t.background.blend(t.input_background()))
        });
        let r = hsla_ratio(text, fill);
        if r < 4.5 {
            low.push(format!("{name}: busy message = {r:.2}"));
        }
        idle(cx, &s);
    }
    assert!(low.is_empty(), "{low:#?}");
}

/// A shadowed row of the variables overview in each theme. The Apinata themes keep the 0.6
/// dimming; the high-contrast themes draw it at full opacity in `muted.foreground`, and what
/// shows is that text over the dialog background, which must clear 4.5:1.
#[gpui_kit::test]
fn shadowed_variable_rows_reach_aa_in_high_contrast(cx: &mut gpui_kit::TestAppContext) {
    use apinata_app::theme::{DARK, LIGHT, shadowed_look};
    use gpui_kit::component::ActiveTheme as _;
    let mut low = Vec::new();
    for name in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            let t = cx.theme();
            let (opacity, text) = shadowed_look(t);
            if [HC_LIGHT, HC_DARK].contains(&name) {
                assert_eq!(opacity, 1.0, "{name}: no extra dimming");
                assert_eq!(text, Some(t.muted_foreground), "{name}: muted text");
                let r = hsla_ratio(t.muted_foreground, t.background);
                if r < 4.5 {
                    low.push(format!("{name}: shadowed row = {r:.2}"));
                }
            } else {
                assert_eq!((opacity, text), (0.6, None), "{name} is unchanged");
            }
        });
    }
    assert!(low.is_empty(), "{low:#?}");
}

/// An off row of a key-value table draws its cells in `muted.foreground` (`kv_table`), on the
/// raised field fill, or on the muted fill when the table is locked. Both must clear 4.5:1 in
/// every theme, high contrast included.
#[test]
fn off_table_rows_reach_aa_on_both_field_fills() {
    let mut low = Vec::new();
    let mut seen = 0;
    for (_, t) in themes() {
        seen += 1;
        let name = t["name"].as_str().unwrap();
        for fill in ["secondary.background", "muted.background"] {
            let r = ratio(&col(&t, "muted.foreground"), &col(&t, fill));
            if r < 4.5 {
                low.push(format!("{name}: off row on {fill} = {r:.2}"));
            }
        }
    }
    assert_eq!(seen, 4);
    assert!(low.is_empty(), "{low:#?}");
}

/// A disabled button's label as drawn: the kit's `muted_foreground` at half alpha unless
/// `disabled_button_text` replaces it. Over every surface a button sits on, and over the
/// primary tint (15%) a disabled primary button gets, the high-contrast themes must clear
/// 4.5:1 and the Apinata themes must keep the kit's look.
#[gpui_kit::test]
fn disabled_buttons_reach_aa_in_high_contrast(cx: &mut gpui_kit::TestAppContext) {
    use apinata_app::theme::{DARK, LIGHT};
    use apinata_app::ui::disabled_button_text;
    use gpui_kit::component::ActiveTheme as _;
    let sets: Vec<Value> = themes().into_iter().map(|(_, t)| t).collect();
    let mut low = Vec::new();
    for name in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        crate::support::use_theme(cx, name);
        let t = sets.iter().find(|t| t["name"] == name).unwrap();
        let text = cx.update(|cx| disabled_button_text(cx.theme()));
        if ![HC_LIGHT, HC_DARK].contains(&name) {
            assert_eq!(text, None, "{name} keeps the kit's disabled look");
            continue;
        }
        let muted = col(t, "muted.foreground");
        let Some(text) = text else {
            low.push(format!("{name}: the kit's half-alpha label is kept"));
            continue;
        };
        let hex = |h: gpui_kit::Hsla| {
            let c = h.to_rgb();
            let b = |v: f32| (v * 255.0).round() as u8;
            format!("#{:02X}{:02X}{:02X}{:02X}", b(c.r), b(c.g), b(c.b), b(c.a))
        };
        assert_eq!(
            hex(text)[..7].to_uppercase(),
            muted.to_uppercase(),
            "{name}"
        );
        for surface in [
            "background",
            "sidebar.background",
            "popover.background",
            "secondary.background",
        ] {
            let under = col(t, surface);
            let tint = over(&format!("{}26", col(t, "primary.background")), &under);
            for (what, fill) in [("plain", under.clone()), ("primary tint", tint)] {
                let r = ratio(&muted, &fill);
                if r < 4.5 {
                    low.push(format!(
                        "{name}: disabled label on {what} {surface} = {r:.2}"
                    ));
                }
            }
        }
    }
    assert!(low.is_empty(), "{low:#?}");
}

/// A real disabled `Button` as the views build it: in a high-contrast theme its own text
/// colour is the full-strength `muted_foreground`. The kit replays a button's own style inside
/// its disabled state (`instance_style` in gpui-component 0.9.0 `button.rs`), so that colour
/// wins over the kit's half-alpha `muted_foreground`; elsewhere the button carries none.
#[gpui_kit::test]
fn a_disabled_button_draws_the_override_label_in_high_contrast(cx: &mut gpui_kit::TestAppContext) {
    use apinata_app::theme::{DARK, LIGHT};
    use apinata_app::ui::LockButton as _;
    use gpui_kit::Styled as _;
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::component::button::Button;
    for name in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            let muted = cx.theme().muted_foreground;
            let own = |disabled: bool, cx: &gpui_kit::App| {
                Button::new("b")
                    .label("Go")
                    .lock(disabled, cx)
                    .style()
                    .text
                    .color
            };
            assert_eq!(
                own(false, cx),
                None,
                "{name}: an enabled button keeps the kit's"
            );
            if [HC_LIGHT, HC_DARK].contains(&name) {
                let drawn = own(true, cx).expect("a disabled button sets its label colour");
                assert_eq!(drawn, muted, "{name}: full-strength muted text");
                assert_ne!(
                    drawn,
                    muted.opacity(0.5),
                    "{name}: not the kit's disabled label"
                );
            } else {
                assert_eq!(own(true, cx), None, "{name} keeps the kit's disabled look");
            }
        });
    }
}

/// Every `Button::disabled` call needs the kit's `Disableable` trait in scope, and only `ui.rs`
/// (which wraps it as `LockButton::lock`) may import it. A view that disables a button
/// directly would draw the kit's half-alpha label, which fails 4.5:1 in high contrast.
#[test]
fn only_ui_rs_imports_the_kits_disableable_trait() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(files.len() > 50, "found the sources");
    let bad: Vec<_> = files
        .iter()
        .filter(|p| p.file_name().unwrap() != "ui.rs")
        .filter(|p| names_disableable(&std::fs::read_to_string(p).unwrap()))
        .collect();
    assert!(
        bad.is_empty(),
        "disable buttons through ui::LockButton: {bad:?}"
    );
}

/// Opaque surfaces a button can sit on in any theme.
const SURFACES: [&str; 4] = ["background", "sidebar", "popover", "secondary"];

/// The opaque colour `fill` shows as over each surface of the live theme.
fn over_surfaces(
    t: &gpui_kit::component::Theme,
    fill: gpui_kit::Hsla,
) -> Vec<(&'static str, gpui_kit::Hsla)> {
    let surface = |n: &str| match n {
        "background" => t.background,
        "sidebar" => t.sidebar,
        "popover" => t.popover,
        _ => t.secondary,
    };
    SURFACES
        .iter()
        .map(|n| (*n, surface(n).blend(fill)))
        .collect()
}

/// Every selected control's label on its fill, as the views draw them (the label and fill come
/// from `selected_look`, and the select button's open fill from `select_open_fill`), over every
/// surface, in all four bundled themes at 4.5:1.
#[gpui_kit::test]
fn selected_button_labels_reach_aa_in_every_theme(cx: &mut gpui_kit::TestAppContext) {
    use apinata_app::theme::{DARK, LIGHT, SelectedStyle, select_open_fill, selected_look};
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::component::Colorize as _;
    let mut low = Vec::new();
    for name in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            let t = cx.theme();
            let mut check = |control: &str, text: gpui_kit::Hsla, fill: gpui_kit::Hsla| {
                for (surface, shown) in over_surfaces(t, fill) {
                    let r = hsla_ratio(text, shown);
                    if r < 4.5 {
                        low.push(format!("{name}: {control} on {surface} = {r:.2}"));
                    }
                }
            };
            for (control, style) in [
                ("ghost toggle", SelectedStyle::Ghost),
                ("plain toggle", SelectedStyle::Plain),
                ("chosen collection", SelectedStyle::ChosenOutlinePrimary),
            ] {
                let (text, fill) = selected_look(t, style);
                check(control, text, fill);
            }
            // The select button is outline-default: the kit fills it `input` at 30% (dark) or
            // the background, 50% on hover and 70% while its menu is open.
            let open = select_open_fill(t).unwrap_or(t.input.mix_oklab(t.transparent, 0.7));
            check("select open", t.button_foreground, open);
            check(
                "select hover",
                t.button_foreground,
                t.input.mix_oklab(t.transparent, 0.5),
            );
            check("select idle", t.button_foreground, t.input_background());
            if ![HC_DARK].contains(&name) {
                assert_eq!(
                    select_open_fill(t),
                    None,
                    "{name} keeps the kit's open fill"
                );
            }
        });
    }
    assert!(low.is_empty(), "{low:#?}");
}

/// A button a view marks selected carries the label and fill `selected_look` names, and only
/// while selected; the kit replays a button's own style inside its selected state, so these
/// are what is drawn.
#[gpui_kit::test]
fn a_chosen_button_carries_the_measured_look(cx: &mut gpui_kit::TestAppContext) {
    use apinata_app::theme::{DARK, LIGHT, SelectedStyle, selected_look};
    use apinata_app::ui::ChooseButton as _;
    use gpui_kit::Styled as _;
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::component::button::Button;
    for name in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            for style in [
                SelectedStyle::Ghost,
                SelectedStyle::Plain,
                SelectedStyle::ChosenOutlinePrimary,
            ] {
                let (text, fill) = selected_look(cx.theme(), style);
                let mut on = Button::new("b").choose(true, style, cx);
                assert_eq!(on.style().text.color, Some(text), "{name} {style:?} label");
                assert_eq!(
                    match on.style().background.clone() {
                        Some(gpui_kit::Fill::Color(b)) => b.as_solid(),
                        _ => None,
                    },
                    Some(fill),
                    "{name} {style:?} fill"
                );
                let mut off = Button::new("b").choose(false, style, cx);
                assert_eq!(off.style().text.color, None, "{name} {style:?} idle label");
                assert!(
                    off.style().background.is_none(),
                    "{name} {style:?} idle fill"
                );
            }
        });
    }
}

/// `src` with comments and string or character literals blanked to spaces (newlines kept), so
/// a guard sees only code.
fn code_only(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let blank = |c: char| if c == '\n' { '\n' } else { ' ' };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            // Block comments nest in Rust.
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blank(chars[i]));
                    i += 1;
                }
            }
        } else if c == 'r' && matches!(next, Some('"' | '#')) && !raw_prefix_is_ident(&chars, i) {
            // A raw string ends at a quote followed by as many hashes as it opened with.
            let mut j = i + 1;
            while chars.get(j) == Some(&'#') {
                j += 1;
            }
            if chars.get(j) != Some(&'"') {
                out.push(c);
                i += 1;
                continue;
            }
            let hashes = j - i - 1;
            let mut k = j + 1;
            while k < chars.len()
                && !(chars[k] == '"' && (1..=hashes).all(|h| chars.get(k + h) == Some(&'#')))
            {
                k += 1;
            }
            let end = (k + hashes + 1).min(chars.len());
            out.extend(chars[i..end].iter().map(|&c| blank(c)));
            i = end;
        } else if c == '"' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '"' {
                j += if chars[j] == '\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(chars.len());
            out.extend(chars[i..end].iter().map(|&c| blank(c)));
            i = end;
        } else if c == '\'' {
            // A character literal closes within a few characters; a lifetime never does.
            let lit = if next == Some('\\') {
                // The escaped character itself may be a quote, so look for the close after it.
                chars[i + 3..]
                    .iter()
                    .take(8)
                    .position(|&c| c == '\'')
                    .map(|p| i + 3 + p + 1)
            } else {
                (chars.get(i + 2) == Some(&'\'')).then_some(i + 3)
            };
            if let Some(end) = lit {
                out.extend(chars[i..end].iter().map(|&c| blank(c)));
                i = end;
            } else {
                out.push(c);
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Whether the `r` at `i` ends an identifier, as opposed to opening a raw (or `br`) string.
fn raw_prefix_is_ident(chars: &[char], i: usize) -> bool {
    let ident = |j: usize| chars[j].is_alphanumeric() || chars[j] == '_';
    match i {
        0 => false,
        1 => ident(0) && chars[0] != 'b',
        _ => ident(i - 1) && !(chars[i - 1] == 'b' && !ident(i - 2)),
    }
}

/// Whether the code globs the kit's `component` module, alone or as an item of its braces
/// (nested bare braces included), since the traits come into scope through it. A glob of a
/// submodule (`component::input::*`) does not count. Aliasing the module (`component as c`) and
/// then globbing the alias is not followed.
fn globs_component(code: &str) -> bool {
    let squeezed: Vec<char> = code.split_whitespace().flat_map(str::chars).collect();
    let marker: Vec<char> = "component::".chars().collect();
    (0..squeezed.len()).any(|at| {
        if !squeezed[at..].starts_with(&marker) {
            return false;
        }
        let rest = &squeezed[at + marker.len()..];
        if rest.first() == Some(&'*') {
            return true;
        }
        if rest.first() != Some(&'{') {
            return false;
        }
        // Bare groups opened right after `{` or `,` import into the same module; a group after
        // a path (`tab::{..}`) imports from a submodule.
        let mut stack = vec![true];
        let mut prev = '{';
        for &c in &rest[1..] {
            match c {
                '{' => stack.push(matches!(prev, '{' | ',') && stack.last() == Some(&true)),
                '}' => {
                    stack.pop();
                    if stack.is_empty() {
                        return false;
                    }
                }
                '*' if matches!(prev, '{' | ',') && stack.last() == Some(&true) => return true,
                _ => {}
            }
            prev = c;
        }
        false
    })
}

/// Whether `src` has a `use` statement naming the kit trait `name`, flat or inside braces over
/// any number of lines, with or without `as _`, or a glob import of the kit's `component`
/// module, which holds the traits.
fn imports_trait(src: &str, name: &str) -> bool {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    code_only(src).split(';').any(|stmt| {
        let Some(at) = stmt.find("use ") else {
            return false;
        };
        // Only a statement that begins with `use` (after attributes or `pub`) counts.
        let head = &stmt[..at];
        let starts_use = at == 0 || head.ends_with(char::is_whitespace);
        starts_use
            && (stmt[at..]
                .split(|c: char| !is_ident(c))
                .any(|word| word == name)
                || globs_component(stmt))
    })
}

fn imports_selectable(src: &str) -> bool {
    imports_trait(src, "Selectable")
}

/// Whether `src` names the kit's `Disableable` trait in code, or imports the module holding it
/// by glob.
fn names_disableable(src: &str) -> bool {
    let code = code_only(src);
    code.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word == "Disableable")
        || code
            .split(';')
            .any(|s| s.contains("use ") && globs_component(s))
}

#[test]
fn the_import_matchers_catch_globs_and_ignore_comments_and_strings() {
    for glob in [
        "use gpui_kit::component::*;",
        "use gpui_kit::{component::*, Foo};",
        "use gpui_kit::{\n    component::{\n        *,\n    },\n};",
    ] {
        assert!(imports_selectable(glob), "selectable glob: {glob}");
    }
    for glob in [
        "use gpui_kit::component::*;",
        "use gpui_kit::{component::*, Foo};",
    ] {
        assert!(names_disableable(glob), "disableable glob: {glob}");
    }
    for quiet in [
        "// use gpui_kit::component::Selectable as _;\nfn f() {}",
        "/* use gpui_kit::component::*; */",
        "const DOC: &str = \"use gpui_kit::component::Selectable;\";",
        "const RAW: &str = r#\"use gpui_kit::component::*;\"#;",
        "use gpui_kit::*;",
    ] {
        assert!(!imports_selectable(quiet), "selectable flagged: {quiet}");
        assert!(!names_disableable(quiet), "disableable flagged: {quiet}");
    }
    for glob in [
        "use gpui_kit::component::{Button, *};",
        "use gpui_kit::component::{self, Foo, *};",
        "use gpui_kit::component::{Button, {Foo, *}};",
        "use gpui_kit::{component::{Button,\n    *,\n}, Bar};",
    ] {
        assert!(imports_selectable(glob), "selectable glob: {glob}");
        assert!(names_disableable(glob), "disableable glob: {glob}");
    }
    for quiet in [
        "use gpui_kit::component::{input::*, Button};",
        "use gpui_kit::component::{Button, tab::{Tab, *}};",
        "use gpui_kit::component::input::*;",
    ] {
        assert!(!imports_selectable(quiet), "selectable flagged: {quiet}");
        assert!(!names_disableable(quiet), "disableable flagged: {quiet}");
    }
    // Literals the blanker must get right: an escaped quote char, byte and raw byte strings.
    for quiet in [
        "let q = '\\''; // use gpui_kit::component::*;\nfn f() {}",
        "let b = b\"use gpui_kit::component::*;\";",
        "let b = br#\"use gpui_kit::component::*;\"#;",
        "let b = br\"use gpui_kit::component::*;\";",
        "let c = b'\"'; let s = \"x\"; // use gpui_kit::component::*;",
    ] {
        assert!(!imports_selectable(quiet), "selectable flagged: {quiet}");
        assert!(!names_disableable(quiet), "disableable flagged: {quiet}");
    }
    assert!(names_disableable(
        "let q = '\\''; use gpui_kit::component::Disableable as _;"
    ));
    assert!(names_disableable(
        "use gpui_kit::component::Disableable as _;"
    ));
    assert!(!names_disableable(
        "// Disableable is wrapped by LockButton\nfn f() {}"
    ));
    assert!(!names_disableable(
        "let s = \"Disableable\"; let c = '\"'; fn g<'a>() {}"
    ));
    // A real import after a comment containing an apostrophe is still seen.
    assert!(names_disableable(
        "// it's\nuse gpui_kit::component::Disableable as _;"
    ));
}

#[test]
fn the_selectable_import_matcher_sees_braced_and_multiline_imports() {
    for hit in [
        "use gpui_kit::component::Selectable as _;",
        "use gpui_kit::component::{ActiveTheme as _, Selectable as _};",
        "use gpui_kit::{\n    Foo,\n    Selectable as _,\n    Sizable as _,\n};",
        "use gpui_kit::Selectable;",
        "fn f() {}\npub use kit::{A, Selectable};",
    ] {
        assert!(imports_selectable(hit), "should match: {hit}");
    }
    for miss in [
        "use gpui_kit::component::{ActiveTheme as _, Sizable as _};",
        "use apinata_app::ui::ChooseButton as _;",
        "use kit::SelectableRow;",
        "let reuse = Selectable;",
    ] {
        assert!(!imports_selectable(miss), "should not match: {miss}");
    }
}

/// Only `ui.rs` may import the kit's `Selectable` trait, so a view marks a button selected
/// through `ui::ChooseButton` and what it draws is what the contrast test measures.
#[test]
fn only_ui_rs_imports_the_kits_selectable_trait() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(files.len() > 50, "found the sources");
    let bad: Vec<_> = files
        .iter()
        .filter(|p| p.file_name().unwrap() != "ui.rs")
        .filter(|p| imports_selectable(&std::fs::read_to_string(p).unwrap()))
        .collect();
    assert!(
        bad.is_empty(),
        "select buttons through ui::ChooseButton: {bad:?}"
    );
}

/// The drawn mixed dash takes its colour from `mixed_dash_color`, the function the contrast
/// test measures; inlining the old branch in `Checkbox::render` would leave that test green.
#[test]
fn checkbox_render_draws_the_dash_with_mixed_dash_color() {
    let ui =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui.rs"))
            .unwrap();
    let code = code_only(&ui);
    let start = code
        .find("impl RenderOnce for Checkbox")
        .expect("the Checkbox render impl");
    // The impl ends at the first closing brace in column 0 after it opens.
    let body = &code[start..];
    let body = &body[..body.find("\n}\n").expect("end of the impl")];
    assert!(
        colours_dash_with_helper(body),
        "Checkbox::render must colour the dash with mixed_dash_color"
    );
}

/// Whether the code draws the dash with the helper's result, not merely calls it.
fn colours_dash_with_helper(code: &str) -> bool {
    let squeezed: String = code.split_whitespace().collect();
    squeezed.contains(".text_color(mixed_dash_color(")
}

#[test]
fn the_dash_matcher_wants_a_live_use_of_the_helper() {
    assert!(colours_dash_with_helper(
        ".path(p)\n    .text_color(mixed_dash_color(self.disabled, cx)),"
    ));
    assert!(colours_dash_with_helper(
        ".text_color(\n    mixed_dash_color(self.disabled, cx),\n)"
    ));
    assert!(!colours_dash_with_helper(
        "let _ = mixed_dash_color(true, cx);"
    ));
    assert!(!colours_dash_with_helper(
        "let c = mixed_dash_color(true, cx); .text_color(red)"
    ));
    assert!(!colours_dash_with_helper(&code_only(
        "// .text_color(mixed_dash_color(a, cx))"
    )));
}

/// Disabled controls as the views draw them, over every surface they sit on, at 4.5:1 in all
/// four bundled themes: a disabled popup menu item (the kit's `muted_foreground` on the
/// popover), a disabled checkbox's label and tick (`checkbox_look`, the tick on its muted box
/// at the look's opacity) and a disabled lock toggle (`lock_look`, glyph on the surface). The
/// two Apinata themes keep the half-opacity look; the tick and glyph are held to 4.5:1 in the
/// high-contrast pair.
#[gpui_kit::test]
fn disabled_menu_items_checkboxes_and_lock_toggles_reach_aa(cx: &mut gpui_kit::TestAppContext) {
    use apinata_app::theme::{DARK, LIGHT};
    use apinata_app::ui::{checkbox_look, lock_look, mixed_dash_color};
    use gpui_kit::component::ActiveTheme as _;
    let mut low = Vec::new();
    for name in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            let t = cx.theme();
            let mut check = |what: &str, surface: &str, r: f64| {
                if r < 4.5 {
                    low.push(format!("{name}: {what} on {surface} = {r:.2}"));
                }
            };
            check(
                "disabled menu item",
                "popover",
                hsla_ratio(t.muted_foreground, t.popover),
            );
            let boxed = checkbox_look(true, true, cx);
            let lock_on = lock_look(true, true, cx);
            let lock_off = lock_look(false, true, cx);
            let high = [HC_LIGHT, HC_DARK].contains(&name);
            if !high {
                assert_eq!(boxed.opacity, 0.5, "{name} keeps the half-opacity box");
                assert_eq!(lock_on.opacity, 0.5, "{name} keeps the half-opacity lock");
            }
            for (surface, under) in [
                ("background", t.background),
                ("sidebar", t.sidebar),
                ("popover", t.popover),
            ] {
                check(
                    "disabled checkbox label",
                    surface,
                    hsla_ratio(t.muted_foreground, under),
                );
                if !high {
                    continue;
                }
                // The box and its tick are one group drawn at the look's opacity.
                let fill = under.blend(boxed.fill);
                let tick = fill.blend(boxed.tick.expect("a checked box has a tick"));
                check(
                    "disabled checkbox tick",
                    surface,
                    hsla_ratio(
                        under.blend(tick.opacity(boxed.opacity)),
                        under.blend(fill.opacity(boxed.opacity)),
                    ),
                );
                // The mixed dash is a child of the same group, so it dims with the box.
                let dash = fill.blend(mixed_dash_color(true, cx));
                check(
                    "disabled checkbox mixed dash",
                    surface,
                    hsla_ratio(
                        under.blend(dash.opacity(boxed.opacity)),
                        under.blend(fill.opacity(boxed.opacity)),
                    ),
                );
                for (what, look) in [("lock on", lock_on), ("lock off", lock_off)] {
                    // A filled lock is a chip: the glyph is read against its own fill, both
                    // dimmed together; a bare one against the surface.
                    let boxed = look
                        .fill
                        .map_or(under, |f| under.blend(f.opacity(look.opacity)));
                    let glyph = under.blend(look.color.opacity(look.opacity));
                    let glyph = look.fill.map_or(glyph, |f| {
                        under.blend(f.blend(look.color).opacity(look.opacity))
                    });
                    check(what, surface, hsla_ratio(glyph, boxed));
                }
            }
        });
    }
    assert!(low.is_empty(), "{low:#?}");
}
