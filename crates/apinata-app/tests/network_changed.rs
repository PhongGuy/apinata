//! Network settings edited outside the app (a pull, an editor, an agent): the Network section
//! says so, and the next send asks once. Edits made in the app never raise the notice.

use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::config_editor::ConfigSection;
use apinata_app::tabs::TabKind;
use apinata_core::{AckState, CollectionNetwork, HostOverride, NetworkAck, saved_network};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn root_with(rows: &[(&str, &str)]) -> String {
    let mut text = String::from("opencollection: 1.0.0\ninfo:\n  name: c\n");
    if !rows.is_empty() {
        text.push_str("extensions:\n  apinata:\n    hostOverrides:\n");
        for (host, address) in rows {
            text.push_str(&format!(
                "      - host: {host}\n        address: {address}\n"
            ));
        }
    }
    text
}

fn net(rows: &[(&str, &str)]) -> CollectionNetwork {
    let mut net = CollectionNetwork::default();
    net.host_overrides = rows
        .iter()
        .map(|(h, a)| HostOverride::new(*h, *a))
        .collect();
    net
}

fn collection(rows: &[(&str, &str)]) -> tempfile::TempDir {
    let a = request("a", "http://127.0.0.1:9/ok");
    write_tree(&[("opencollection.yml", &root_with(rows)), ("a.yml", &a)])
}

const A: [(&str, &str); 1] = [("api.example.com", "203.0.113.5")];
const B: [(&str, &str); 1] = [("api.example.com", "198.51.100.7")];

// A store that already acknowledged `rows` for the collection, written before the app starts.
fn acknowledged(
    col: &tempfile::TempDir,
    rows: &[(&str, &str)],
) -> impl FnOnce(&apinata_app::persist::Paths) {
    let (root, network) = (col.path().to_path_buf(), net(rows));
    move |paths| {
        NetworkAck::update(&paths.network_ack, |a| a.acknowledge(&root, &network)).unwrap();
    }
}

fn state(s: &Started) -> AckState {
    let ack = NetworkAck::load(s.files.path().join("network-ack.json")).unwrap();
    ack.check(&s.root, &saved_network(&s.root).unwrap())
}

// How the record compares with host override `rows`, whatever the disk holds now.
fn acked_state(s: &Started, rows: &[(&str, &str)]) -> AckState {
    let ack = NetworkAck::load(s.files.path().join("network-ack.json")).unwrap();
    ack.check(&s.root, &net(rows))
}

fn outside_edit(cx: &mut TestAppContext, s: &Started, rows: &[(&str, &str)]) {
    std::fs::write(s.root.join("opencollection.yml"), root_with(rows)).unwrap();
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::ReloadAll, cx));
    let want = net(rows).host_overrides;
    wait_real(cx, |cx| {
        let content = &ws_snap(cx, &s.ws, &s.root).content;
        content
            .collection
            .network
            .as_ref()
            .is_some_and(|n| n.host_overrides == want)
    });
}

fn open_network(
    cx: &mut TestAppContext,
    s: &Started,
) -> gpui_kit::Entity<apinata_app::config_editor::ConfigEditor> {
    let tab =
        s.ws.update(cx, |w, cx| {
            w.open_settings(&s.root, TabKind::Collection, cx)
        })
        .unwrap();
    frame(cx, s.win);
    let ed = s
        .shell
        .read_with(cx, |v, cx| {
            v.tab_area().read(cx).config_editor(tab).cloned()
        })
        .unwrap();
    let ix = ed
        .read_with(cx, |e, _| {
            e.sections()
                .iter()
                .position(|x| *x == ConfigSection::Network)
        })
        .unwrap();
    cx.update_window(s.win, |_, w, cx| w.click(("config-section", ix), cx))
        .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "config-network"));
    ed
}

// The lines of the Network section's notice, or none.
fn lines(
    cx: &mut TestAppContext,
    ed: &gpui_kit::Entity<apinata_app::config_editor::ConfigEditor>,
) -> Vec<String> {
    let section = ed.read_with(cx, |e, _| e.network().cloned()).unwrap();
    section.read_with(cx, |s, _| {
        s.change_notice()
            .map(|n| n.lines.clone())
            .unwrap_or_default()
    })
}

fn notice(cx: &mut TestAppContext, s: &Started) -> bool {
    present(cx, s.win, "network-change-notice")
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, s.win);
}

