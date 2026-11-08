use crate::support;

use apinata_app::env_table::{
    EnvRow, EnvTableEvent, EnvVarTable, SECRET_HIDDEN, SECRET_UNSET, env_rows, env_vars_from,
};
use apinata_core::{EnvVariable, VarValue};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, ElementId, Entity, Focusable as _, IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, TestAppContext, Window, WindowOptions,
    div, px,
};
use support::*;

struct Host {
    table: Entity<EnvVarTable>,
    events: Vec<EnvTableEvent>,
    /// Whether the table is in the tree; a hidden one is not drawn, as under another tab.
    shown: bool,
    /// A focus target outside the table, always drawn.
    elsewhere: gpui_kit::FocusHandle,
    _sub: Subscription,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::InteractiveElement as _;
        div()
            .size_full()
            .child(div().track_focus(&self.elsewhere))
            .children(self.shown.then(|| self.table.clone()))
    }
}

fn host(
    cx: &mut TestAppContext,
    rows: Vec<EnvRow>,
) -> (AnyWindowHandle, Entity<Host>, Entity<EnvVarTable>) {
    support::init(cx);
    let (win, host) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| {
                    let table = cx.new(|cx| EnvVarTable::new(w, cx));
                    table.update(cx, |t, cx| t.load(rows, w, cx));
                    let _sub = cx.subscribe(&table, |h: &mut Host, _, e: &EnvTableEvent, _| {
                        h.events.push(e.clone())
                    });
                    Host {
                        table,
                        events: Vec::new(),
                        shown: true,
                        elsewhere: cx.focus_handle(),
                        _sub,
                    }
                })
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    let table = host.read_with(cx, |h, _| h.table.clone());
    (win, host, table)
}

fn rows() -> Vec<EnvRow> {
    vec![
        EnvRow {
            name: "host".into(),
            value: "x".into(),
            enabled: true,
            secret: false,
            source: Some(0),
            unsaved: false,
        },
        EnvRow {
            name: "token".into(),
            value: String::new(),
            enabled: true,
            secret: true,
            source: Some(1),
            unsaved: false,
        },
    ]
}

fn events(cx: &mut TestAppContext, host: &Entity<Host>) -> Vec<EnvTableEvent> {
    host.update(cx, |h, _| std::mem::take(&mut h.events))
}

fn value_of(cx: &mut TestAppContext, t: &Entity<EnvVarTable>, ix: usize) -> (bool, String) {
    let input = t
        .read_with(cx, |t, _| t.value_input(ix).cloned())
        .expect("row");
    input.read_with(cx, |i, _| {
        (i.presentation().is_masked(), i.value().to_string())
    })
}

#[test]
fn rows_round_trip_and_keep_typed_and_secret_entries() {
    let mut port = EnvVariable::new(
        "port",
        VarValue::Typed {
            kind: "number".into(),
            data: "80".into(),
        },
    );
    port.enabled = false;
    let mut token = EnvVariable::new("token", VarValue::Text(String::new()));
    token.secret = true;
    let base = vec![
        EnvVariable::new("host", VarValue::Text("x".into())),
        port,
        token,
    ];
    let mut r = env_rows(&base);
    assert_eq!(
        r[1],
        EnvRow {
            name: "port".into(),
            value: "80".into(),
            enabled: false,
            secret: false,
            source: Some(1),
            unsaved: false,
        }
    );
    assert_eq!(
        r[2],
        EnvRow {
            name: "token".into(),
            value: String::new(),
            enabled: true,
            secret: true,
            source: Some(2),
            unsaved: false,
        }
    );
    r[1].value = "81".into();
    r[2].value = "leak".into();
    r.remove(0);
    r.push(EnvRow {
        name: "new".into(),
        value: "v".into(),
        enabled: true,
        ..EnvRow::default()
    });
    let out = env_vars_from(&base, &r);
    assert_eq!(
        out.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
        ["port", "token", "new"]
    );
    assert!(
        out[0].value
            == VarValue::Typed {
                kind: "number".into(),
                data: "81".into()
            }
            && !out[0].enabled
    );
    assert!(
        out[1].secret && out[1].value == VarValue::Text(String::new()),
        "a secret row's text never reaches the file"
    );
    assert!(!out[2].secret && out[2].value == VarValue::Text("v".into()));
}

#[gpui_kit::test]
fn a_plain_value_edit_sends_the_whole_list(cx: &mut TestAppContext) {
    let (win, host, _t) = host(cx, rows());
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    let Some(EnvTableEvent::Changed(r)) = events(cx, &host).pop() else {
        panic!("no change")
    };
    assert_eq!((r[0].value.as_str(), r[1].secret, r.len()), ("xy", true, 2));
}

#[gpui_kit::test]
fn a_secret_row_is_masked_and_commits_its_value_on_enter(cx: &mut TestAppContext) {
    let (win, host, t) = host(cx, rows());
    assert_eq!(value_of(cx, &t, 1), (true, String::new()));
    let name = t.read_with(cx, |t, _| t.name_input(1).cloned()).unwrap();
    assert!(
        name.read_with(cx, |i, _| {
            let p = i.presentation();
            p.is_readonly() && !p.is_disabled()
        }),
        "the stored value is keyed by the name, which still reads as normal text"
    );
    cx.update_window(win, |_, w, cx| {
        w.click(("env-value", 1usize), cx);
        w.input("s3cret", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        events(cx, &host).is_empty(),
        "typing a secret drafts nothing"
    );
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        events(cx, &host),
        [EnvTableEvent::SecretValue {
            name: "token".into(),
            value: "s3cret".into()
        }]
    );
    assert_eq!(t.read_with(cx, |t, cx| t.rows(cx))[1].value, "");
}

#[gpui_kit::test]
fn reveal_and_the_secret_toggle_are_requests_not_edits(cx: &mut TestAppContext) {
    let (win, host, t) = host(cx, rows());
    assert!(present(cx, win, ("env-reveal", 1usize)) && !present(cx, win, ("env-reveal", 0usize)));
    cx.update_window(win, |_, w, cx| {
        w.click(("env-reveal", 1usize), cx);
        w.click(("env-secret", 0usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        events(cx, &host),
        [
            EnvTableEvent::Reveal("token".into()),
            EnvTableEvent::SetSecret {
                name: "host".into(),
                secret: true
            },
        ]
    );
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.show_secret("token", Some("v1".into()), w, cx))
    })
    .unwrap();
    assert_eq!(value_of(cx, &t, 1), (false, "v1".into()));
    cx.update_window(win, |_, w, cx| t.update(cx, |t, cx| t.load(rows(), w, cx)))
        .unwrap();
    assert_eq!(
        value_of(cx, &t, 1),
        (false, "v1".into()),
        "an echo keeps a revealed value"
    );
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.hide_secret("token", w, cx))
    })
    .unwrap();
    assert_eq!(
        (t.read_with(cx, |t, _| t.revealed(1)), value_of(cx, &t, 1)),
        (false, (true, String::new()))
    );
}

