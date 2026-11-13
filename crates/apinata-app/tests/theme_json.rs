//! The shipped theme JSON sets every kit colour and highlight key in both themes, each to
//! the designed token for that key, and nothing the kit does not know.

use apinata_app::theme::{DARK, LIGHT, THEME_JSON};
use gpui_kit::component::highlighter::HighlightThemeStyle;
use gpui_kit::component::{ThemeConfigColors, ThemeSet, try_parse_color};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

// (token, dark, light), from the palette.
const TOKENS: &[(&str, &str, &str)] = &[
    ("background", "#131218", "#FBFAFC"),
    ("surface", "#1A1921", "#F3F1F6"),
    ("raised", "#23212C", "#FFFFFF"),
    ("hover", "#2A2834", "#ECE9F1"),
    ("selection", "#3B2A1F", "#FFE3CC"),
    // The kit paints the input selection at most 30% over the field; see theme_contrast.
    ("input_selection", "#FFFFFF", "#000000"),
    ("selected", "#33303F", "#E2DEE9"),
    ("divider", "#2F2C3A", "#DEDAE5"),
    // Control outlines sit darker than dividers so unchecked boxes reach 3:1.
    ("outline", "#6E6980", "#8A8596"),
    ("foreground", "#ECEAF2", "#1B1530"),
    ("muted", "#A19DAE", "#5F5A6B"),
    ("brand", "#FF922B", "#C2410C"),
    ("brand_hover", "#FFA94D", "#A8380A"),
    ("brand_active", "#F08C2E", "#9A3412"),
    ("on_brand", "#1B1530", "#FFFFFF"),
    ("success", "#51CF66", "#237032"),
    ("info", "#4DABF7", "#1864AB"),
    ("warning", "#FCC419", "#855800"),
    ("danger", "#FF6B6B", "#B42525"),
    ("on_status", "#1B1530", "#FFFFFF"),
    ("transparent", "#00000000", "#00000000"),
    // The light backdrop is darker than its old 30% so a dialog clearly sits over the window.
    ("overlay", "#00000080", "#00000066"),
];

fn keys_for(token: &str, keys: &[&'static str]) -> Vec<(String, String)> {
    keys.iter()
        .map(|k| (k.to_string(), token.to_string()))
        .collect()
}

// Every kit colour key and the token it carries (spec "Complete kit theme").
fn key_map() -> Vec<(String, String)> {
    let mut m = Vec::new();
    m.extend(keys_for(
        "background",
        &[
            "background",
            "tab.active.background",
            "table.background",
            "list.background",
            "group_box.background",
            "accordion.background",
        ],
    ));
    m.extend(keys_for(
        "foreground",
        &[
            "foreground",
            "tab.active.foreground",
            "group_box.foreground",
            "group_box.title.foreground",
            "popover.foreground",
            "secondary.foreground",
            "accent.foreground",
            "sidebar.foreground",
            "sidebar.accent.foreground",
            "button.foreground",
            "button.secondary.foreground",
            "description_list.label.foreground",
            "table.foot.foreground",
            "switch.thumb.background",
        ],
    ));
    m.extend(keys_for(
        "muted",
        &[
            "muted.foreground",
            "tab.foreground",
            "table.head.foreground",
            "scrollbar.thumb.hover.background",
        ],
    ));
    m.extend(keys_for(
        "surface",
        &[
            "title_bar.background",
            "status_bar.background",
            "sidebar.background",
            "tab_bar.background",
            "tab_bar.segmented.background",
            "tab.background",
            "muted.background",
            "list.head.background",
            "table.head.background",
            "table.foot.background",
            "list.even.background",
            "table.even.background",
            "description_list.label.background",
        ],
    ));
    m.extend(keys_for(
        "raised",
        &[
            "popover.background",
            "secondary.background",
            "button.background",
            "button.secondary.background",
            "table.active.background",
            "sidebar.accent.background",
        ],
    ));
    m.extend(keys_for(
        "hover",
        &[
            "accent.background",
            "secondary.hover.background",
            "secondary.active.background",
            "button.hover.background",
            "button.active.background",
            "button.secondary.hover.background",
            "button.secondary.active.background",
            "list.hover.background",
            "table.hover.background",
        ],
    ));
    m.extend(keys_for("selection", &["drop_target.background"]));
    m.extend(keys_for("input_selection", &["selection.background"]));
    m.extend(keys_for("selected", &["list.active.background"]));
    m.extend(keys_for(
        "brand",
        &[
            "primary.background",
            "button.primary.background",
            "sidebar.primary.background",
            "ring",
            "caret",
            "link",
            "table.active.border",
            "progress.bar.background",
            "slider.thumb.background",
            "chart.1",
        ],
    ));
    m.extend(keys_for(
        "brand_hover",
        &[
            "primary.hover.background",
            "button.primary.hover.background",
            "link.hover",
        ],
    ));
    m.extend(keys_for(
        "brand_active",
        &[
            "primary.active.background",
            "button.primary.active.background",
            "link.active",
        ],
    ));
    m.extend(keys_for(
        "on_brand",
        &[
            "primary.foreground",
            "button.primary.foreground",
            "sidebar.primary.foreground",
        ],
    ));
    m.extend(keys_for(
        "outline",
        &["input.border", "scrollbar.thumb.background"],
    ));
    m.extend(keys_for(
        "divider",
        &[
            "list.active.border",
            "border",
            "sidebar.border",
            "title_bar.border",
            "status_bar.border",
            "window.border",
            "table.row.border",
            "drag.border",
            "switch.background",
            "slider.background",
            "skeleton.background",
            "chart.grid",
        ],
    ));
    m.extend(keys_for("transparent", &["scrollbar.background"]));
    m.extend(keys_for("overlay", &["overlay"]));
    m.extend(keys_for(
        "info",
        &[
            "chart.2",
            "base.blue",
            "base.blue.light",
            "base.cyan",
            "base.cyan.light",
        ],
    ));
    m.extend(keys_for(
        "success",
        &["chart.3", "chart.bullish", "base.green", "base.green.light"],
    ));
    m.extend(keys_for(
        "warning",
        &["chart.4", "base.yellow", "base.yellow.light"],
    ));
    m.extend(keys_for(
        "danger",
        &[
            "chart.5",
            "chart.bearish",
            "base.red",
            "base.red.light",
            "base.magenta",
            "base.magenta.light",
        ],
    ));
    for s in ["success", "info", "warning", "danger"] {
        for k in [
            "{s}.background",
            "{s}.hover.background",
            "{s}.active.background",
            "button.{s}.background",
            "button.{s}.hover.background",
            "button.{s}.active.background",
        ] {
            m.push((k.replace("{s}", s), s.to_string()));
        }
        m.push((format!("{s}.foreground"), "on_status".into()));
        m.push((format!("button.{s}.foreground"), "on_status".into()));
    }
    m
}

// Nested objects become dotted keys; a syntax style object counts as one key.
fn flatten(prefix: &str, v: &Value, out: &mut Map<String, Value>) {
    match v {
        Value::Object(m) if prefix != "syntax" || m.is_empty() => {
            for (k, v) in m {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, v, out);
            }
        }
        Value::Object(m) => {
            for (k, v) in m {
                out.insert(format!("syntax.{k}"), v.clone());
            }
        }
        _ => {
            out.insert(prefix.to_string(), v.clone());
        }
    }
}

