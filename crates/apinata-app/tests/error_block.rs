//! The error block's disclosure and Retry, and stale errors clearing on an edit.

use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::response_pane::{ErrorKind, ResponsePane};
use apinata_app::send_model::SendModel;
use apinata_app::send_model::SendState;
use apinata_app::workspace::GlobalOp;
use apinata_core::{EnvVariable, ItemPath, KeyValue, ScriptPhase, VarValue, Variable};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};
use support::*;
use wiremock::ResponseTemplate;

const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n";

fn refused_tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:1/")),
        ("b.yml", &request("b", "http://127.0.0.1:1/")),
        ("environments/dev.yml", DEV),
    ])
}

fn idle(cx: &mut TestAppContext, m: &gpui_kit::Entity<SendModel>) -> bool {
    m.read_with(cx, |m, _| matches!(m.state(), SendState::Idle))
}

#[gpui_kit::test]
fn details_open_on_demand_and_retry_is_offered(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = refused_tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    support::init(cx);
    let m = model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    let shown = view
        .read_with(cx, |p, cx| p.error_view(cx))
        .expect("an error");
    assert_eq!(
        (shown.kind, shown.title),
        (ErrorKind::Refused, "Connection refused")
    );
    assert!(present(cx, win, "response-error"));
    assert!(present(cx, win, "response-retry"));
    assert!(present(cx, win, "response-error-details-toggle"));
    assert!(
        !present(cx, win, "response-error-details"),
        "collapsed at first"
    );
    cx.update_window(win, |_, w, cx| w.click("response-error-details-toggle", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(present(cx, win, "response-error-details"));
    cx.update_window(win, |_, w, cx| w.click("response-error-details-toggle", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(!present(cx, win, "response-error-details"));
}

#[gpui_kit::test]
fn an_error_clears_on_its_own_edit_or_an_environment_change(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = refused_tree();
    let (_state, ws, root) = open_dir(cx, &col);
    clear_environment(cx, &ws, &root);
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    frame(cx, win);
    assert!(
        present(cx, win, "response-error"),
        "the send itself does not clear it"
    );
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("b.yml", "http://127.0.0.1:2/"), cx)
    });
    cx.run_until_parked();
    assert!(
        present(cx, win, "response-error"),
        "another request's edit leaves it"
    );
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("a.yml", "http://127.0.0.1:3/"), cx)
    });
    cx.run_until_parked();
    assert!(idle(cx, &model));
    assert!(!present(cx, win, "response-error"));

    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    frame(cx, win);
    assert!(present(cx, win, "response-error"));
    ws.update(cx, |w, cx| {
        w.select_environment(&root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    assert!(idle(cx, &model));
}

// The pane reads the selected environment of the root it sent to.
#[gpui_kit::test]
fn the_pane_names_the_selected_environment(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/{{nope}}")),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: x\n",
        ),
    ]);
    http_ready(cx);
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| {
        w.select_environment(&root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    assert_eq!(
        model.read_with(cx, |m, _| m.root().map(|p| p.to_path_buf())),
        Some(root.clone())
    );
    support::init(cx);
    let m = model.clone();
    let (_win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    let shown = view
        .read_with(cx, |p, cx| p.error_view(cx))
        .expect("an error");
    assert_eq!(
        shown.sentence.as_deref(),
        Some("nope is not defined in dev.")
    );
    assert_eq!(shown.add_variable.as_deref(), Some("nope"));
}

// Regression guard: green before and after this task (no red phase); clear_error must never
// drop a received response.
#[gpui_kit::test]
fn a_received_error_status_is_a_response_and_stays(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(500));
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("a.yml", "http://127.0.0.1:3/"), cx)
    });
    cx.run_until_parked();
    assert!(model.read_with(cx, |m, _| matches!(m.state(), SendState::Done(_))));
}