#[gpui_kit::test]
fn delete_and_typing_in_the_empty_row_change_the_list(cx: &mut TestAppContext) {
    let (win, host, t) = host(cx, rows());
    let found = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            [
                ("env-name", 2usize),
                ("env-enabled", 2),
                ("env-secret", 2),
                ("env-delete", 2),
            ]
            .map(|id| w.try_find(id).is_some())
        })
        .unwrap();
    assert_eq!(found, [true, false, false, false]);
    assert!(
        cx.update_window(win, |_, w, _| w.try_find("env-add").is_none())
            .unwrap()
    );
    assert_eq!(t.read_with(cx, |t, cx| t.rows(cx)), rows());
    cx.update_window(win, |_, w, cx| w.click(("env-delete", 0usize), cx))
        .unwrap();
    frame(cx, win);
    type_at_end(cx, win, None, ("env-name", 1usize), "n");
    cx.run_until_parked();
    let ev = events(cx, &host);
    let Some(EnvTableEvent::Changed(after)) = ev.last() else {
        panic!("{ev:?}")
    };
    assert_eq!(
        after
            .iter()
            .map(|r| (r.name.as_str(), r.enabled, r.secret, r.source))
            .collect::<Vec<_>>(),
        [("token", true, true, Some(1)), ("n", true, false, None)]
    );
}

#[gpui_kit::test]
fn tab_from_the_last_value_reaches_lock_and_remove_then_the_empty_row(cx: &mut TestAppContext) {
    let (win, _host, t) = host(cx, vec![rows()[0].clone()]);
    let blank = t.read_with(cx, |t, _| t.name_input(1).cloned()).unwrap();
    cx.update_window(win, |_, w, cx| w.click(("env-value", 0usize), cx))
        .unwrap();
    // The row's lock and Remove are the Tab stops between its value and the empty row.
    for stop in ["env-secret", "env-delete"] {
        cx.simulate_keystrokes(win, "tab");
        cx.run_until_parked();
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            assert_eq!(w.find((stop, 0usize)).focused(), Some(true), "{stop}");
        })
        .unwrap();
    }
    cx.simulate_keystrokes(win, "tab");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        assert!(blank.focus_handle(cx).is_focused(w))
    })
    .unwrap();
}

// Locking the environment disables each saved row's lock, and the accessibility tree says so.
#[gpui_kit::test]
fn a_locked_environment_reports_its_secrecy_locks_disabled(cx: &mut TestAppContext) {
    let (win, _host, t) = host(cx, rows());
    let disabled = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            (0..2usize)
                .map(|ix| w.find(("env-secret", ix)).disabled() == Some(true))
                .collect::<Vec<_>>()
        })
        .unwrap()
    };
    assert_eq!(disabled(cx), [false, false]);
    t.update(cx, |t, cx| t.set_secret_locked(true, cx));
    cx.run_until_parked();
    assert_eq!(disabled(cx), [true, true]);
}

// A disabled lock is no Tab stop: with the environment locked, Tab walks the whole table
// without ever landing on a row's lock.
#[gpui_kit::test]
fn a_disabled_secrecy_lock_is_skipped_by_tab(cx: &mut TestAppContext) {
    let (win, _host, t) = host(cx, rows());
    t.update(cx, |t, cx| t.set_secret_locked(true, cx));
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("env-value", 0usize), cx)
    })
    .unwrap();
    for stop in 0..8 {
        cx.simulate_keystrokes(win, "tab");
        cx.run_until_parked();
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            for ix in 0..2usize {
                assert_ne!(
                    w.find(("env-secret", ix)).focused(),
                    Some(true),
                    "Tab {stop} landed on the disabled lock of row {ix}"
                );
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_reload_never_gives_the_empty_row_to_a_secret(cx: &mut TestAppContext) {
    let (win, _host, t) = host(cx, rows());
    let blank = t.read_with(cx, |t, _| t.name_input(2).cloned()).unwrap();
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.load(moved_rows(), w, cx))
    })
    .unwrap();
    let n = t.read_with(cx, |t, cx| t.rows(cx).len());
    assert_eq!(
        t.read_with(cx, |t, _| t.name_input(n).cloned()),
        Some(blank)
    );
}

#[gpui_kit::test]
fn the_empty_env_row_lines_up_with_the_real_rows(cx: &mut TestAppContext) {
    let (win, _host, _t) = host(cx, vec![rows()[0].clone()]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        for id in ["env-name", "env-value"] {
            let real = w.find((id, 0usize)).bounds();
            let blank = w.find((id, 1usize)).bounds();
            assert_eq!(
                (blank.left(), blank.size.width),
                (real.left(), real.size.width),
                "{id}"
            );
        }
    })
    .unwrap();
    // The checkbox and Delete slots only hold space; nothing in them is shown.
    let hidden = cx
        .update_window(win, |_, w, _| {
            [("env-slot-enabled", 1usize), ("env-slot-delete", 1)].map(|id| !w.find(id).visible())
        })
        .unwrap();
    assert_eq!(hidden, [true, true]);
    // The Secret slot is bare space, so no hidden lock reaches the accessibility tree.
    let lock = cx
        .update_window(win, |_, w, _| {
            w.try_find(("env-slot-secret", 1usize)).is_some()
        })
        .unwrap();
    assert!(!lock, "the empty row holds no lock");
}

#[gpui_kit::test]
fn a_secret_value_commits_on_blur_but_not_twice(cx: &mut TestAppContext) {
    let (win, host, _t) = host(cx, rows());
    // Focus events carry empty paths while the window is inactive, so no blur would fire.
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    let commit = |value: &str| EnvTableEvent::SecretValue {
        name: "token".into(),
        value: value.into(),
    };
    // Blur is seen by comparing drawn frames, so each focus change gets a frame.
    type_at_end(cx, win, None, ("env-value", 1usize), "a");
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-name", 0usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(
        events(cx, &host),
        [commit("a")],
        "blur after Enter sends nothing more"
    );
    type_at_end(cx, win, None, ("env-value", 1usize), "b");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-name", 0usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(events(cx, &host), [commit("ab")]);
}

#[gpui_kit::test]
fn requests_commit_pending_secret_typing_first(cx: &mut TestAppContext) {
    let (win, host, t) = host(cx, rows());
    type_at_end(cx, win, None, ("env-value", 1usize), "s");
    cx.update_window(win, |_, w, cx| w.click(("env-reveal", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        events(cx, &host),
        [
            EnvTableEvent::SecretValue {
                name: "token".into(),
                value: "s".into()
            },
            EnvTableEvent::Reveal("token".into()),
        ],
        "typing is sent before the reveal can overwrite it"
    );
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.show_secret("token", Some("s".into()), w, cx))
    })
    .unwrap();
    type_at_end(cx, win, None, ("env-value", 1usize), "2");
    cx.update_window(win, |_, w, cx| w.click(("env-secret", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        events(cx, &host),
        [EnvTableEvent::SetSecret {
            name: "token".into(),
            secret: false
        },],
        "the owner decides what becomes of the typing once the store has answered"
    );
    assert!(t.read_with(cx, |t, cx| t.has_secret_typing(cx)));
}

#[gpui_kit::test]
fn take_typing_hands_over_a_rows_typing_once_and_silently(cx: &mut TestAppContext) {
    let (win, host, t) = host(cx, rows());
    type_at_end(cx, win, None, ("env-value", 1usize), "s");
    cx.run_until_parked();
    let taken = t.update(cx, |t, cx| {
        (
            t.take_typing("token", cx),
            t.take_typing("token", cx),
            t.take_typing("host", cx),
            t.has_secret_typing(cx),
        )
    });
    assert_eq!(taken, (Some("s".into()), None, None, false));
    assert_eq!(events(cx, &host), []);
}

#[gpui_kit::test]
fn an_echo_keeps_secret_typing_even_when_rows_move(cx: &mut TestAppContext) {
    let (win, host, t) = host(cx, rows());
    type_at_end(cx, win, None, ("env-value", 1usize), "abc");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| t.update(cx, |t, cx| t.load(rows(), w, cx)))
        .unwrap();
    assert_eq!(value_of(cx, &t, 1), (true, "abc".into()));
    let mut moved = rows();
    moved.insert(
        0,
        EnvRow {
            name: "first".into(),
            value: "1".into(),
            enabled: true,
            ..EnvRow::default()
        },
    );
    cx.update_window(win, |_, w, cx| t.update(cx, |t, cx| t.load(moved, w, cx)))
        .unwrap();
    assert_eq!(value_of(cx, &t, 2), (true, "abc".into()));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.click(("env-value", 2usize), cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        events(cx, &host),
        [EnvTableEvent::SecretValue {
            name: "token".into(),
            value: "abc".into()
        }]
    );
}