fn start_send(
    cx: &mut TestAppContext,
    s: &Started,
) -> gpui_kit::Entity<apinata_app::send_model::SendModel> {
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    model
}

fn asked(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.run_until_parked();
    present(cx, s.win, "network-change-continue")
}

#[gpui_kit::test]
fn a_collection_that_sets_nothing_is_stored_without_a_word(cx: &mut TestAppContext) {
    let col = collection(&[]);
    let s = started(cx, &col);
    open_network(cx, &s);
    assert!(!notice(cx, &s));
    assert_eq!(state(&s), AckState::Unchanged);
}

#[gpui_kit::test]
fn a_first_sight_of_settings_informs_but_never_asks(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection(&A);
    let s = started(cx, &col);
    open_network(cx, &s);
    assert!(notice(cx, &s) && present(cx, s.win, "network-change-ack"));
    let model = start_send(cx, &s);
    finished(cx, &model);
    assert!(!asked(cx, &s));
    // Still flagged until the person says they have seen it.
    assert!(matches!(state(&s), AckState::FirstSeen(_)));
}

#[gpui_kit::test]
fn an_outside_change_shows_the_notice_and_asks_before_the_next_send(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    open_network(cx, &s);
    assert!(!notice(cx, &s));
    outside_edit(cx, &s, &B);
    frame(cx, s.win);
    assert!(notice(cx, &s));
    let model = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "network-change-continue");
    finished(cx, &model);
    assert_eq!(state(&s), AckState::Unchanged);
    assert!(!notice(cx, &s));
    let again = start_send(cx, &s);
    finished(cx, &again);
    assert!(!asked(cx, &s));
}

#[gpui_kit::test]
fn cancel_drops_that_send_and_keeps_the_notice(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    open_network(cx, &s);
    outside_edit(cx, &s, &B);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "network-change-cancel");
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    assert!(matches!(state(&s), AckState::Changed(_)));
    frame(cx, s.win);
    assert!(notice(cx, &s));
}

#[gpui_kit::test]
fn a_disk_flip_back_to_the_acknowledged_state_still_asks(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    outside_edit(cx, &s, &B);
    // The send uses B from memory, so the acknowledged A back on disk cannot vouch for it.
    std::fs::write(s.root.join("opencollection.yml"), root_with(&A)).unwrap();
    assert_eq!(state(&s), AckState::Unchanged);
    let _model = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
}

#[gpui_kit::test]
fn acknowledge_clears_the_notice_and_asks_nothing(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    open_network(cx, &s);
    outside_edit(cx, &s, &B);
    frame(cx, s.win);
    click(cx, &s, "network-change-ack");
    assert!(!notice(cx, &s));
    assert_eq!(state(&s), AckState::Unchanged);
    let model = start_send(cx, &s);
    finished(cx, &model);
    assert!(!asked(cx, &s));
}

fn edit_in_app(cx: &mut TestAppContext, s: &Started, rows: &[(&str, &str)]) {
    let rows = net(rows).host_overrides;
    s.ws.update(cx, |w, cx| {
        w.send(
            &s.root,
            Command::Edit {
                target: Target::Collection,
                edit: Edit::HostOverrides(rows),
            },
            cx,
        );
        w.send(&s.root, Command::Save(Target::Collection), cx);
    });
    cx.run_until_parked();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn an_edit_made_in_the_app_raises_no_notice(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection(&[]);
    let s = started(cx, &col);
    edit_in_app(cx, &s, &A);
    open_network(cx, &s);
    assert!(!notice(cx, &s));
    assert_eq!(state(&s), AckState::Unchanged);
    let model = start_send(cx, &s);
    finished(cx, &model);
    assert!(!asked(cx, &s));
}

#[gpui_kit::test]
fn an_in_app_edit_does_not_hide_an_earlier_outside_change(cx: &mut TestAppContext) {
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    outside_edit(cx, &s, &B);
    edit_in_app(cx, &s, &[B[0], ("other.example.com", "192.0.2.1")]);
    open_network(cx, &s);
    assert!(notice(cx, &s));
    assert!(matches!(state(&s), AckState::Changed(_)));
}

#[gpui_kit::test]
fn an_unreadable_record_shows_the_notice_and_asks(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection(&A);
    let s = started_with(cx, &col, |paths| {
        std::fs::write(&paths.network_ack, "not json").unwrap();
    });
    open_network(cx, &s);
    assert!(notice(cx, &s));
    let model = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "network-change-continue");
    finished(cx, &model);
    // The record stays as it was; the person is told again next time.
    assert_eq!(
        std::fs::read_to_string(s.files.path().join("network-ack.json")).unwrap(),
        "not json"
    );
}