// A tree op rebuilds every view; a request created beside this one is not an edit of it.
#[gpui_kit::test]
fn a_fresh_error_survives_a_tree_op_on_another_item(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = refused_tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    frame(cx, win);
    let create = apinata_app::command::TreeOp::CreateRequest {
        parent: ItemPath::root(),
        name: "c".into(),
    };
    ws.update(cx, |w, cx| w.send(&root, Command::Tree(create), cx));
    cx.run_until_parked();
    assert!(col.path().join("c.yml").exists(), "the create landed");
    assert!(
        present(cx, win, "response-error"),
        "another item's tree op leaves it"
    );
    assert!(!idle(cx, &model));
}

// The watcher reloads every view on any change; only this request's own change clears it.
#[gpui_kit::test]
fn a_reload_clears_the_error_only_when_this_request_changed_on_disk(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = refused_tree();
    let w = watched(cx, &col);
    let (win, editor) = editor_for(cx, &w.ws, &w.root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    model.update(cx, |m, cx| m.send(&w.root, ip("a.yml"), cx));
    finished(cx, &model);
    frame(cx, win);
    std::fs::write(
        col.path().join("b.yml"),
        request("b", "http://127.0.0.1:2/"),
    )
    .unwrap();
    wait_for(cx, "b.yml to reload", |cx| {
        url(&ws_snap(cx, &w.ws, &w.root), "b.yml") == "http://127.0.0.1:2/"
    });
    assert!(
        present(cx, win, "response-error"),
        "another file's reload leaves it"
    );
    assert!(!idle(cx, &model));
    std::fs::write(
        col.path().join("a.yml"),
        request("a", "http://127.0.0.1:3/"),
    )
    .unwrap();
    wait_for(cx, "a.yml to reload", |cx| {
        url(&ws_snap(cx, &w.ws, &w.root), "a.yml") == "http://127.0.0.1:3/"
    });
    assert!(idle(cx, &model));
    assert!(!present(cx, win, "response-error"));
}

// A runner result opened in a tab that never sent reads the environment of the tab's root.
#[gpui_kit::test]
fn a_run_result_opened_in_a_fresh_tab_names_the_selected_environment(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/{{nope}}")),
        ("environments/dev.yml", DEV),
    ]);
    http_ready(cx);
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| {
        w.select_environment(&root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    let runner = sender(cx, &ws);
    runner.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    let result = finished(cx, &runner);
    support::init(cx);
    let shared = ws.clone();
    let (win, area) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| apinata_app::tab_area::TabArea::new(shared, w, cx))
            })
        })
        .unwrap();
    let tab = ws
        .update(cx, |w, cx| w.open_item(&root, ip("a.yml"), false, cx))
        .expect("a tab");
    cx.update_window(win, |_, w, cx| {
        area.update(cx, |t, cx| t.show_result(tab, result, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let pane = area.read_with(cx, |t, cx| {
        t.editor(tab)
            .expect("an editor")
            .read(cx)
            .response()
            .clone()
    });
    let shown = pane
        .read_with(cx, |p, cx| p.error_view(cx))
        .expect("an error");
    assert_eq!(
        shown.sentence.as_deref(),
        Some("nope is not defined in dev.")
    );
    assert!(present(cx, win, "response-add-env"));
}

// WCAG relative luminance of `c` over an opaque backdrop.
fn luminance(c: gpui_kit::Hsla) -> f32 {
    let rgb = c.to_rgb();
    let lin = |v: f32| {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(rgb.r) + 0.7152 * lin(rgb.g) + 0.0722 * lin(rgb.b)
}

// Retry reads as a control at rest: its outline holds the 3:1 WCAG asks of a control's
// boundary against the pane, in both themes.
#[gpui_kit::test]
fn the_retry_outline_is_visible_in_both_themes(cx: &mut TestAppContext) {
    use gpui_kit::component::{ActiveTheme as _, ThemeMode};
    http_ready(cx);
    let col = refused_tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    support::init(cx);
    let m = model.clone();
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        cx.update(|cx| apinata_app::theme::set_mode(mode, None, cx));
        let (outline, backdrop) = cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                let button = w.find("response-retry").bounds();
                let scale = w.scale_factor();
                let quad = w
                    .painted_quads()
                    .into_iter()
                    .find(|q| {
                        (q.bounds.origin.x.0 / scale - f32::from(button.left())).abs() < 1.
                            && (q.bounds.origin.y.0 / scale - f32::from(button.top())).abs() < 1.
                            && q.border_widths.left.0 > 0.
                    })
                    .expect("Retry's outline is painted");
                (quad.border_color, cx.theme().background)
            })
            .unwrap();
        let (a, b) = (luminance(outline), luminance(backdrop));
        let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
        assert!(
            ratio >= 3.0,
            "{mode:?}: {ratio:.2} for {outline:?} on {backdrop:?}"
        );
    }
}

fn header_edit(target: Target) -> Command {
    Command::Edit {
        target,
        edit: Edit::Headers(vec![KeyValue::new("X-A", "1")]),
    }
}

#[gpui_kit::test]
fn an_error_clears_on_a_collection_or_folder_edit(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f")),
        ("f/a.yml", &request("a", "http://127.0.0.1:1/")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, editor) = editor_for(cx, &ws, &root, "f/a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    for target in [Target::Collection, Target::Folder(ip("f"))] {
        model.update(cx, |m, cx| m.send(&root, ip("f/a.yml"), cx));
        finished(cx, &model);
        frame(cx, win);
        assert!(
            present(cx, win, "response-error"),
            "{target:?}: the send shows its error"
        );
        ws.update(cx, |w, cx| w.send(&root, header_edit(target.clone()), cx));
        cx.run_until_parked();
        assert!(
            idle(cx, &model),
            "{target:?}: an inherited header edit clears the error"
        );
    }
}

// Shows a refused send's error in a fresh editor of `f/a.yml`, then applies `edit` to `target`.
fn error_after(cx: &mut TestAppContext, target: Target, edit: Edit) -> bool {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f")),
        ("f/a.yml", &request("a", "http://127.0.0.1:1/")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, editor) = editor_for(cx, &ws, &root, "f/a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    model.update(cx, |m, cx| m.send(&root, ip("f/a.yml"), cx));
    finished(cx, &model);
    frame(cx, win);
    assert!(present(cx, win, "response-error"));
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Edit { target, edit }, cx)
    });
    cx.run_until_parked();
    !idle(cx, &model)
}