#[test]
fn a_secret_event_never_prints_its_value() {
    let event = EnvTableEvent::SecretValue {
        name: "token".into(),
        value: "s3cret".into(),
    };
    let shown = format!("{event:?}");
    assert!(
        shown.contains("token") && !shown.contains("s3cret"),
        "{shown}"
    );
}

#[gpui_kit::test]
fn typing_after_a_reload_moves_a_secret_row_stays_secret(cx: &mut TestAppContext) {
    let (win, host, t) = host(cx, rows());
    type_at_end(cx, win, None, ("env-value", 1usize), "abc");
    frame(cx, win);
    let moved = moved_rows();
    cx.update_window(win, |_, w, cx| t.update(cx, |t, cx| t.load(moved, w, cx)))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.input("def", cx))
        .unwrap();
    cx.run_until_parked();
    let leaked = events(cx, &host)
        .iter()
        .any(|e| format!("{e:?}").contains("def"));
    assert!(!leaked, "secret typing never reaches a draft");
    assert_eq!(value_of(cx, &t, 1), (false, "x".into()));
    assert_eq!(value_of(cx, &t, 2), (true, "abcdef".into()));
    // A reload that replaces the focused secret row must not hand its input to a plain row.
    let mut replaced = moved_rows();
    replaced[2] = EnvRow {
        name: "other".into(),
        value: "o".into(),
        enabled: true,
        ..EnvRow::default()
    };
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.load(replaced, w, cx))
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.input("ghi", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(
        events(cx, &host).is_empty(),
        "nothing typed reaches a plain row"
    );
    assert_eq!(value_of(cx, &t, 2), (false, "o".into()));
}

/// `rows()` with a plain row inserted above them, as an outside edit would.
fn moved_rows() -> Vec<EnvRow> {
    let mut moved = rows();
    moved.insert(
        0,
        EnvRow {
            name: "first".into(),
            value: "1".into(),
            enabled: true,
            ..EnvRow::default()
        },
    );
    moved
}

fn placeholder(cx: &mut TestAppContext, t: &Entity<EnvVarTable>, ix: usize) -> String {
    let input = t
        .read_with(cx, |t, _| t.value_input(ix).cloned())
        .expect("row");
    input.read_with(cx, |i, _| i.presentation().placeholder().to_string())
}

#[gpui_kit::test]
fn a_hidden_secret_shows_dots_and_a_revealed_one_hides_again(cx: &mut TestAppContext) {
    let (win, _host, t) = host(cx, rows());
    assert_eq!(placeholder(cx, &t, 0), "Value");
    assert_eq!(placeholder(cx, &t, 1), SECRET_HIDDEN);
    assert!(!present(cx, win, ("env-hide", 1usize)));
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.show_secret("token", None, w, cx))
    })
    .unwrap();
    assert_eq!(placeholder(cx, &t, 1), SECRET_UNSET);
    assert!(present(cx, win, ("env-hide", 1usize)));
    assert!(!present(cx, win, ("env-reveal", 1usize)));
    cx.update_window(win, |_, w, cx| w.click(("env-hide", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert!(!t.read_with(cx, |t, _| t.revealed(1)));
    assert_eq!(value_of(cx, &t, 1), (true, String::new()));
    assert_eq!(placeholder(cx, &t, 1), SECRET_HIDDEN);
    assert!(present(cx, win, ("env-reveal", 1usize)));
}

#[gpui_kit::test]
fn hide_stores_typing_before_masking(cx: &mut TestAppContext) {
    let (win, host, t) = host(cx, rows());
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| {
            t.show_secret("token", Some("old".into()), w, cx)
        })
    })
    .unwrap();
    type_at_end(cx, win, None, ("env-value", 1usize), "x");
    cx.run_until_parked();
    events(cx, &host);
    cx.update_window(win, |_, w, cx| w.click(("env-hide", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        events(cx, &host),
        [EnvTableEvent::SecretValue {
            name: "token".into(),
            value: "oldx".into()
        }]
    );
    assert_eq!(value_of(cx, &t, 1), (true, String::new()));
}

#[gpui_kit::test]
fn a_reused_row_that_becomes_secret_gets_the_dots(cx: &mut TestAppContext) {
    let mut plain = rows();
    plain[1].secret = false;
    let (win, _host, t) = host(cx, plain);
    assert_eq!(placeholder(cx, &t, 1), "Value");
    cx.update_window(win, |_, w, cx| t.update(cx, |t, cx| t.load(rows(), w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(placeholder(cx, &t, 1), SECRET_HIDDEN);
}

#[gpui_kit::test]
fn hide_leaves_focus_out_of_the_masked_input(cx: &mut TestAppContext) {
    let (win, host, t) = host(cx, rows());
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| {
            t.show_secret("token", Some("s3cr3t-key".into()), w, cx)
        })
    })
    .unwrap();
    type_at_end(cx, win, None, ("env-value", 1usize), "");
    cx.update_window(win, |_, w, cx| w.click(("env-hide", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    events(cx, &host);
    let input = t
        .read_with(cx, |t, _| t.value_input(1).cloned())
        .expect("row");
    let focused = cx
        .update_window(win, |_, w, cx| {
            input.read(cx).focus_handle(cx).is_focused(w)
        })
        .unwrap();
    assert!(!focused, "the masked input keeps focus after Hide");
    cx.update_window(win, |_, w, cx| w.input("X", cx)).unwrap();
    cx.run_until_parked();
    assert_eq!(value_of(cx, &t, 1), (true, String::new()));
    assert_eq!(events(cx, &host), []);
}

#[gpui_kit::test]
fn secret_rows_keep_the_columns_of_plain_rows(cx: &mut TestAppContext) {
    let (win, _host, t) = host(cx, rows());
    // Rows 0 (plain), 1 (secret) and 2 (empty) line up, and the secret's button fits its slot.
    let aligned = |cx: &mut TestAppContext, button: &'static str| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            for id in ["env-name", "env-value", "env-action-slot"] {
                let plain = w.find((id, 0usize)).bounds();
                for ix in [1usize, 2] {
                    let other = w.find((id, ix)).bounds();
                    assert_eq!(
                        (other.left(), other.size.width),
                        (plain.left(), plain.size.width),
                        "{id} on row {ix} with {button}"
                    );
                }
            }
            let delete = |ix: usize| w.find(("env-delete", ix)).bounds().left();
            assert_eq!(delete(1), delete(0), "Delete with {button}");
            let slot = w.find(("env-action-slot", 1usize)).bounds();
            let b = w.find((button, 1usize)).bounds();
            assert!(
                b.left() >= slot.left() && b.right() <= slot.right(),
                "{button} fits its slot"
            );
        })
        .unwrap();
    };
    aligned(cx, "env-reveal");
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.show_secret("token", None, w, cx))
    })
    .unwrap();
    aligned(cx, "env-hide");
}

