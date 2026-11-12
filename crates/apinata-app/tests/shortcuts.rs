//! The keymap.json model: chords, effective keys, diagnostics, edits and conflicts.

use std::collections::BTreeSet;

use apinata_app::commands::{Command, Place, Scope, Target, places, registry};
use apinata_app::keymap::ToggleTheme;
use apinata_app::palette::PaletteGroup;
use apinata_app::shortcuts::*;
use gpui_kit::{KeyBinding, Keystroke, Modifiers, OperatingSystem, TestAppContext};

fn map(entries: &[(&str, &[&str])]) -> Overrides {
    entries
        .iter()
        .map(|(id, keys)| (id.to_string(), keys.iter().map(|k| k.to_string()).collect()))
        .collect()
}

fn chord(s: &str) -> Chord {
    Chord::parse(s).unwrap_or_else(|| panic!("{s} parses"))
}

fn file(s: &str) -> String {
    chord(s).to_file_string()
}

fn keys(cmds: &[Command], o: &Overrides, id: &str) -> Vec<String> {
    let c = cmds.iter().find(|c| c.id == id).expect(id);
    effective(c, o).iter().map(Chord::to_file_string).collect()
}

fn cmd(id: &'static str, scope: Scope, chords: &[&'static str]) -> Command {
    Command {
        id,
        label: id,
        group: PaletteGroup::View,
        scope,
        targets: vec![Target {
            action: Box::new(ToggleTheme),
            context: "Shell",
        }],
        default_chords: chords.to_vec(),
        in_palette: false,
        keywords: &[],
    }
}