#[gpui_kit::test]
fn the_answer_acknowledges_what_the_question_showed(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    outside_edit(cx, &s, &B);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    // A write that lands while the question is open was never shown.
    let c = [("api.example.com", "192.0.2.99")];
    std::fs::write(s.root.join("opencollection.yml"), root_with(&c)).unwrap();
    click(cx, &s, "network-change-continue");
    finished(cx, &model);
    assert!(matches!(state(&s), AckState::Changed(_)));
    assert_eq!(acked_state(&s, &B), AckState::Unchanged);
}

#[gpui_kit::test]
fn an_outside_write_right_after_an_in_app_save_still_flags(cx: &mut TestAppContext) {
    let col = collection(&[]);
    let s = started(cx, &col);
    edit_in_app(cx, &s, &A);
    assert_eq!(state(&s), AckState::Unchanged);
    std::fs::write(s.root.join("opencollection.yml"), root_with(&B)).unwrap();
    assert!(matches!(state(&s), AckState::Changed(_)));
}

#[gpui_kit::test]
fn a_run_is_asked_about_an_outside_change_too(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    outside_edit(cx, &s, &B);
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let runner = cx.new(|_| {
        apinata_app::runner_model::RunnerModel::new(ws, root, apinata_core::ItemPath::root())
    });
    assert!(runner.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "network-change-continue");
    wait_real(cx, |cx| runner.read_with(cx, |m, _| !m.is_running()));
    assert_eq!(state(&s), AckState::Unchanged);
}

#[gpui_kit::test]
fn a_download_is_asked_about_an_outside_change_too(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    outside_edit(cx, &s, &B);
    let ws = s.ws.clone();
    let dl = cx.new(|_| apinata_app::download_model::DownloadModel::new(ws));
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("body.bin");
    assert!(dl.update(cx, |m, cx| m.start(&s.root, ip("a.yml"), file, cx)));
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "network-change-cancel");
    wait_real(cx, |cx| dl.read_with(cx, |m, _| !m.is_running()));
    assert!(matches!(state(&s), AckState::Changed(_)));
}

fn proxy_root(host: &str) -> String {
    format!(
        "opencollection: 1.0.0\ninfo:\n  name: c\nconfig:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: \"{host}\"\n      port: 3128\n"
    )
}

fn env_file(name: &str, value: &str) -> String {
    format!("name: {name}\nvariables:\n  - name: p\n    value: {value}\n")
}

#[gpui_kit::test]
fn userinfo_in_a_proxy_host_reaches_neither_the_notice_nor_the_record(cx: &mut TestAppContext) {
    let a = request("a", "http://127.0.0.1:9/ok");
    let col = write_tree(&[
        (
            "opencollection.yml",
            &proxy_root("user:secretpw@proxy.test"),
        ),
        ("a.yml", &a),
    ]);
    let s = started(cx, &col);
    let ed = open_network(cx, &s);
    let shown = lines(cx, &ed).join("|");
    assert!(
        shown.contains("proxy.test") && !shown.contains("secretpw"),
        "{shown}"
    );
    click(cx, &s, "network-change-ack");
    let bytes = std::fs::read_to_string(s.files.path().join("network-ack.json")).unwrap();
    assert!(
        !bytes.contains("secretpw") && bytes.contains("proxy.test"),
        "{bytes}"
    );
}