#[gpui_kit::test]
fn env_columns_have_headings(cx: &mut TestAppContext) {
    assert_eq!(apinata_app::env_table::ENV_HEADINGS, ["Name", "Value"]);
    let (win, _host, _t) = host(cx, vec![rows()[0].clone()]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        for (head, cell) in [
            ("env-head-name", "env-name"),
            ("env-head-value", "env-value"),
        ] {
            let h = w.find(head).bounds();
            let c = w.find((cell, 0usize)).bounds();
            assert_eq!(h.left(), c.left(), "{head}");
        }
    })
    .unwrap();
}

#[test]
fn table_cells_share_one_monospace_input() {
    for (file, src) in [
        ("env_table.rs", include_str!("../src/env_table.rs")),
        ("kv_table.rs", include_str!("../src/kv_table.rs")),
        (
            "assertions_section.rs",
            include_str!("../src/assertions_section.rs"),
        ),
    ] {
        assert!(
            !src.contains("Input::new("),
            "{file}: cells go through ui::table_input"
        );
        assert!(src.contains("table_input("), "{file}");
    }
    assert!(include_str!("../src/ui.rs").contains("pub fn table_input("));
    assert!(include_str!("../src/ui.rs").contains("mono_font_family"));
}

// Delete is the compact x icon button the key-value tables use, so it is no wider than tall.
#[gpui_kit::test]
fn the_delete_control_is_an_icon_button(cx: &mut TestAppContext) {
    let (win, _host, _t) = host(cx, rows());
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let b = w.find(("env-delete", 0usize)).bounds();
        assert!(b.size.width <= b.size.height, "{b:?}");
    })
    .unwrap();
}

// Reveal and Hide are the two faces of one square eye icon button, named by its tooltip words.
#[gpui_kit::test]
fn reveal_and_hide_are_eye_icon_buttons(cx: &mut TestAppContext) {
    let (win, _host, t) = host(cx, rows());
    let look = |cx: &mut TestAppContext, id: (&'static str, usize)| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let e = w.find(id);
            (e.bounds().size, e.label().map(str::to_owned))
        })
        .unwrap()
    };
    assert!(present(cx, win, ("env-reveal", 1usize)));
    let (size, label) = look(cx, ("env-eye", 1));
    assert_eq!(
        size.width, size.height,
        "an icon button is square: {size:?}"
    );
    assert_eq!(label.as_deref(), Some("Show value"));
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.show_secret("token", None, w, cx))
    })
    .unwrap();
    assert!(present(cx, win, ("env-hide", 1usize)));
    let (size, label) = look(cx, ("env-eye", 1));
    assert_eq!(
        size.width, size.height,
        "an icon button is square: {size:?}"
    );
    assert_eq!(label.as_deref(), Some("Hide value"));
}

