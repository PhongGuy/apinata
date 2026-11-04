//! Environment and collection variables a script sets live per open collection and show,
//! marked, in the Runtime variables dialog.

use crate::support;

use apinata_app::runtime_vars::open_runtime_vars;
use apinata_core::{EnvOverlay, OverlayLayer, RuntimeVars};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

#[test]
fn overlay_entries_list_the_environment_layer_first() {
    let o = EnvOverlay::new();
    o.set_text(OverlayLayer::Collection, "b", "2");
    o.set_text(OverlayLayer::Environment, "z", "1");
    let e = o.entries();
    let got: Vec<_> = e
        .iter()
        .map(|v| (v.layer(), v.name(), v.value(), v.is_deleted()))
        .collect();
    assert_eq!(
        got,
        [
            (OverlayLayer::Environment, "z", Some("1"), false),
            (OverlayLayer::Collection, "b", Some("2"), false)
        ]
    );
}

#[gpui_kit::test]
fn the_dialog_marks_script_rows_and_clears_both_stores(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    vars.set_text("token", "abc");
    overlay.set_text(OverlayLayer::Environment, "host", "h");
    let (v, o) = (vars.clone(), overlay.clone());
    let view = cx
        .update_window(s.win, |_, w, cx| {
            open_runtime_vars(v, o, Vec::new(), Vec::new(), w, cx)
        })
        .unwrap();
    frame(cx, s.win);
    assert_eq!(
        view.read_with(cx, |v, _| v.script_rows()),
        [(
            "host".to_owned(),
            "h".to_owned(),
            "Environment, set by a script".to_owned()
        )]
    );
    assert!(
        present(cx, s.win, ("runtime-var", 0usize))
            && present(cx, s.win, ("script-var-note", 0usize))
    );
    cx.update_window(s.win, |_, w, cx| w.click("runtime-vars-clear", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(vars.is_empty() && overlay.is_empty());
    assert!(present(cx, s.win, "runtime-vars-empty"));
}

#[gpui_kit::test]
fn a_script_collection_var_lands_in_the_open_collection(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"bru.setCollectionVar('region', 'eu')\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(result.error.is_none(), "{:?}", result.error);
    let overlay =
        s.ws.read_with(cx, |w, cx| w.env_overlay(&s.root, cx))
            .expect("open");
    let names: Vec<_> = overlay
        .entries()
        .iter()
        .map(|v| (v.layer(), v.name().to_owned(), v.value().map(str::to_owned)))
        .collect();
    assert_eq!(
        names,
        [(
            OverlayLayer::Collection,
            "region".to_owned(),
            Some("eu".to_owned())
        )]
    );
}

#[gpui_kit::test]
fn a_script_var_survives_context_rebuilds(cx: &mut TestAppContext) {
    use apinata_app::command::{Command, Edit, Target};
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"bru.setCollectionVar('region', 'eu')\"\n",
        request("a", &format!("{}/set", server.uri()))
    );
    let b = request("b", &format!("{}/{{{{region}}}}", server.uri()));
    let dev = "name: dev\nvariables:\n  - name: host\n    value: x\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &a),
        ("b.yml", &b),
        ("environments/dev.yml", dev),
    ]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    let model = sender(cx, &s.ws);
    let send_b = |cx: &mut TestAppContext| {
        assert!(model.update(cx, |m, cx| m.send(&s.root, ip("b.yml"), cx)));
        let result = finished(cx, &model);
        assert!(result.error.is_none(), "{:?}", result.error);
    };
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    assert!(finished(cx, &model).error.is_none());
    // Both an environment switch and an environment edit drop the cached context.
    let select = Command::SelectEnvironment(Some("dev".into()));
    s.ws.update(cx, |w, cx| w.send(&s.root, select, cx));
    cx.run_until_parked();
    send_b(cx);
    let color = Command::Edit {
        target: Target::Environment("dev".into()),
        edit: Edit::Color(Some("#00ff00".into())),
    };
    s.ws.update(cx, |w, cx| w.send(&s.root, color, cx));
    cx.run_until_parked();
    send_b(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let paths: Vec<_> = (handle
        .block_on(server.received_requests())
        .unwrap_or_default())
    .iter()
    .map(|r| r.url.path().to_owned())
    .collect();
    assert_eq!(paths, ["/set", "/eu", "/eu"]);
}

#[gpui_kit::test]
fn a_script_env_var_never_follows_an_environment_switch(cx: &mut TestAppContext) {
    use apinata_app::command::Command;
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"bru.setEnvVar('region', 'from-script'); bru.setCollectionVar('kept', 'yes')\"\n",
        request("a", &format!("{}/set", server.uri()))
    );
    let b = request("b", &format!("{}/{{{{region}}}}", server.uri()));
    let env =
        |name: &str| format!("name: {name}\nvariables:\n  - name: region\n    value: {name}\n");
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &a),
        ("b.yml", &b),
        ("environments/staging.yml", &env("staging")),
        ("environments/prod.yml", &env("prod")),
    ]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    let model = sender(cx, &s.ws);
    let send = |cx: &mut TestAppContext, item: &str| {
        assert!(model.update(cx, |m, cx| m.send(&s.root, ip(item), cx)));
        let result = finished(cx, &model);
        assert!(result.error.is_none(), "{:?}", result.error);
    };
    let select = |cx: &mut TestAppContext, name: &str| {
        let select = Command::SelectEnvironment(Some(name.into()));
        s.ws.update(cx, |w, cx| w.send(&s.root, select, cx));
        cx.run_until_parked();
    };
    select(cx, "staging");
    send(cx, "a.yml");
    send(cx, "b.yml");
    select(cx, "prod");
    send(cx, "b.yml");
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let paths: Vec<_> = (handle
        .block_on(server.received_requests())
        .unwrap_or_default())
    .iter()
    .map(|r| r.url.path().to_owned())
    .collect();
    assert_eq!(paths, ["/set", "/from-script", "/prod"]);
    // The Runtime variables dialog reads the same overlay: only the collection row is left.
    let overlay =
        s.ws.read_with(cx, |w, cx| w.env_overlay(&s.root, cx))
            .expect("open");
    let rows: Vec<_> = overlay
        .entries()
        .iter()
        .map(|v| (v.layer(), v.name().to_owned()))
        .collect();
    assert_eq!(rows, [(OverlayLayer::Collection, "kept".to_owned())]);
}