#[gpui_kit::test]
fn switching_environment_makes_a_variable_proxy_a_change(cx: &mut TestAppContext) {
    let a = request("a", "http://127.0.0.1:9/ok");
    let (dev, prod) = (env_file("dev", "dev.test"), env_file("prod", "prod.test"));
    let col = write_tree(&[
        ("opencollection.yml", &proxy_root("{{p}}")),
        ("environments/dev.yml", &dev),
        ("environments/prod.yml", &prod),
        ("a.yml", &a),
    ]);
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, Command::SelectEnvironment(Some("dev".into())), cx)
    });
    cx.run_until_parked();
    let ed = open_network(cx, &s);
    click(cx, &s, "network-change-ack");
    assert!(!notice(cx, &s));
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, Command::SelectEnvironment(Some("prod".into())), cx)
    });
    wait_real(cx, |cx| !lines(cx, &ed).is_empty());
    frame(cx, s.win);
    assert!(lines(cx, &ed).join("|").contains("prod.test"));
    // An edit of the environment file shows too, once reloaded.
    click(cx, &s, "network-change-ack");
    std::fs::write(
        col.path().join("environments/prod.yml"),
        env_file("prod", "other.test"),
    )
    .unwrap();
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::ReloadAll, cx));
    wait_real(cx, |cx| lines(cx, &ed).join("|").contains("other.test"));
}

#[gpui_kit::test]
fn an_unreadable_record_can_be_reset_from_the_notice(cx: &mut TestAppContext) {
    let col = collection(&A);
    let s = started_with(cx, &col, |paths| {
        std::fs::write(&paths.network_ack, "not json").unwrap();
    });
    let ed = open_network(cx, &s);
    assert!(present(cx, s.win, "network-reset-acks"));
    click(cx, &s, "network-reset-acks");
    wait_real(cx, |cx| !present(cx, s.win, "network-reset-acks"));
    let moved = s.files.path().join("network-ack.json.bad");
    assert_eq!(std::fs::read_to_string(moved).unwrap(), "not json");
    // The collection reads as never seen again: informed, but not asked.
    assert!(lines(cx, &ed).iter().all(|l| !l.contains("Cannot verify")));
}

#[gpui_kit::test]
fn acknowledge_records_what_the_notice_showed_after_the_disk_changed_again(
    cx: &mut TestAppContext,
) {
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    open_network(cx, &s);
    outside_edit(cx, &s, &B);
    frame(cx, s.win);
    assert!(notice(cx, &s));
    // Written without a reload, so the notice still shows B.
    let c = [("api.example.com", "192.0.2.99")];
    std::fs::write(s.root.join("opencollection.yml"), root_with(&c)).unwrap();
    click(cx, &s, "network-change-ack");
    assert_eq!(acked_state(&s, &B), AckState::Unchanged);
    assert!(matches!(acked_state(&s, &c), AckState::Changed(_)));
}

// A collection whose proxy host is `{{ph}}`: 127.0.0.1 at the collection, `localhost` in
// folder `f`, with one request `f/b.yml` there.
fn shadowed_collection() -> tempfile::TempDir {
    let root = "opencollection: 1.0.0\ninfo:\n  name: c\nconfig:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: \"{{ph}}\"\n      port: 9\nrequest:\n  variables:\n    - name: ph\n      value: 127.0.0.1\n";
    let folder = "info:\n  name: f\n  type: folder\nrequest:\n  variables:\n    - name: ph\n      value: localhost\n";
    let b = request("b", "http://example.test/b");
    write_tree(&[
        ("opencollection.yml", root),
        ("f/folder.yml", folder),
        ("f/b.yml", &b),
    ])
}

// A store that acknowledged the collection's own settings, as resolved, before the app starts.
fn acknowledged_resolved(col: &tempfile::TempDir) -> impl FnOnce(&apinata_app::persist::Paths) {
    let root = col.path().to_path_buf();
    move |paths| {
        let c = apinata_core::Collection::open(&root).unwrap();
        let ctx = apinata_core::VarContext::new(&c, apinata_core::ContextOptions::new()).unwrap();
        let network = c.file().network();
        NetworkAck::update(&paths.network_ack, |a| {
            let snapshot = apinata_core::NetworkSnapshot::resolve(&network, &ctx, a.salt());
            a.acknowledge_snapshot(&root, &snapshot);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_folder_variable_shadowing_the_proxy_asks_before_the_send(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = shadowed_collection();
    let s = started_with(cx, &col, acknowledged_resolved(&col));
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&s.root, ip("f/b.yml"), cx));
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "network-change-cancel");
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
}

#[gpui_kit::test]
fn a_folder_variable_shadowing_the_proxy_asks_before_a_run(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = shadowed_collection();
    let s = started_with(cx, &col, acknowledged_resolved(&col));
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let runner = cx.new(|_| {
        apinata_app::runner_model::RunnerModel::new(ws, root, apinata_core::ItemPath::root())
    });
    assert!(runner.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "network-change-cancel");
    wait_real(cx, |cx| runner.read_with(cx, |m, _| !m.is_running()));
}

#[gpui_kit::test]
fn a_folder_variable_shadowing_the_proxy_asks_before_a_download(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = shadowed_collection();
    let s = started_with(cx, &col, acknowledged_resolved(&col));
    let ws = s.ws.clone();
    let dl = cx.new(|_| apinata_app::download_model::DownloadModel::new(ws));
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("body.bin");
    assert!(dl.update(cx, |m, cx| m.start(&s.root, ip("f/b.yml"), file, cx)));
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "network-change-cancel");
    wait_real(cx, |cx| dl.read_with(cx, |m, _| !m.is_running()));
    assert!(!out.path().join("body.bin").exists());
}