#[gpui_kit::test]
fn the_eye_button_toggles_twice_back_to_hidden(cx: &mut TestAppContext) {
    let (win, host, t) = host(cx, rows());
    let label = |cx: &mut TestAppContext, id: (&'static str, usize)| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(id).label().map(str::to_owned)
        })
        .unwrap()
    };
    let hidden = t.read_with(cx, |t, cx| t.rows(cx));
    cx.update_window(win, |_, w, cx| w.click(("env-reveal", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    // The table asks; the host answers with the stored value, as the environment editor does.
    assert_eq!(events(cx, &host), [EnvTableEvent::Reveal("token".into())]);
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.show_secret("token", Some("v1".into()), w, cx))
    })
    .unwrap();
    assert!(present(cx, win, ("env-hide", 1usize)));
    assert_eq!(label(cx, ("env-eye", 1)).as_deref(), Some("Hide value"));
    cx.update_window(win, |_, w, cx| w.click(("env-hide", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert!(present(cx, win, ("env-reveal", 1usize)));
    assert_eq!(label(cx, ("env-eye", 1)).as_deref(), Some("Show value"));
    assert!(!present(cx, win, ("env-hide", 1usize)));
    assert_eq!(value_of(cx, &t, 1), (true, String::new()));
    assert_eq!(placeholder(cx, &t, 1), SECRET_HIDDEN);
    assert_eq!(t.read_with(cx, |t, cx| t.rows(cx)), hidden);
}

#[gpui_kit::test]
fn secrecy_is_a_lock_in_the_value_cell(cx: &mut TestAppContext) {
    let (win, _host, _t) = host(cx, rows());
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        for (ix, secret) in [(0usize, false), (1, true)] {
            let lock = w.find(("env-secret", ix));
            assert_eq!(lock.checked(), Some(secret), "row {ix}");
            assert_eq!(lock.label(), Some("Secret"), "row {ix}");
            let l = lock.bounds();
            let cell = w.find(("env-value-cell", ix)).bounds();
            let value = w.find(("env-value", ix)).bounds();
            assert!(
                l.left() >= value.right() && l.right() <= cell.right(),
                "row {ix}: lock {l:?} follows the value inside {cell:?}"
            );
            assert!(
                l.size.width <= l.size.height + px(0.5),
                "row {ix}: an icon {l:?}"
            );
        }
        assert!(
            w.try_find("env-head-slot-secret").is_none(),
            "no Secret column"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_secret_rows_name_is_never_greyed(cx: &mut TestAppContext) {
    let (win, _host, t) = host(cx, rows());
    let name = t.read_with(cx, |t, _| t.name_input(1).cloned()).unwrap();
    assert!(!name.read_with(cx, |i, _| i.presentation().is_disabled()));
    cx.update_window(win, |_, w, cx| {
        w.click(("env-name", 1usize), cx);
        w.input("x", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(name.read_with(cx, |i, _| i.value().to_string()), "token");
}

#[gpui_kit::test]
fn the_lock_is_muted_when_off_and_an_inverted_chip_when_on(cx: &mut TestAppContext) {
    use apinata_app::style::DISABLED_OPACITY;
    use apinata_app::ui::lock_look;
    use gpui_kit::assets::IconName;
    use gpui_kit::component::ActiveTheme as _;
    support::init(cx);
    cx.update(|cx| {
        let t = cx.theme();
        let (fg, bg, muted) = (t.foreground, t.background, t.muted_foreground);
        let on = lock_look(true, false, cx);
        assert_eq!((on.icon, on.color, on.opacity), (IconName::Lock, bg, 1.));
        assert_eq!(on.fill, Some(fg), "an inverted chip");
        let off = lock_look(false, false, cx);
        assert_eq!((off.icon, off.color), (IconName::LockOpen, muted));
        assert_eq!(lock_look(true, true, cx).opacity, DISABLED_OPACITY);
    });
}

fn hover(cx: &mut TestAppContext, win: AnyWindowHandle, target: impl Into<ElementId>) {
    let target = target.into();
    cx.update_window(win, |_, w, cx| {
        w.hover(target, cx);
        w.render_frame(cx);
    })
    .unwrap();
}

fn edge(cx: &mut TestAppContext, win: AnyWindowHandle, id: (&'static str, usize)) -> bool {
    cx.update_window(win, |_, w, _| w.find(id).visible())
        .unwrap()
}

#[gpui_kit::test]
fn a_cell_shows_its_edge_on_hover_or_focus(cx: &mut TestAppContext) {
    let (win, _host, _t) = host(cx, rows());
    // Hover-only edges are the dark themes' behaviour; light ones rest visible.
    support::use_theme(cx, apinata_app::theme::DARK);
    hover(cx, win, "env-head");
    assert!(!edge(cx, win, ("env-name-edge", 0)));
    assert!(!edge(cx, win, ("env-value-edge", 0)));
    hover(cx, win, ("env-name", 0usize));
    assert!(edge(cx, win, ("env-name-edge", 0)));
    assert!(!edge(cx, win, ("env-value-edge", 0)));
    cx.update_window(win, |_, w, cx| w.click(("env-value", 0usize), cx))
        .unwrap();
    hover(cx, win, "env-head");
    assert!(
        edge(cx, win, ("env-value-edge", 0)),
        "a focused cell keeps its edge"
    );
}

#[gpui_kit::test]
fn the_heading_sits_on_the_table_head_fill(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    let (win, _host, _t) = host(cx, rows());
    let filled = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let head = w.find("env-head").bounds();
            let scale = w.scale_factor();
            let fill = cx.theme().table_head;
            w.painted_quads().into_iter().any(|q| {
                let x = q.bounds.origin.x.0 / scale;
                let y = q.bounds.origin.y.0 / scale;
                let width = q.bounds.size.width.0 / scale;
                q.background.as_solid() == Some(fill)
                    && (x - f32::from(head.left())).abs() < 1.
                    && (y - f32::from(head.top())).abs() < 1.
                    && (width - f32::from(head.size.width)).abs() < 1.
            })
        })
        .unwrap();
    assert!(filled, "the heading row is filled with table_head");
}

#[gpui_kit::test]
fn name_and_value_keep_their_minimum_in_a_narrow_table(cx: &mut TestAppContext) {
    use apinata_app::style::{ENV_CELL_MIN, at_zoom};
    let (win, _host, _t) = host(cx, rows());
    gpui_kit::VisualTestContext::from_window(win, cx)
        .simulate_resize(gpui_kit::size(px(240.), px(400.)));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        for ix in [0usize, 1, 2] {
            for id in ["env-name", "env-value"] {
                let b = w.find((id, ix)).bounds();
                assert!(
                    b.size.width >= at_zoom(ENV_CELL_MIN, 1.0) - px(0.5),
                    "{id} {ix}: {b:?}"
                );
            }
        }
    })
    .unwrap();
}

// Between the row's minimum and the width where the equal halves fit, the headings still
// start where their columns do.
#[gpui_kit::test]
fn headings_stay_over_their_columns_in_a_narrow_table(cx: &mut TestAppContext) {
    let (win, _host, _t) = host(cx, rows());
    for width in [330., 300., 280., 271.] {
        gpui_kit::VisualTestContext::from_window(win, cx)
            .simulate_resize(gpui_kit::size(px(width), px(400.)));
        frame(cx, win);
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            for (head, cell) in [
                ("env-head-name", "env-name"),
                ("env-head-value", "env-value"),
            ] {
                let h = w.find(head).bounds();
                let c = w.find((cell, 0usize)).bounds();
                assert!(
                    (h.left() - c.left()).abs() < px(0.5),
                    "{width} px: {head} {h:?} over {cell} {c:?}"
                );
            }
            assert!(
                w.try_find("env-head-slot-lock").is_none(),
                "no lock in the heading"
            );
        })
        .unwrap();
    }
}

// The heading's column slots are bare space, so a screen reader finds no control there.
#[gpui_kit::test]
fn the_heading_exposes_no_controls(cx: &mut TestAppContext) {
    use gpui_kit::Role;
    let (win, _host, _t) = host(cx, rows());
    let controls = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let head = ElementId::from("env-head");
            gpui_kit::base::test_support::snapshots(w)
                .into_iter()
                .filter(|s| s.path().contains(&head))
                .filter(|s| matches!(s.role(), Some(Role::CheckBox | Role::Button)))
                .map(|s| format!("{:?}", s.path()))
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert!(controls.is_empty(), "controls in the heading: {controls:?}");
}

#[gpui_kit::test]
fn add_row_appends_a_focused_unsaved_row_once(cx: &mut TestAppContext) {
    let (win, host, table) = host(cx, rows());
    let ix = cx
        .update_window(win, |_, w, cx| {
            table.update(cx, |t, cx| t.add_row("x", w, cx))
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(ix, 2);
    let shown = table.read_with(cx, |t, cx| t.rows(cx));
    assert_eq!(shown.len(), 3);
    assert_eq!(
        (
            shown[2].name.as_str(),
            shown[2].value.as_str(),
            shown[2].enabled,
            shown[2].secret,
            shown[2].unsaved
        ),
        ("x", "", true, false, true)
    );
    assert!(matches!(events(cx, &host).last(), Some(EnvTableEvent::Changed(r)) if r.len() == 3));
    let input = table
        .read_with(cx, |t, _| t.value_input(2).cloned())
        .unwrap();
    assert!(
        cx.update_window(win, |_, w, cx| input
            .read(cx)
            .focus_handle(cx)
            .is_focused(w))
            .unwrap()
    );
    // A name the table already holds is focused, not added again.
    let again = cx
        .update_window(win, |_, w, cx| {
            table.update(cx, |t, cx| t.add_row("host", w, cx))
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(again, 0);
    assert_eq!(table.read_with(cx, |t, cx| t.rows(cx)).len(), 3);
    let first = table
        .read_with(cx, |t, _| t.value_input(0).cloned())
        .unwrap();
    assert!(
        cx.update_window(win, |_, w, cx| first
            .read(cx)
            .focus_handle(cx)
            .is_focused(w))
            .unwrap()
    );
}

// After a row's Remove is clicked, focus lands on a neighbour's name so the editor's keys
// (Save among them) still reach their handlers.
fn delete_focus(cx: &mut TestAppContext, count: usize, ix: usize, target: usize) {
    let plain: Vec<EnvRow> = (0..count)
        .map(|n| EnvRow {
            name: format!("v{n}"),
            value: "x".into(),
            enabled: true,
            secret: false,
            source: Some(n),
            unsaved: false,
        })
        .collect();
    let (win, _host, t) = host(cx, plain);
    let want = t
        .read_with(cx, |t, _| t.name_input(target).cloned())
        .unwrap();
    cx.update_window(win, |_, w, cx| w.click(("env-delete", ix), cx))
        .unwrap();
    cx.run_until_parked();
    let on_target = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            want.focus_handle(cx).is_focused(w)
        })
        .unwrap();
    assert!(
        on_target,
        "deleting row {ix} of {count} focuses row {target}"
    );
}

#[gpui_kit::test]
fn deleting_a_middle_env_row_focuses_the_next_row(cx: &mut TestAppContext) {
    delete_focus(cx, 3, 1, 2);
}

#[gpui_kit::test]
fn deleting_the_last_env_row_focuses_the_previous_row(cx: &mut TestAppContext) {
    delete_focus(cx, 3, 2, 1);
}

#[gpui_kit::test]
fn deleting_the_only_env_row_focuses_the_empty_row(cx: &mut TestAppContext) {
    delete_focus(cx, 1, 0, 1);
}

/// A focused secret name shows the lock hint and stays read-only, never disabled, in every
/// theme: the name reads as normal text because the stored secret is keyed by it.
#[gpui_kit::test]
fn a_focused_secret_name_shows_the_lock_hint_in_every_theme(cx: &mut TestAppContext) {
    use apinata_app::theme::{DARK, HC_DARK, HC_LIGHT, LIGHT};
    for theme in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        let (win, _host, t) = host(cx, rows());
        use_theme(cx, theme);
        let name = t.read_with(cx, |t, _| t.name_input(1).cloned()).unwrap();
        let hint = |cx: &mut TestAppContext| {
            let id = name.entity_id();
            cx.update_window(win, |_, w, _| w.try_find(("lock-hint", id)).is_some())
                .unwrap()
        };
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        assert!(!hint(cx), "{theme}: no hint while unfocused");
        cx.update_window(win, |_, w, cx| {
            let handle = name.read(cx).focus_handle(cx);
            w.focus(&handle, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        assert!(hint(cx), "{theme}: a focused secret name shows the hint");
        assert!(
            name.read_with(cx, |i, _| {
                let p = i.presentation();
                p.is_readonly() && !p.is_disabled()
            }),
            "{theme}: read-only, not disabled"
        );
    }
}

#[gpui_kit::test]
fn a_press_in_the_edge_of_a_value_cell_focuses_it(cx: &mut TestAppContext) {
    let (win, _host, t) = host(cx, rows());
    let input = t.read_with(cx, |t, _| t.value_input(0).cloned()).unwrap();
    press_in(cx, win, ("env-value", 0usize), 4.);
    cx.update_window(win, |_, w, cx| {
        assert!(
            input.focus_handle(cx).is_focused(w),
            "4px press did not focus"
        )
    })
    .unwrap();
}

// Focus anywhere in a row counts, not only its cells: a load dropping a row whose action
// button holds focus leaves the table holding focus, so the editor's keys keep reaching it.
#[gpui_kit::test]
fn a_load_dropping_a_row_with_its_button_focused_keeps_focus(cx: &mut TestAppContext) {
    let plain = |name: &str| EnvRow {
        name: name.into(),
        value: "v".into(),
        enabled: true,
        ..EnvRow::default()
    };
    let (win, _host, t) = host(cx, vec![plain("a"), plain("b")]);
    // Tab from the last row's value moves to its action button, outside any cell.
    cx.update_window(win, |_, w, cx| {
        w.click(("env-value", 1usize), cx);
        w.press("tab", cx)
    })
    .unwrap();
    cx.run_until_parked();
    let focused = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.focused(cx).is_some()
        })
        .unwrap();
    let in_cell =
        holds_focus(cx, win, ("env-value", 1usize)) || holds_focus(cx, win, ("env-name", 2usize));
    assert!(
        focused && !in_cell,
        "Tab leaves the cells for the row's button"
    );
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.load(vec![plain("a")], w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(holds_focus(cx, win, "env-table"));
}

fn var(name: &str, value: &str) -> EnvRow {
    EnvRow {
        name: name.into(),
        value: value.into(),
        enabled: true,
        secret: false,
        source: None,
        unsaved: false,
    }
}

/// Load `rows` `loads` times in one update, as the app's notices can before its next frame.
fn load_before_a_draw(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    t: &Entity<EnvVarTable>,
    rows: Vec<EnvRow>,
    loads: usize,
) {
    cx.update_window(win, |_, w, cx| {
        for _ in 0..loads {
            t.update(cx, |t, cx| t.load(rows.clone(), w, cx));
        }
    })
    .unwrap();
    cx.run_until_parked();
}

// A disk write that inserts n above b and edits c, read by one load or by two before a
// draw: b's Remove keeps focus, one slot down.
fn insert_and_edit_keeps_remove(cx: &mut TestAppContext, edited: Vec<EnvRow>, loads: usize) {
    let ambc = vec![var("a", "1"), var("m", "2"), var("b", "3"), var("c", "4")];
    let (win, _host, t) = host(cx, ambc);
    cx.update_window(win, |_, w, cx| w.click(("env-name", 0usize), cx))
        .unwrap();
    tab_within_to(cx, win, "env-table", ("env-delete", 2usize));
    load_before_a_draw(cx, win, &t, edited, loads);
    assert!(holds_focus(cx, win, ("env-delete", 3usize)));
}

fn amnbc_edited_below() -> Vec<EnvRow> {
    let c = var("c", "changed");
    vec![
        var("a", "1"),
        var("m", "2"),
        var("n", "9"),
        var("b", "3"),
        c,
    ]
}

fn amnbc_edited_above() -> Vec<EnvRow> {
    let m = var("m", "changed");
    vec![
        var("a", "1"),
        m,
        var("n", "9"),
        var("b", "3"),
        var("c", "4"),
    ]
}

#[gpui_kit::test]
fn a_reload_inserting_above_and_editing_below_keeps_focus_on_the_variables_remove(
    cx: &mut TestAppContext,
) {
    insert_and_edit_keeps_remove(cx, amnbc_edited_below(), 1);
}

#[gpui_kit::test]
fn two_reloads_inserting_above_and_editing_below_keep_focus_on_the_variables_remove(
    cx: &mut TestAppContext,
) {
    insert_and_edit_keeps_remove(cx, amnbc_edited_below(), 2);
}

#[gpui_kit::test]
fn a_reload_inserting_and_editing_above_keeps_focus_on_the_variables_remove(
    cx: &mut TestAppContext,
) {
    insert_and_edit_keeps_remove(cx, amnbc_edited_above(), 1);
}

#[gpui_kit::test]
fn two_reloads_inserting_and_editing_above_keep_focus_on_the_variables_remove(
    cx: &mut TestAppContext,
) {
    insert_and_edit_keeps_remove(cx, amnbc_edited_above(), 2);
}

// After an insert above moves focus from c's Remove to its new slot, the frames that follow
// draw the focus ring there only, not on the Remove now at c's old index.
fn insert_above_moves_the_ring(cx: &mut TestAppContext, loads: usize) {
    let (win, _host, t) = host(cx, vec![var("a", "1"), var("c", "3")]);
    cx.update_window(win, |_, w, cx| w.click(("env-name", 0usize), cx))
        .unwrap();
    tab_within_to(cx, win, "env-table", ("env-delete", 1usize));
    cx.update(|cx| apinata_app::focus_mode::set(apinata_app::focus_mode::Modality::Keyboard, cx));
    frame(cx, win);
    let abc = vec![var("a", "1"), var("b", "2"), var("c", "3")];
    load_before_a_draw(cx, win, &t, abc, loads);
    assert!(ring_drawn(cx, win, ("env-delete", 2usize)));
    assert!(!ring_drawn(cx, win, ("env-delete", 1usize)));
}

#[gpui_kit::test]
fn a_reload_inserting_above_leaves_no_ring_on_the_old_variables_remove(cx: &mut TestAppContext) {
    insert_above_moves_the_ring(cx, 1);
}

#[gpui_kit::test]
fn two_reloads_inserting_above_leave_no_ring_on_the_old_variables_remove(cx: &mut TestAppContext) {
    insert_above_moves_the_ring(cx, 2);
}

// A reload moves c's Remove and the table hides before the next draw; focus then moves
// elsewhere (and `twice`, another load lands while hidden). Showing the table leaves it there.
fn hidden_load_keeps_focus_elsewhere(cx: &mut TestAppContext, twice: bool) {
    let (win, host, t) = host(cx, vec![var("a", "1"), var("c", "3")]);
    cx.update_window(win, |_, w, cx| w.click(("env-name", 0usize), cx))
        .unwrap();
    tab_within_to(cx, win, "env-table", ("env-delete", 1usize));
    let abc = vec![var("a", "1"), var("b", "2"), var("c", "3")];
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.load(abc, w, cx));
        host.update(cx, |h, cx| {
            h.shown = false;
            cx.notify()
        });
    })
    .unwrap();
    frame(cx, win);
    let elsewhere = host.read_with(cx, |h, _| h.elsewhere.clone());
    cx.update_window(win, |_, w, cx| elsewhere.focus(w, cx))
        .unwrap();
    frame(cx, win);
    let moved = cx
        .update_window(win, |_, w, _| elsewhere.is_focused(w))
        .unwrap();
    assert!(moved, "focus moved elsewhere");
    if twice {
        let nabc = vec![var("n", "0"), var("a", "1"), var("b", "2"), var("c", "3")];
        load_before_a_draw(cx, win, &t, nabc, 1);
        frame(cx, win);
    }
    host.update(cx, |h, cx| {
        h.shown = true;
        cx.notify()
    });
    frame(cx, win);
    let kept = cx
        .update_window(win, |_, w, _| elsewhere.is_focused(w))
        .unwrap();
    assert!(kept, "showing the table leaves focus where it was moved");
}

#[gpui_kit::test]
fn a_reload_hidden_before_its_draw_leaves_focus_where_it_was_moved(cx: &mut TestAppContext) {
    hidden_load_keeps_focus_elsewhere(cx, false);
}

#[gpui_kit::test]
fn a_second_reload_while_hidden_leaves_focus_where_it_was_moved(cx: &mut TestAppContext) {
    hidden_load_keeps_focus_elsewhere(cx, true);
}

// A secret row typed before its name reached the file, and a load that declares it, as a save
// from outside the table does.
fn declared_behind_the_typing(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Host>, Entity<EnvVarTable>) {
    let mut typed = rows();
    typed[1].unsaved = true;
    let (win, host, t) = host(cx, typed);
    type_at_end(cx, win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.load(rows(), w, cx));
    })
    .unwrap();
    cx.run_until_parked();
    events(cx, &host);
    (win, host, t)
}