#[gpui_kit::test]
fn a_send_running_across_a_switch_writes_nothing_into_the_next_environment(
    cx: &mut TestAppContext,
) {
    use apinata_app::command::Command;
    http_ready(cx);
    let slow = ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(500));
    let server = mock_server(cx, slow);
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: after-response\n      code: \"bru.setEnvVar('region', 'from-script')\"\n",
        request("a", &format!("{}/set", server.uri()))
    );
    let b = request("b", &format!("{}/{{{{region}}}}", server.uri()));
    let env =
        |name: &str| format!("name: {name}\nvariables:\n  - name: region\n    value: {name}\n");
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &a),
        ("b.yml", &b),
        ("environments/staging.yml", &env("staging")),
        ("environments/prod.yml", &env("prod")),
    ]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    let select = |cx: &mut TestAppContext, name: &str| {
        let select = Command::SelectEnvironment(Some(name.into()));
        s.ws.update(cx, |w, cx| w.send(&s.root, select, cx));
        cx.run_until_parked();
    };
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let received = || {
        (handle
            .block_on(server.received_requests())
            .unwrap_or_default())
        .iter()
        .map(|r| r.url.path().to_owned())
        .collect::<Vec<_>>()
    };
    select(cx, "staging");
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    // Switch while the staging send waits on its response, before its post-response script.
    wait_real(cx, |_| !received().is_empty());
    select(cx, "prod");
    assert!(finished(cx, &model).error.is_none());
    let overlay =
        s.ws.read_with(cx, |w, cx| w.env_overlay(&s.root, cx))
            .expect("open");
    assert!(overlay.env_names().is_empty());
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("b.yml"), cx)));
    assert!(finished(cx, &model).error.is_none());
    assert_eq!(received(), ["/set", "/prod"]);
}