// The response pane over `model`, to read what it would show.
fn pane_of(
    cx: &mut TestAppContext,
    model: &gpui_kit::Entity<apinata_app::send_model::SendModel>,
) -> gpui_kit::Entity<apinata_app::response_pane::ResponsePane> {
    support::init(cx);
    let model = model.clone();
    let (_, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| apinata_app::response_pane::ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    view
}

#[gpui_kit::test]
fn a_declined_send_reads_as_a_cancel_not_a_failure(cx: &mut TestAppContext) {
    use apinata_app::response_pane::{ErrorKind, ErrorTone};
    http_ready(cx);
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    outside_edit(cx, &s, &B);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "network-change-cancel");
    let result = finished(cx, &model);
    assert_eq!(result.outcome, apinata_core::Outcome::Cancelled);
    assert!(result.response.is_none());
    let view = pane_of(cx, &model);
    let shown = view
        .read_with(cx, |p, cx| p.error_view(cx))
        .expect("a calm note");
    assert_eq!(
        (shown.kind, shown.title, shown.tone),
        (ErrorKind::Cancelled, "Cancelled", ErrorTone::Neutral)
    );
}

#[gpui_kit::test]
fn a_real_send_failure_still_shows_the_red_error(cx: &mut TestAppContext) {
    use apinata_app::response_pane::{ErrorKind, ErrorTone};
    http_ready(cx);
    // Nothing listens on port 1.
    let col = collection_for("http://127.0.0.1:1", "");
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    finished(cx, &model);
    let view = pane_of(cx, &model);
    let shown = view
        .read_with(cx, |p, cx| p.error_view(cx))
        .expect("an error");
    assert_eq!(
        (shown.kind, shown.tone),
        (ErrorKind::Refused, ErrorTone::Danger)
    );
}

#[gpui_kit::test]
fn a_declined_run_request_counts_as_cancelled(cx: &mut TestAppContext) {
    use apinata_app::runner_view::tally;
    http_ready(cx);
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    outside_edit(cx, &s, &B);
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let runner = cx.new(|_| {
        apinata_app::runner_model::RunnerModel::new(ws, root, apinata_core::ItemPath::root())
    });
    assert!(runner.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "network-change-cancel");
    wait_real(cx, |cx| runner.read_with(cx, |m, _| !m.is_running()));
    let t = runner.read_with(cx, |m, _| tally(m.rows()));
    assert_eq!((t.cancelled, t.errored, t.failed), (1, 0, 0), "{t:?}");
}

#[gpui_kit::test]
fn a_declined_download_ends_cancelled(cx: &mut TestAppContext) {
    use apinata_app::download_model::DownloadState;
    http_ready(cx);
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    outside_edit(cx, &s, &B);
    let ws = s.ws.clone();
    let dl = cx.new(|_| apinata_app::download_model::DownloadModel::new(ws));
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("body.bin");
    assert!(dl.update(cx, |m, cx| m.start(&s.root, ip("a.yml"), file, cx)));
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "network-change-cancel");
    wait_real(cx, |cx| dl.read_with(cx, |m, _| !m.is_running()));
    assert!(dl.read_with(cx, |m, _| matches!(m.state(), DownloadState::Cancelled)));
}

fn change(
    cx: &mut TestAppContext,
    ed: &gpui_kit::Entity<apinata_app::config_editor::ConfigEditor>,
) -> Option<apinata_app::network_gate::Notice> {
    let section = ed.read_with(cx, |e, _| e.network().cloned()).unwrap();
    section.read_with(cx, |s, _| s.change_notice().cloned())
}