#[gpui_kit::test]
fn a_guarded_row_releases_nothing_unconfirmed(cx: &mut TestAppContext) {
    let (win, host, t) = declared_behind_the_typing(cx);
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        events(cx, &host),
        [EnvTableEvent::TypingHeld],
        "Enter asks the owner and hands over nothing"
    );
    let taken = t.update(cx, |t, cx| {
        (t.take_pending_secrets(true, cx), t.take_typing("token", cx))
    });
    assert_eq!(taken, (Vec::new(), None));
    assert_eq!(value_of(cx, &t, 1).1, "typed", "the typing stays");
    assert!(t.read_with(cx, |t, cx| t.has_secret_typing(cx)));
}

#[gpui_kit::test]
fn a_guarded_rows_typing_still_asks_for_a_check(cx: &mut TestAppContext) {
    let (_win, _host, t) = declared_behind_the_typing(cx);
    let typing = t.read_with(cx, |t, cx| t.unsaved_secret_typing(cx));
    assert_eq!(typing, [("token".to_owned(), "typed".to_owned())]);
}

#[gpui_kit::test]
fn a_confirmed_row_releases_once(cx: &mut TestAppContext) {
    let (_win, _host, t) = declared_behind_the_typing(cx);
    t.update(cx, |t, cx| t.confirm_typing("token", cx));
    let taken = t.update(cx, |t, cx| {
        (t.take_pending_secrets(true, cx), t.take_typing("token", cx))
    });
    assert_eq!(taken.0, [("token".to_owned(), "typed".to_owned())]);
    assert_eq!(taken.1, None);
}