#[gpui_kit::test]
fn an_inherited_var_or_script_edit_clears_the_error(cx: &mut TestAppContext) {
    http_ready(cx);
    for target in [Target::Collection, Target::Folder(ip("f"))] {
        let vars = Edit::Vars(vec![Variable::new("v", VarValue::Text("1".into()))]);
        let script = Edit::Script(ScriptPhase::PreRequest, "let a = 1".into());
        for edit in [vars, script] {
            let field = edit.field();
            assert!(
                !error_after(cx, target.clone(), edit),
                "{target:?} {field:?} clears the error"
            );
        }
    }
}

// Auth and network are part of what a request inherits, so an edit to either clears a shown
// error as the headers, vars and scripts do. The network settings live on the collection only.
#[gpui_kit::test]
fn an_inherited_auth_edit_clears_the_error(cx: &mut TestAppContext) {
    http_ready(cx);
    for target in [Target::Collection, Target::Folder(ip("f"))] {
        let auth = Edit::Auth(apinata_core::AuthSpec::Bearer { token: "t".into() });
        assert!(
            !error_after(cx, target.clone(), auth),
            "{target:?} auth clears the error"
        );
    }
}

#[gpui_kit::test]
fn a_collection_network_edit_clears_the_error(cx: &mut TestAppContext) {
    use apinata_core::{ClientCertificate, CollectionProxy};
    http_ready(cx);
    let proxy = Edit::Proxy(CollectionProxy::Off);
    let certs = Edit::ClientCertificates(vec![ClientCertificate::pem("a.test", "c.pem", "k.pem")]);
    for edit in [proxy, certs] {
        let field = edit.field();
        assert!(
            !error_after(cx, Target::Collection, edit),
            "{field:?} clears the error"
        );
    }
}