#[gpui_kit::test]
fn deleting_the_selected_environment_drops_its_script_values(cx: &mut TestAppContext) {
    use apinata_app::command::{Command, EnvOp};
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"bru.setEnvVar('region', 'from-script')\"\n",
        request("a", &format!("{}/set", server.uri()))
    );
    let staging = "name: staging\nvariables:\n  - name: region\n    value: staging\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &a),
        ("environments/staging.yml", staging),
    ]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    let select = Command::SelectEnvironment(Some("staging".into()));
    s.ws.update(cx, |w, cx| w.send(&s.root, select, cx));
    cx.run_until_parked();
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    assert!(finished(cx, &model).error.is_none());
    let overlay =
        s.ws.read_with(cx, |w, cx| w.env_overlay(&s.root, cx))
            .expect("open");
    assert_eq!(overlay.env_names(), ["region"]);
    let delete = Command::Env(EnvOp::Delete {
        name: "staging".into(),
    });
    s.ws.update(cx, |w, cx| w.send(&s.root, delete, cx));
    cx.run_until_parked();
    assert!(overlay.env_names().is_empty());
}

// A script value that carries a secret, or that a script stored under a secret's name, is
// masked in the dialog until its own row is revealed.
#[gpui_kit::test]
fn the_dialog_masks_tainted_and_secret_script_values_until_revealed(cx: &mut TestAppContext) {
    use apinata_app::command::Command;
    use apinata_app::shell::OpenRuntimeVariables;
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"bru.setEnvVar('auth', 'Bearer ' + bru.getProcessEnv('TOKEN')); bru.setEnvVar('apikey', 'k-123'); bru.setEnvVar('plain', 'hello')\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let staging = "name: staging\nvariables:\n  - secret: true\n    name: apikey\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &a),
        (".env", "TOKEN=s3cr3t-value\n"),
        ("environments/staging.yml", staging),
    ]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    let select = Command::SelectEnvironment(Some("staging".into()));
    s.ws.update(cx, |w, cx| w.send(&s.root, select, cx));
    cx.run_until_parked();
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(
        result.error.is_none(),
        "{:?}",
        result.error.as_ref().map(|e| e.to_string())
    );
    let vars =
        s.ws.read_with(cx, |w, cx| w.runtime_vars(&s.root, cx))
            .expect("open");
    assert!(vars.tainted().iter().any(|t| t == "s3cr3t-value"));
    gpui_kit::VisualTestContext::from_window(s.win, cx).dispatch_action(OpenRuntimeVariables);
    let value = |cx: &mut TestAppContext, ix: usize| {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            Some(
                w.find(("script-var-value", ix))
                    .value()
                    .unwrap_or("")
                    .to_owned(),
            )
        })
        .unwrap()
    };
    let click = |cx: &mut TestAppContext, id: (&'static str, usize)| {
        cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
        frame(cx, s.win);
    };
    // Rows by name: apikey (a secret's name), auth (holds the .env value), plain.
    assert_eq!(value(cx, 0).as_deref(), Some(""));
    assert_eq!(value(cx, 1).as_deref(), Some(""));
    assert_eq!(value(cx, 2).as_deref(), Some("hello"));
    assert!(!present(cx, s.win, ("script-var-reveal", 2usize)));
    click(cx, ("script-var-reveal", 1));
    assert_eq!(value(cx, 1).as_deref(), Some("Bearer s3cr3t-value"));
    assert_eq!(value(cx, 0).as_deref(), Some(""));
    click(cx, ("script-var-hide", 1));
    assert_eq!(value(cx, 1).as_deref(), Some(""));
    assert!(present(cx, s.win, ("script-var-reveal", 1usize)));
    // A value typed into the tainted row stays masked, with its eye to reveal it.
    replace_input(
        cx,
        s.win,
        None,
        ("script-var-value", 1usize),
        "typed-new-value",
    );
    cx.update_window(s.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(value(cx, 1).as_deref(), Some(""));
    assert!(present(cx, s.win, ("script-var-reveal", 1usize)));
    click(cx, ("script-var-reveal", 1));
    assert_eq!(value(cx, 1).as_deref(), Some("typed-new-value"));
}