#[gpui_kit::test]
fn an_edit_voids_a_confirmation(cx: &mut TestAppContext) {
    let (win, _host, t) = declared_behind_the_typing(cx);
    t.update(cx, |t, cx| t.confirm_typing("token", cx));
    type_at_end(cx, win, None, ("env-value", 1usize), "2");
    cx.run_until_parked();
    let taken = t.update(cx, |t, cx| t.take_pending_secrets(true, cx));
    assert_eq!(taken, []);
    assert_eq!(value_of(cx, &t, 1).1, "typed2");
}

#[gpui_kit::test]
fn a_rename_voids_a_confirmation(cx: &mut TestAppContext) {
    let mut typed = rows();
    typed[1].unsaved = true;
    let (win, _host, t) = host(cx, typed);
    type_at_end(cx, win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    t.update(cx, |t, cx| t.confirm_typing("token", cx));
    // A secret row's name is read-only, so a rename goes through the plain state.
    cx.update_window(win, |_, w, cx| w.click(("env-secret", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    type_at_end(cx, win, None, ("env-name", 1usize), "2");
    cx.run_until_parked();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-secret", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    let taken = t.update(cx, |t, cx| t.take_pending_secrets(true, cx));
    assert_eq!(taken, []);
}

#[gpui_kit::test]
fn a_release_uses_up_a_confirmation(cx: &mut TestAppContext) {
    let mut typed = rows();
    typed[1].unsaved = true;
    let (win, _host, t) = host(cx, typed);
    type_at_end(cx, win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    t.update(cx, |t, cx| t.confirm_typing("token", cx));
    let first = t.update(cx, |t, cx| t.take_pending_secrets(true, cx));
    assert_eq!(first.len(), 1);
    type_at_end(cx, win, None, ("env-value", 1usize), "2");
    cx.run_until_parked();
    let second = t.update(cx, |t, cx| t.take_pending_secrets(true, cx));
    assert_eq!(second, []);
}

#[gpui_kit::test]
fn discarding_voids_a_confirmation(cx: &mut TestAppContext) {
    let (win, _host, t) = declared_behind_the_typing(cx);
    t.update(cx, |t, cx| t.confirm_typing("token", cx));
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.discard_secret_typing(w, cx))
    })
    .unwrap();
    type_at_end(cx, win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    // Discarded typing leaves an ordinary declared secret, which Enter stores as ever.
    assert!(
        t.read_with(cx, |t, cx| t.unsaved_secret_typing(cx))
            .is_empty()
    );
}

#[gpui_kit::test]
fn a_guard_survives_emptying(cx: &mut TestAppContext) {
    let (win, host, t) = declared_behind_the_typing(cx);
    replace_input(cx, win, None, ("env-value", 1usize), "");
    cx.run_until_parked();
    assert!(
        t.read_with(cx, |t, cx| t.unsaved_secret_typing(cx))
            .is_empty()
    );
    type_at_end(cx, win, None, ("env-value", 1usize), "x");
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(events(cx, &host), [EnvTableEvent::TypingHeld]);
    let taken = t.update(cx, |t, cx| {
        (t.take_pending_secrets(true, cx), t.take_typing("token", cx))
    });
    assert_eq!(taken, (Vec::new(), None));
    let typing = t.read_with(cx, |t, cx| t.unsaved_secret_typing(cx));
    assert_eq!(typing, [("token".to_owned(), "x".to_owned())]);
}

#[gpui_kit::test]
fn a_load_leaves_the_guard_of_an_emptied_row(cx: &mut TestAppContext) {
    let (win, host, t) = declared_behind_the_typing(cx);
    replace_input(cx, win, None, ("env-value", 1usize), "");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.load(rows(), w, cx));
    })
    .unwrap();
    cx.run_until_parked();
    type_at_end(cx, win, None, ("env-value", 1usize), "x");
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(events(cx, &host), [EnvTableEvent::TypingHeld]);
}