#[gpui_kit::test]
fn an_edit_to_the_selected_environment_clears_the_error(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = refused_tree();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| {
        w.select_environment(&root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    frame(cx, win);
    assert!(present(cx, win, "response-error"));
    let variables = vec![EnvVariable::new("host", VarValue::Text("y".into()))];
    let edit = Command::Edit {
        target: Target::Environment("dev".into()),
        edit: Edit::Variables(variables),
    };
    ws.update(cx, |w, cx| w.send(&root, edit, cx));
    cx.run_until_parked();
    assert!(idle(cx, &model), "an environment edit clears the error");
}

#[gpui_kit::test]
fn a_global_reload_keeps_the_error_and_a_global_edit_clears_it(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = refused_tree();
    let globals = write_tree(&[(
        "g.yml",
        "name: g\nvariables:\n  - name: token\n    value: t\n",
    )]);
    let (_state, ws, root) = open_dir(cx, &col);
    let dir = globals.path().to_path_buf();
    ws.update(cx, |w, cx| w.set_global_dir(dir, cx));
    cx.run_until_parked();
    ws.update(cx, |w, cx| {
        w.select_global_environment(Some("g".into()), cx)
    });
    cx.run_until_parked();
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    frame(cx, win);
    assert!(present(cx, win, "response-error"));
    ws.update(cx, |w, cx| w.reload_globals(cx));
    cx.run_until_parked();
    assert!(!idle(cx, &model), "an unchanged global reload leaves it");
    let op = GlobalOp::Save {
        name: "g".into(),
        variables: vec![EnvVariable::new("token", VarValue::Text("u".into()))],
        secrets: Vec::new(),
    };
    ws.update(cx, |w, cx| w.edit_globals(op, cx));
    cx.run_until_parked();
    assert!(idle(cx, &model), "a global variable edit clears it");
}

#[gpui_kit::test]
fn the_details_sit_in_an_outlined_box(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = refused_tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    support::init(cx);
    let m = model.clone();
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.click("response-error-details-toggle", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(outlined(cx, win, "response-error-details"));
}

// An edit while an older kept send is on screen clears the failure waiting under it.
#[gpui_kit::test]
fn an_edit_clears_the_failure_held_behind_a_kept_send(cx: &mut TestAppContext) {
    use apinata_app::last_response::Stored;
    use apinata_core::{Outcome, RequestResult, Response};
    use std::time::SystemTime;

    http_ready(cx);
    let col = refused_tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    frame(cx, win);
    let kept = Stored {
        result: RequestResult::from_parts(
            ip("a.yml"),
            "a",
            Outcome::Passed,
            Some(Response::new(201).with_text_body("k")),
            Vec::new(),
        ),
        saved_at: SystemTime::UNIX_EPOCH,
        body_dropped: false,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    };
    model.update(cx, |m, cx| m.view_kept(0, kept, cx));
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("a.yml", "http://127.0.0.1:3/"), cx)
    });
    cx.run_until_parked();
    model.update(cx, |m, cx| m.back_to_latest(cx));
    assert!(idle(cx, &model), "the failure is gone, not shown again");
}

// `.env` is not part of the inherited snapshot's files, yet defining the missing variable
// there is exactly what fixes an undefined-variable error.
#[gpui_kit::test]
fn a_dotenv_change_clears_the_error(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &request("a", "http://127.0.0.1:1/{{process.env.HOST}}"),
        ),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    frame(cx, win);
    assert!(present(cx, win, "response-error"), "undefined at first");
    let env = col.path().join(".env");
    std::fs::write(&env, "HOST=x\n").unwrap();
    ws.update(cx, |w, cx| w.send(&root, Command::Reload(vec![env]), cx));
    cx.run_until_parked();
    assert!(idle(cx, &model));
    assert!(!present(cx, win, "response-error"));
}