fn keys(v: &Value) -> BTreeSet<String> {
    let mut m = Map::new();
    flatten("", v, &mut m);
    m.keys().cloned().collect()
}

fn themes() -> Vec<Value> {
    let set: Value = serde_json::from_str(THEME_JSON).unwrap();
    set["themes"].as_array().unwrap().clone()
}

#[test]
fn the_set_holds_one_dark_and_one_light_theme() {
    let set: ThemeSet = serde_json::from_str(THEME_JSON).unwrap();
    let names: Vec<_> = set
        .themes
        .iter()
        .map(|t| (t.name.to_string(), t.mode.is_dark()))
        .collect();
    assert_eq!(
        names,
        [(DARK.to_string(), true), (LIGHT.to_string(), false)]
    );
}

#[test]
fn every_colour_key_is_set_and_known() {
    let known = keys(&serde_json::to_value(ThemeConfigColors::default()).unwrap());
    for t in themes() {
        let got = keys(&t["colors"]);
        assert_eq!(
            got, known,
            "{}: colour keys differ from the kit's",
            t["name"]
        );
    }
}

#[test]
fn every_highlight_key_is_set_and_known() {
    let known = keys(&serde_json::to_value(HighlightThemeStyle::default()).unwrap());
    for t in themes() {
        assert_eq!(keys(&t["highlight"]), known, "{}", t["name"]);
        for (k, style) in t["highlight"]["syntax"].as_object().unwrap() {
            assert!(
                style["color"].is_string(),
                "{} syntax.{k} has no colour",
                t["name"]
            );
        }
    }
}

#[test]
fn every_colour_value_parses() {
    for t in themes() {
        let mut flat = Map::new();
        flatten("", &t["colors"], &mut flat);
        flatten("", &t["highlight"], &mut flat);
        for (k, v) in flat {
            let s = v
                .get("color")
                .unwrap_or(&v)
                .as_str()
                .unwrap_or_else(|| panic!("{k} is not a string"));
            assert!(try_parse_color(s).is_ok(), "{} {k}: {s}", t["name"]);
        }
    }
}

#[test]
fn each_key_carries_its_token() {
    let map = key_map();
    for (ix, t) in themes().iter().enumerate() {
        for (key, token) in &map {
            let &(_, dark, light) = TOKENS.iter().find(|(n, ..)| *n == token.as_str()).unwrap();
            let want = if ix == 0 { dark } else { light };
            let got = t["colors"][key.as_str()].as_str().unwrap_or("");
            assert!(
                got.eq_ignore_ascii_case(want),
                "{} {key}: {got}, want {token} {want}",
                t["name"]
            );
        }
    }
    let mapped: BTreeSet<_> = map.iter().map(|(k, _)| k.clone()).collect();
    assert_eq!(
        mapped,
        keys(&serde_json::to_value(ThemeConfigColors::default()).unwrap())
    );
}