#[gpui_kit::test]
fn a_row_emptied_before_it_was_declared_stays_guarded(cx: &mut TestAppContext) {
    let mut typed = rows();
    typed[1].unsaved = true;
    let (win, host, t) = host(cx, typed);
    type_at_end(cx, win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    replace_input(cx, win, None, ("env-value", 1usize), "");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.load(rows(), w, cx));
    })
    .unwrap();
    cx.run_until_parked();
    events(cx, &host);
    type_at_end(cx, win, None, ("env-value", 1usize), "x");
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(events(cx, &host), [EnvTableEvent::TypingHeld]);
}

#[gpui_kit::test]
fn a_lock_flip_leaves_the_mark_of_earlier_typing(cx: &mut TestAppContext) {
    let mut typed = rows();
    typed[1].unsaved = true;
    let (win, host, t) = host(cx, typed);
    type_at_end(cx, win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    replace_input(cx, win, None, ("env-value", 1usize), "");
    cx.run_until_parked();
    for _ in 0..2 {
        frame(cx, win);
        cx.update_window(win, |_, w, cx| w.click(("env-secret", 1usize), cx))
            .unwrap();
        cx.run_until_parked();
    }
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.load(rows(), w, cx));
    })
    .unwrap();
    cx.run_until_parked();
    events(cx, &host);
    type_at_end(cx, win, None, ("env-value", 1usize), "x");
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(events(cx, &host), [EnvTableEvent::TypingHeld]);
}

#[gpui_kit::test]
fn emptying_voids_a_confirmation(cx: &mut TestAppContext) {
    let (win, _host, t) = declared_behind_the_typing(cx);
    t.update(cx, |t, cx| t.confirm_typing("token", cx));
    replace_input(cx, win, None, ("env-value", 1usize), "");
    cx.run_until_parked();
    type_at_end(cx, win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    let taken = t.update(cx, |t, cx| t.take_pending_secrets(true, cx));
    assert_eq!(taken, []);
}

#[gpui_kit::test]
fn an_emptied_guarded_row_writes_nothing(cx: &mut TestAppContext) {
    let (win, host, t) = declared_behind_the_typing(cx);
    replace_input(cx, win, None, ("env-value", 1usize), "");
    cx.run_until_parked();
    assert!(!t.read_with(cx, |t, cx| t.holds_typing(cx)));
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(events(cx, &host), []);
    let taken = t.update(cx, |t, cx| t.take_pending_secrets(true, cx));
    assert_eq!(taken, []);
}

// A release already left this row's text, so emptying it still differs from what was committed.
#[gpui_kit::test]
fn an_emptied_row_past_a_release_writes_nothing(cx: &mut TestAppContext) {
    let mut typed = rows();
    typed[1].unsaved = true;
    let (win, host, t) = host(cx, typed.clone());
    type_at_end(cx, win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    t.update(cx, |t, cx| {
        t.confirm_typing("token", cx);
        t.take_pending_secrets(true, cx);
    });
    type_at_end(cx, win, None, ("env-value", 1usize), "2");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.load(rows(), w, cx));
    })
    .unwrap();
    cx.run_until_parked();
    events(cx, &host);
    replace_input(cx, win, None, ("env-value", 1usize), "");
    cx.run_until_parked();
    assert!(!t.read_with(cx, |t, cx| t.holds_typing(cx)));
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(events(cx, &host), []);
    let taken = t.update(cx, |t, cx| t.take_pending_secrets(true, cx));
    assert_eq!(taken, []);
}

#[gpui_kit::test]
fn an_ordinary_declared_secret_stores_on_enter(cx: &mut TestAppContext) {
    let (win, host, _t) = host(cx, rows());
    type_at_end(cx, win, None, ("env-value", 1usize), "a");
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        events(cx, &host),
        [EnvTableEvent::SecretValue {
            name: "token".into(),
            value: "a".into()
        }]
    );
}

#[gpui_kit::test]
fn a_blur_leaves_a_guarded_rows_typing(cx: &mut TestAppContext) {
    let (win, host, t) = declared_behind_the_typing(cx);
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-value", 1usize), cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-name", 0usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(events(cx, &host), []);
    assert_eq!(value_of(cx, &t, 1).1, "typed");
}

#[gpui_kit::test]
fn reveal_and_hide_leave_a_guarded_rows_typing(cx: &mut TestAppContext) {
    let (win, host, t) = declared_behind_the_typing(cx);
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-reveal", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(events(cx, &host), [], "nothing to read or commit");
    assert_eq!(value_of(cx, &t, 1), (false, "typed".to_owned()));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-hide", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(events(cx, &host), []);
    assert_eq!(value_of(cx, &t, 1), (true, "typed".to_owned()));
}

#[gpui_kit::test]
fn a_saved_secrets_name_cannot_be_retyped(cx: &mut TestAppContext) {
    let (win, _host, t) = host(cx, rows());
    type_at_end(cx, win, None, ("env-name", 1usize), "other");
    cx.run_until_parked();
    let names = t.read_with(cx, |t, cx| t.rows(cx)[1].name.clone());
    assert_eq!(names, "token");
}

#[gpui_kit::test]
fn held_typing_is_declared_typing_the_table_has_not_released(cx: &mut TestAppContext) {
    let (_win, _host, t) = declared_behind_the_typing(cx);
    assert!(t.read_with(cx, |t, cx| t.holds_typing(cx)));
    t.update(cx, |t, cx| t.confirm_typing("token", cx));
    assert!(
        t.read_with(cx, |t, cx| t.holds_typing(cx)),
        "until released"
    );
    t.update(cx, |t, cx| t.take_pending_secrets(false, cx));
    assert!(!t.read_with(cx, |t, cx| t.holds_typing(cx)));
    // Typing in a row not yet in the file is the draft's to report, and an ordinary secret's
    // typing leaves on its own.
    let mut typed = rows();
    typed[1].unsaved = true;
    let (win2, _host2, unsaved) = host(cx, typed);
    type_at_end(cx, win2, None, ("env-value", 1usize), "x");
    cx.run_until_parked();
    assert!(!unsaved.read_with(cx, |t, cx| t.holds_typing(cx)));
}