#[test]
fn chords_parse_the_file_syntax_and_refuse_the_rest() {
    assert_eq!(file("secondary-b"), "secondary-b");
    assert_eq!(file("secondary-shift-p"), "secondary-shift-p");
    assert_eq!(file("alt-f5"), "alt-f5");
    assert_eq!(file("secondary-{"), "secondary-{");
    for bad in [
        "",
        "secondary-",
        "ctrl-k ctrl-s",
        "secondary-shift-[",
        "shift-1",
        "ctrl",
        "a->b",
        "secondary-nosuch-x",
    ] {
        assert_eq!(Chord::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn secondary_and_its_platform_modifier_are_one_chord() {
    let platform = if cfg!(target_os = "macos") {
        "cmd-k"
    } else {
        "ctrl-k"
    };
    assert_eq!(chord(platform), chord("secondary-k"));
    assert_eq!(file(platform), "secondary-k");
    assert_ne!(chord("alt-k"), chord("secondary-k"));
}

// The model reads the build target, not an `App`; the running app must agree with it, and a
// chord shows the platform key by the host's own name.
#[gpui_kit::test]
fn the_host_os_is_the_running_os(cx: &mut TestAppContext) {
    assert_eq!(cx.update(|cx| cx.platform_info().os), HOST_OS);
}

#[test]
fn a_chord_names_the_platform_key_as_its_host_does() {
    let (os, platform, secondary) = if cfg!(target_os = "macos") {
        (OperatingSystem::Mac, "cmd-k", "cmd-shift-k")
    } else if cfg!(target_os = "windows") {
        (OperatingSystem::Windows, "win-k", "ctrl-shift-k")
    } else {
        (OperatingSystem::Linux, "super-k", "ctrl-shift-k")
    };
    assert_eq!(HOST_OS, os);
    let k = Keystroke {
        modifiers: Modifiers::command(),
        key: "k".into(),
        key_char: None,
    };
    assert_eq!(unparse(&k), platform);
    assert_eq!(unparse(chord("secondary-shift-k").keystroke()), secondary);
    assert!(is_secondary(&chord("secondary-k").keystroke().modifiers));
}

#[test]
fn recorded_keystrokes_become_canonical_chords() {
    let ks = |modifiers: Modifiers, key: &str, ch: Option<&str>| Keystroke {
        modifiers,
        key: key.into(),
        key_char: ch.map(Into::into),
    };
    let sec = if cfg!(target_os = "macos") {
        Modifiers::command()
    } else {
        Modifiers::control()
    };
    let sec_shift = Modifiers { shift: true, ..sec };
    let rec = |k: Keystroke| Chord::from_keystroke(&k).map(|c| c.to_file_string());
    assert_eq!(rec(ks(sec, "b", Some("b"))), Some("secondary-b".into()));
    assert_eq!(rec(ks(sec_shift, "{", None)), Some("secondary-{".into()));
    assert_eq!(
        rec(ks(sec_shift, "p", None)),
        Some("secondary-shift-p".into())
    );
    for name in ["control", "shift", "alt", "platform", "function"] {
        assert_eq!(rec(ks(Modifiers::none(), name, None)), None, "{name}");
    }
}

#[test]
fn bare_keys_need_a_modifier_except_function_keys() {
    for k in ["b", "shift-b", "enter", "space", "-"] {
        assert!(chord(k).needs_modifier(), "{k}");
    }
    for k in ["f1", "f5", "f24", "shift-f3", "alt-b", "secondary-b"] {
        assert!(!chord(k).needs_modifier(), "{k}");
    }
}

#[test]
fn every_default_is_canonical_and_the_defaults_never_conflict() {
    let cmds = registry();
    for c in &cmds {
        for d in &c.default_chords {
            assert!(Chord::parse(d).is_some(), "{}: {d}", c.id);
        }
    }
    let none = Overrides::new();
    assert_eq!(
        keys(&cmds, &none, "tabs.previous"),
        [
            file("secondary-{"),
            file("ctrl-pageup"),
            file("ctrl-shift-tab")
        ]
    );
    assert_eq!(
        keys(&cmds, &none, "tabs.next"),
        [file("secondary-}"), file("ctrl-pagedown"), file("ctrl-tab")]
    );
    assert!(conflicted(&cmds, &none).is_empty());
}

#[test]
fn overrides_replace_unbind_or_fall_back_to_the_defaults() {
    let cmds = registry();
    let o = map(&[
        ("view.toggle_sidebar", &["secondary-\\"]),
        ("tabs.close", &[]),
        ("tabs.reopen", &["secondary-shift-["]),
        ("old.command", &["secondary-q"]),
        ("tabs.last", &["ctrl-k ctrl-l"]),
    ]);
    assert_eq!(keys(&cmds, &o, "view.toggle_sidebar"), ["secondary-\\"]);
    assert!(keys(&cmds, &o, "tabs.close").is_empty());
    assert_eq!(keys(&cmds, &o, "tabs.reopen"), ["secondary-shift-t"]);
    assert_eq!(keys(&cmds, &o, "tabs.last"), ["secondary-9"]);
    let d = diagnose(&cmds, &o);
    assert_eq!(d.unknown, ["old.command"]);
    assert_eq!(d.invalid, ["tabs.last", "tabs.reopen"]);
    assert!(d.is_invalid("tabs.reopen") && !d.is_invalid("tabs.close"));
    let w = d.warnings();
    assert_eq!(w.len(), 2, "{w:?}");
    assert!(
        w[0].contains("keymap.json") && w[0].contains("1 unknown"),
        "{w:?}"
    );
    assert!(w[1].contains("2 invalid"), "{w:?}");
    assert!(w.iter().all(|m| !m.to_lowercase().contains("bruno")));
    assert!(diagnose(&cmds, &Overrides::new()).warnings().is_empty());
}

#[test]
fn an_edit_back_to_the_defaults_removes_the_entry() {
    let cmds = registry();
    let mut o = Overrides::new();
    assign(
        &cmds,
        &mut o,
        "view.toggle_sidebar",
        chord("secondary-\\"),
        Resolution::Replace,
    );
    assert_eq!(o, map(&[("view.toggle_sidebar", &["secondary-\\"])]));
    assign(
        &cmds,
        &mut o,
        "view.toggle_sidebar",
        chord("secondary-b"),
        Resolution::Replace,
    );
    assert!(o.is_empty(), "{o:?}");
}

#[test]
fn replace_takes_the_chord_and_swap_trades_keys() {
    let cmds = registry();
    let w = chord("secondary-w");
    assert_eq!(
        conflicts_with(&cmds, &Overrides::new(), "view.toggle_sidebar", &w),
        ["tabs.close"]
    );
    let mut o = Overrides::new();
    assign(
        &cmds,
        &mut o,
        "view.toggle_sidebar",
        w.clone(),
        Resolution::Replace,
    );
    assert_eq!(
        o,
        map(&[
            ("tabs.close", &[]),
            ("view.toggle_sidebar", &["secondary-w"])
        ])
    );
    let mut o = Overrides::new();
    assign(&cmds, &mut o, "view.toggle_sidebar", w, Resolution::Swap);
    assert_eq!(
        o,
        map(&[
            ("tabs.close", &["secondary-b"]),
            ("view.toggle_sidebar", &["secondary-w"])
        ])
    );
    let mut o = Overrides::new();
    assign(
        &cmds,
        &mut o,
        "tabs.close",
        chord("ctrl-pagedown"),
        Resolution::Replace,
    );
    assert_eq!(
        keys(&cmds, &o, "tabs.next"),
        [file("secondary-}"), file("ctrl-tab")]
    );
    assert_eq!(keys(&cmds, &o, "tabs.close"), [file("ctrl-pagedown")]);
}

#[test]
fn reset_and_reset_all_keep_unknown_entries() {
    let cmds = registry();
    let mut o = map(&[
        ("view.toggle_sidebar", &["secondary-\\"]),
        ("tabs.close", &[]),
        ("tabs.last", &["not a key"]),
        ("old.command", &["secondary-q"]),
    ]);
    reset(&mut o, "tabs.close");
    reset(&mut o, "tabs.last");
    assert_eq!(
        o,
        map(&[
            ("old.command", &["secondary-q"]),
            ("view.toggle_sidebar", &["secondary-\\"])
        ])
    );
    reset_all(&cmds, &mut o);
    assert_eq!(o, map(&[("old.command", &["secondary-q"])]));
}

#[test]
fn scopes_overlap_when_equal_or_either_is_the_shell() {
    let (shell, req, env) = (
        Scope::Shell,
        Scope::Editor("RequestEditor"),
        Scope::Editor("EnvEditor"),
    );
    assert!(scopes_overlap(shell, req) && scopes_overlap(env, shell) && scopes_overlap(req, req));
    assert!(!scopes_overlap(req, env));
    let cmds = vec![
        cmd("a.one", req, &["secondary-k"]),
        cmd("a.two", env, &["secondary-k"]),
        cmd("a.three", shell, &[]),
    ];
    let none = Overrides::new();
    assert!(conflicted(&cmds, &none).is_empty());
    assert_eq!(
        conflicts_with(&cmds, &none, "a.three", &chord("secondary-k")),
        ["a.one", "a.two"]
    );
    let o = map(&[("a.three", &["secondary-k"])]);
    assert_eq!(
        conflicted(&cmds, &o),
        BTreeSet::from(["a.one", "a.two", "a.three"])
    );
}

#[test]
fn a_chord_the_kit_binds_in_text_inputs_is_flagged() {
    // `KeyBinding::new` takes no host OS, so the kit's chords are spelled for the target.
    let (a, j) = if cfg!(target_os = "macos") {
        ("cmd-a", "cmd-j")
    } else {
        ("ctrl-a", "ctrl-j")
    };
    let kit = vec![
        KeyBinding::new(a, ToggleTheme, Some("Input")),
        KeyBinding::new(j, ToggleTheme, Some("List")),
    ];
    assert!(input_chord(&kit, &chord("secondary-a")));
    assert!(!input_chord(&kit, &chord("secondary-j")));
    assert!(!input_chord(&kit, &chord("secondary-b")));
}

#[test]
fn a_pane_scope_shadows_the_shell_without_conflict() {
    let (shell, pane, other) = (
        Scope::Shell,
        Scope::Pane("ResponseBody"),
        Scope::Pane("Other"),
    );
    let editor = Scope::Editor("RequestEditor");
    assert!(!scopes_overlap(shell, pane) && !scopes_overlap(pane, shell));
    assert!(scopes_overlap(pane, pane) && !scopes_overlap(pane, other));
    assert!(!scopes_overlap(pane, editor) && !scopes_overlap(editor, pane));
    let cmds = vec![
        cmd("a.one", shell, &["secondary-k"]),
        cmd("a.two", pane, &["secondary-k"]),
    ];
    assert!(conflicted(&cmds, &Overrides::new()).is_empty());
    assert!(conflicts_with(&cmds, &Overrides::new(), "a.two", &chord("secondary-k")).is_empty());
}

#[test]
fn reset_conflict_names_who_holds_the_default_chord() {
    let cmds = registry();
    assert_eq!(reset_conflict(&Overrides::new(), "request.send"), None);
    let free = map(&[("request.send", &["secondary-k"])]);
    assert_eq!(reset_conflict(&free, "request.send"), None);
    let mut swapped = Overrides::new();
    assign(
        &cmds,
        &mut swapped,
        "request.send",
        chord("secondary-shift-p"),
        Resolution::Swap,
    );
    assert_eq!(
        reset_conflict(&swapped, "request.send"),
        Some(vec!["view.command_palette"])
    );
    assert_eq!(
        reset_conflict(&swapped, "view.command_palette"),
        Some(vec!["request.send"])
    );
}

#[test]
fn a_settled_reset_never_leaves_two_commands_on_one_chord() {
    let cmds = registry();
    let swapped = map(&[
        ("request.send", &["secondary-shift-p"]),
        ("view.command_palette", &["secondary-enter"]),
    ]);
    let mut replaced = swapped.clone();
    reset_resolving(&cmds, &mut replaced, "request.send", Resolution::Replace);
    assert_eq!(replaced, map(&[("view.command_palette", &[])]));
    let mut traded = swapped.clone();
    reset_resolving(&cmds, &mut traded, "request.send", Resolution::Swap);
    assert!(traded.is_empty(), "{traded:?}");
    assert!(conflicted(&cmds, &replaced).is_empty());
    assert!(conflicted(&cmds, &traded).is_empty());
}

#[test]
fn swap_on_reset_trades_only_the_chords_given_up() {
    let cmds = registry();
    let mut o = Overrides::new();
    assign(
        &cmds,
        &mut o,
        "tabs.previous",
        chord("ctrl-pagedown"),
        Resolution::Replace,
    );
    assert_eq!(reset_conflict(&o, "tabs.next"), Some(vec!["tabs.previous"]));
    reset_resolving(&cmds, &mut o, "tabs.next", Resolution::Swap);
    assert!(conflicted(&cmds, &o).is_empty(), "{o:?}");
    assert_eq!(keys(&cmds, &o, "tabs.next").len(), 3);
}

#[test]
fn swap_then_reset_either_side_never_shares_a_chord() {
    let cmds = registry();
    let mut o = Overrides::new();
    assign(
        &cmds,
        &mut o,
        "request.send",
        chord("secondary-shift-p"),
        Resolution::Swap,
    );
    assert!(conflicted(&cmds, &o).is_empty());
    assert!(reset_conflict(&o, "view.command_palette").is_some());
    reset_resolving(&cmds, &mut o, "view.command_palette", Resolution::Swap);
    assert!(conflicted(&cmds, &o).is_empty(), "{o:?}");
    match reset_conflict(&o, "request.send") {
        Some(_) => reset_resolving(&cmds, &mut o, "request.send", Resolution::Replace),
        None => reset(&mut o, "request.send"),
    }
    assert!(conflicted(&cmds, &o).is_empty(), "{o:?}");
    assert!(o.is_empty(), "{o:?}");
}

// A table where C sits inside B, B inside the pane A, and D stands alone.
const NESTED: &[Place] = &[
    Place {
        context: "A",
        place: "in A",
        label: "A",
        within: &[],
    },
    Place {
        context: "B",
        place: "in B",
        label: "B",
        within: &["A"],
    },
    Place {
        context: "C",
        place: "in C",
        label: "C",
        within: &["B"],
    },
    Place {
        context: "D",
        place: "in D",
        label: "D",
        within: &[],
    },
];

#[test]
fn a_scope_nested_inside_another_overlaps_it() {
    let (a, b, c, d) = (
        Scope::Pane("A"),
        Scope::Editor("B"),
        Scope::Editor("C"),
        Scope::Editor("D"),
    );
    for (x, y) in [(a, b), (b, c), (a, c)] {
        assert!(scopes_overlap_in(NESTED, x, y), "{x:?} {y:?}");
        assert!(scopes_overlap_in(NESTED, y, x), "{y:?} {x:?}");
    }
    for x in [a, b, c] {
        assert!(
            !scopes_overlap_in(NESTED, x, d),
            "{x:?} stands apart from D"
        );
    }
    assert!(nested_in(NESTED, "C", "A") && !nested_in(NESTED, "A", "C"));
    let code = Scope::Editor("CodeEditor");
    let body = Scope::Pane("ResponseBody");
    assert!(scopes_overlap(code, body) && scopes_overlap(body, code));
    assert!(!scopes_overlap(code, Scope::Editor("Sidebar")));
}

#[test]
fn the_registry_reports_chords_shadowed_inside_code_editors() {
    let cmds = registry();
    let none = Overrides::new();
    // A code editor's exit keys deliberately take Ctrl+Tab from the window's Next tab inside
    // it, so only another window-wide command is named.
    assert_eq!(
        conflicts_with(&cmds, &none, "view.toggle_sidebar", &chord("ctrl-tab")),
        ["tabs.next"]
    );
    assert!(conflicts_with(&cmds, &none, "tabs.next", &chord("ctrl-tab")).is_empty());
    let find_next = if cfg!(target_os = "macos") {
        "cmd-g"
    } else {
        "f3"
    };
    assert_eq!(
        conflicts_with(&cmds, &none, "view.focus_next_control", &chord(find_next)),
        ["view.find_next_in_response"]
    );
    assert_eq!(
        conflicts_with(
            &cmds,
            &none,
            "view.find_next_in_response",
            &chord("ctrl-tab")
        ),
        ["view.focus_next_control"]
    );
    let o = map(&[("view.focus_next_control", &[find_next])]);
    assert_eq!(
        conflicted(&cmds, &o),
        BTreeSet::from(["view.find_next_in_response", "view.focus_next_control"])
    );
}

#[test]
fn only_a_deeper_command_is_named_with_its_place() {
    let (shell, code, body) = (
        Scope::Shell,
        Scope::Editor("CodeEditor"),
        Scope::Pane("ResponseBody"),
    );
    assert_eq!(shown_place(shell, code), Some("in code editors"));
    assert_eq!(shown_place(body, code), Some("in code editors"));
    assert_eq!(shown_place(code, shell), None);
    assert_eq!(shown_place(code, body), None);
    assert_eq!(shown_place(code, code), None);
    assert!(places().iter().any(|p| p.context == "CodeEditor"));
}

#[test]
fn only_a_pane_scope_with_a_place_names_its_area() {
    assert_eq!(scope_label(Scope::Pane("Sidebar")), Some("Sidebar"));
    assert_eq!(scope_label(Scope::Pane("ResponseBody")), Some("Response"));
    assert_eq!(scope_label(Scope::Editor("CodeEditor")), None);
    assert_eq!(scope_label(Scope::Shell), None);
    assert_eq!(scope_label(Scope::Pane("Nowhere")), None);
}