fn edit_unsaved(cx: &mut TestAppContext, s: &Started, edit: Edit) {
    s.ws.update(cx, |w, cx| {
        w.send(
            &s.root,
            Command::Edit {
                target: Target::Collection,
                edit,
            },
            cx,
        )
    });
    cx.run_until_parked();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn an_unsaved_in_app_edit_raises_no_notice(cx: &mut TestAppContext) {
    use apinata_core::{ClientCertificate, CollectionProxy, ProxyConfig};
    let col = collection(&[]);
    let s = started(cx, &col);
    let ed = open_network(cx, &s);
    assert_eq!(change(cx, &ed), None);
    let proxy = ProxyConfig::new("http", "proxy.example.com", "3128");
    edit_unsaved(cx, &s, Edit::Proxy(CollectionProxy::Custom(proxy)));
    assert_eq!(change(cx, &ed), None);
    let cert = ClientCertificate::pem("a.test", "c.pem", "k.pem");
    edit_unsaved(cx, &s, Edit::ClientCertificates(vec![cert]));
    assert_eq!(change(cx, &ed), None);
    assert!(!notice(cx, &s));
}

#[gpui_kit::test]
fn an_outside_edit_still_raises_a_changed_notice(cx: &mut TestAppContext) {
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    let ed = open_network(cx, &s);
    assert_eq!(change(cx, &ed), None);
    outside_edit(cx, &s, &B);
    frame(cx, s.win);
    assert!(change(cx, &ed).is_some_and(|n| n.changed));
    // An unsaved edit made after the outside change does not hide it.
    let c = [("api.example.com", "192.0.2.1")];
    edit_unsaved(cx, &s, Edit::HostOverrides(net(&c).host_overrides));
    assert!(change(cx, &ed).is_some_and(|n| n.changed));
}

#[gpui_kit::test]
fn an_outside_edit_over_unsaved_in_app_edits_raises_a_changed_notice(cx: &mut TestAppContext) {
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    let ed = open_network(cx, &s);
    let c = [("api.example.com", "192.0.2.1")];
    edit_unsaved(cx, &s, Edit::HostOverrides(net(&c).host_overrides));
    assert_eq!(change(cx, &ed), None);
    std::fs::write(s.root.join("opencollection.yml"), root_with(&B)).unwrap();
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::ReloadAll, cx));
    wait_real(cx, |cx| {
        frame(cx, s.win);
        change(cx, &ed).is_some_and(|n| n.changed)
    });
}

// Waits until the send either asks or ends, so a send that should not ask cannot hang.
fn asked_or_done(
    cx: &mut TestAppContext,
    s: &Started,
    model: &gpui_kit::Entity<apinata_app::send_model::SendModel>,
) -> bool {
    wait_real(cx, |cx| {
        asked(cx, s) || model.read_with(cx, |m, _| !m.is_sending())
    });
    asked(cx, s)
}

#[gpui_kit::test]
fn an_outside_write_over_unsaved_in_app_edits_still_asks_before_the_next_send(
    cx: &mut TestAppContext,
) {
    http_ready(cx);
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    let c = [("api.example.com", "192.0.2.1")];
    edit_unsaved(cx, &s, Edit::HostOverrides(net(&c).host_overrides));
    std::fs::write(s.root.join("opencollection.yml"), root_with(&B)).unwrap();
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::ReloadAll, cx));
    wait_real(cx, |cx| {
        ws_snap(cx, &s.ws, &s.root)
            .network_notice
            .as_ref()
            .is_some_and(|n| n.changed)
    });
    let model = start_send(cx, &s);
    assert!(asked_or_done(cx, &s, &model), "an outside change asks");
}

#[gpui_kit::test]
fn own_unsaved_edits_mixed_with_a_shadowing_folder_variable_still_ask(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = shadowed_collection();
    let s = started_with(cx, &col, acknowledged_resolved(&col));
    let c = [("api.example.com", "192.0.2.1")];
    edit_unsaved(cx, &s, Edit::HostOverrides(net(&c).host_overrides));
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&s.root, ip("f/b.yml"), cx));
    assert!(asked_or_done(cx, &s, &model), "the variable's change asks");
    click(cx, &s, "network-change-cancel");
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
}

#[gpui_kit::test]
fn a_send_with_an_own_unsaved_host_override_asks(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection(&A);
    let s = started_with(cx, &col, acknowledged(&col, &A));
    let c = [("api.example.com", "192.0.2.1")];
    edit_unsaved(cx, &s, Edit::HostOverrides(net(&c).host_overrides));
    let model = start_send(cx, &s);
    assert!(asked_or_done(cx, &s, &model), "own unsaved edits ask too");
    click(cx, &s, "network-change-cancel");
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    assert_eq!(acked_state(&s, &A), AckState::Unchanged);
}

// A collection whose proxy on 127.0.0.1:9 has host `host` and bypass list `bypass`, with `p`
// set by environment `dev`.
fn env_proxy_collection(host: &str, bypass: &str, p: &str) -> tempfile::TempDir {
    let root = format!(
        "opencollection: 1.0.0\ninfo:\n  name: c\nconfig:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: \"{host}\"\n      port: 9\n      bypassProxy: \"{bypass}\"\n"
    );
    let a = request("a", "http://127.0.0.1:9/ok");
    write_tree(&[
        ("opencollection.yml", &root),
        ("environments/dev.yml", &env_file("dev", p)),
        ("a.yml", &a),
    ])
}

// Acknowledges `col` under environment `dev`, makes `own` unsaved with an unsaved draft of
// `dev`, writes `p` = `outside` into dev's file, and asserts a saved run then asks.
fn saved_run_asks_after_an_outside_environment_write(
    cx: &mut TestAppContext,
    col: &tempfile::TempDir,
    own: Edit,
    outside: &str,
) {
    http_ready(cx);
    let s = started(cx, col);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, Command::SelectEnvironment(Some("dev".into())), cx)
    });
    cx.run_until_parked();
    open_network(cx, &s);
    click(cx, &s, "network-change-ack");
    assert!(!notice(cx, &s));
    edit_unsaved(cx, &s, own);
    s.ws.update(cx, |w, cx| {
        w.send(
            &s.root,
            Command::Edit {
                target: Target::Environment("dev".into()),
                edit: Edit::Color(Some("#ff0000".into())),
            },
            cx,
        )
    });
    cx.run_until_parked();
    std::fs::write(
        col.path().join("environments/dev.yml"),
        env_file("dev", outside),
    )
    .unwrap();
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::ReloadAll, cx));
    // The draft stays in memory over the outside write, which the saved run reads.
    wait_real(cx, |cx| {
        let snap = ws_snap(cx, &s.ws, &s.root);
        let envs = snap.content.environments.clone();
        envs.iter()
            .any(|e| e.name == "dev" && e.disk == apinata_app::snapshot::DiskState::Changed)
    });
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let runner = cx.new(|_| {
        apinata_app::runner_model::RunnerModel::new(ws, root, apinata_core::ItemPath::root())
    });
    assert!(runner.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| {
        asked(cx, &s) || runner.read_with(cx, |m, _| !m.is_running())
    });
    assert!(asked(cx, &s), "the outside environment change asks");
    click(cx, &s, "network-change-cancel");
    wait_real(cx, |cx| runner.read_with(cx, |m, _| !m.is_running()));
}

#[gpui_kit::test]
fn a_saved_run_asks_about_an_outside_environment_write_kept_out_of_memory(cx: &mut TestAppContext) {
    let col = env_proxy_collection("{{p}}", "", "127.0.0.1");
    let c = [("api.example.com", "192.0.2.1")];
    let own = Edit::HostOverrides(net(&c).host_overrides);
    saved_run_asks_after_an_outside_environment_write(cx, &col, own, "localhost");
}

// An own bypass edit and the outside change of `{{p}}` both read as "bypass list changed", so
// matching the question's lines cannot tell them apart.
#[gpui_kit::test]
fn a_saved_run_asks_about_an_outside_bypass_change_beside_an_own_bypass_edit(
    cx: &mut TestAppContext,
) {
    use apinata_core::{CollectionProxy, ProxyConfig};
    let col = env_proxy_collection("127.0.0.1", "{{p}}", "a.test");
    let mut proxy = ProxyConfig::new("http", "127.0.0.1", "9");
    proxy.bypass = "{{p}},own.test".into();
    let own = Edit::Proxy(CollectionProxy::Custom(proxy));
    saved_run_asks_after_an_outside_environment_write(cx, &col, own, "b.test");
}
