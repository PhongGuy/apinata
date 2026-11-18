//! The team sync dialog: the live problem lines and the action that fixes each one.

use crate::git_support::{Sandbox, write};
use crate::support::team::*;
use crate::support::*;
use apinata_app::team_sync_dialog::{self, SyncView};
use apinata_core::SecretStore as _;
use apinata_core::team::{Block, Role, TeamState};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, px};
use std::time::Duration;

const FAST: Duration = Duration::from_millis(50);

// Stored values long and odd enough that no temp path or message holds them by chance, so a
// check that no label shows them fails only on a real leak.
const TOKEN: &str = "secret-value-token-7f3a";
const HOST_KEY: &str = "secret-value-host-key-7f3a";

fn app(cx: &mut TestAppContext, sb: &Sandbox, col: &tempfile::TempDir) -> Started {
    http_ready(cx);
    team_started(cx, sb, col, |_, _| {}).0
}

fn open(cx: &mut TestAppContext, s: &Started) -> Entity<SyncView> {
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let view = cx
        .update_window(s.win, |_, w, cx| team_sync_dialog::open(&ws, root, w, cx))
        .unwrap();
    frame(cx, s.win);
    view
}

fn lines(cx: &mut TestAppContext, view: &Entity<SyncView>) -> Vec<String> {
    view.read_with(cx, |v, cx| v.lines(cx))
}

fn state(cx: &mut TestAppContext, s: &Started) -> Option<TeamState> {
    s.ws.read_with(cx, |w, _| w.team_status(&s.root).map(|t| t.state))
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, s.win);
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_secret_refusal_lists_the_findings_and_retry_pushes_the_fix(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    let bare = shared_origin(&sb, col.path());
    sb.fake_tool("origin.git/hooks", "pre-receive", "#!/bin/sh\nwhile read old new ref; do\n  if git log -p \"$new\" --not --all | grep -q s3cr3t; then\n    echo \"potluck-reject secret_value $ref a.yml secret_value\" >&2\n    exit 1\n  fi\ndone\n");
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    std::fs::write(col.path().join("a.yml"), request("a", "http://x/s3cr3t")).unwrap();
    wait_for(cx, "the refusal", |cx| {
        matches!(
            state(cx, &s),
            Some(TeamState::Blocked(Block::Refused { .. }))
        )
    });
    let view = open(cx, &s);
    let text = lines(cx, &view);
    assert_eq!(
        text[1],
        "a.yml: secret_value on refs/\u{2060}heads/\u{2060}main"
    );
    assert!(text.iter().all(|l| !l.contains("s3cr3t")), "{text:?}");
    std::fs::write(col.path().join("a.yml"), request("a", "http://x/fixed")).unwrap();
    wait_for(cx, "the fix commit", |cx| {
        s.ws.read_with(cx, |w, _| {
            w.team_status(&s.root).is_some_and(|t| t.unpushed == 2)
        })
    });
    click(cx, &s, "team-retry");
    wait_for(cx, "the squashed push", |_| {
        origin_has(&sb, &bare, "a.yml", "x/fixed")
    });
    wait_for(cx, "in sync", |cx| lines(cx, &view) == ["In sync."]);
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_refusing_hook_shows_its_output_and_retry_commits_once_it_passes(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    let (sb, col) = (Sandbox::new(), collection());
    let bare = shared_origin(&sb, col.path());
    let hook = col.path().join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho nope >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    wait_for(cx, "the hook block", |cx| {
        matches!(state(cx, &s), Some(TeamState::Blocked(Block::Hook { .. })))
    });
    let view = open(cx, &s);
    assert_eq!(
        lines(cx, &view),
        ["A commit hook refused the commit:", "nope"]
    );
    std::fs::remove_file(&hook).unwrap();
    click(cx, &s, "team-retry");
    wait_for(cx, "the push", |_| origin_has(&sb, &bare, "b.yml", "x/b"));
}

#[gpui_kit::test]
fn discard_drops_outside_edits_in_a_read_only_folder(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Read, FAST);
    std::fs::write(col.path().join("a.yml"), request("a", "http://x/outside")).unwrap();
    wait_for(cx, "paused", |cx| {
        state(cx, &s) == Some(TeamState::ReadOnly { paused: true })
    });
    let view = open(cx, &s);
    assert!(shown(cx, s.win, "team-discard"));
    assert_eq!(lines(cx, &view).len(), 2);
    click(cx, &s, "team-discard");
    wait_for(cx, "resumed", |cx| {
        state(cx, &s) == Some(TeamState::ReadOnly { paused: false })
    });
    assert!(
        std::fs::read_to_string(col.path().join("a.yml"))
            .unwrap()
            .contains("x/0")
    );
}

#[gpui_kit::test]
fn an_idle_folder_offers_no_action(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let view = open(cx, &s);
    assert_eq!(lines(cx, &view), ["In sync."]);
    assert!(!shown(cx, s.win, "team-retry"));
    assert!(!shown(cx, s.win, "team-discard"));
    assert!(cx.update(|cx| team_sync_dialog::current(cx)).is_some());
}

#[cfg(unix)]
#[gpui_kit::test]
fn long_hook_output_scrolls_and_keeps_retry_in_the_window(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let hook = col.path().join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    let script = "#!/bin/sh\ni=0\nwhile [ $i -lt 200 ]; do echo \"refused line $i\" >&2; i=$((i+1)); done\nexit 1\n";
    std::fs::write(&hook, script).unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    wait_for(cx, "the hook block", |cx| {
        matches!(state(cx, &s), Some(TeamState::Blocked(Block::Hook { .. })))
    });
    let view = open(cx, &s);
    let last = lines(cx, &view).len() - 1;
    let (retry, window, list, tail, cap) = cx
        .update_window(s.win, |_, w, _| {
            (
                w.try_find("team-retry").map(|e| e.bounds()),
                w.viewport_size(),
                w.try_find("team-lines").map(|e| e.bounds()),
                w.try_find(("team-line", last)).map(|e| e.bounds()),
                apinata_app::dialog_frame::titled_body_room(w) * team_sync_dialog::OUTPUT_SHARE,
            )
        })
        .unwrap();
    let retry = retry.expect("Retry is shown");
    assert!(
        retry.bottom() <= window.height,
        "{retry:?} outside {window:?}"
    );
    // The list fills its cap exactly and the output runs on below it, so the rest scrolls.
    let (list, tail) = (list.expect("the lines"), tail.expect("the output"));
    assert!(
        (list.size.height - cap).abs() <= px(1.),
        "{list:?} against a cap of {cap:?}"
    );
    assert!(tail.bottom() > list.bottom(), "{tail:?} fits in {list:?}");
}

// The first problem line as the open dialog last drew it, not as the workspace holds it now.
fn drawn_headline(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.try_find(("team-line", 0usize))
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[cfg(unix)]
#[gpui_kit::test]
fn the_open_dialog_follows_the_sync_state_until_in_sync(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    let (sb, col) = (Sandbox::new(), collection());
    let bare = shared_origin(&sb, col.path());
    let hook = col.path().join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho nope >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    wait_for(cx, "the hook block", |cx| {
        matches!(state(cx, &s), Some(TeamState::Blocked(Block::Hook { .. })))
    });
    open(cx, &s);
    assert_eq!(
        drawn_headline(cx, &s).as_deref(),
        Some("A commit hook refused the commit:")
    );
    std::fs::remove_file(&hook).unwrap();
    click(cx, &s, "team-retry");
    wait_for(cx, "the push", |_| origin_has(&sb, &bare, "b.yml", "x/b"));
    wait_for(cx, "idle", |cx| state(cx, &s) == Some(TeamState::Idle));
    assert_eq!(drawn_headline(cx, &s).as_deref(), Some("In sync."));
}

const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n  - name: host_key\n    secret: true\n";

fn secrets_collection() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        (
            "r.yml",
            &request("r", "http://x/{{api_token}}/{{host_key}}"),
        ),
        ("environments/dev.yml", DEV),
    ])
}

fn rename_lines(cx: &mut TestAppContext, view: &Entity<SyncView>) -> Vec<String> {
    view.read_with(cx, |v, cx| v.rename_lines(cx))
}

fn open_rows(cx: &mut TestAppContext, s: &Started) -> usize {
    s.ws.read_with(cx, |w, _| {
        w.team_renames(&s.root).map_or(0, |r| r.open_rows().len())
    })
}

fn press(cx: &mut TestAppContext, s: &Started, id: impl Into<gpui_kit::ElementId>) {
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, s.win);
}

/// The URL of `r.yml` resolved in `env` with `store`, as a send would build it.
fn resolved(
    col: &std::path::Path,
    env: &str,
    store: std::sync::Arc<apinata_core::MemoryStore>,
) -> String {
    use apinata_core::{Collection, ContextOptions, VarContext, resolve};
    let c = Collection::open(col).unwrap();
    let ctx = VarContext::new(&c, ContextOptions::new().env(env).secrets(store)).unwrap();
    let req = c
        .tree()
        .requests()
        .iter()
        .find(|r| r.path().ends_with("r.yml"))
        .unwrap();
    resolve(req, &ctx).unwrap().url().to_owned()
}

/// mate renames `dev` to `staging` with `staging` as its file text, pushes, and the app pulls.
fn mate_renames(cx: &mut TestAppContext, sb: &Sandbox, s: &Started, staging: &str) {
    let mate = sb.path().join("mate");
    std::fs::remove_file(mate.join("environments/dev.yml")).unwrap();
    write(&mate, "environments/staging.yml", staging);
    sb.commit_all(&mate, "rename");
    sb.git(&mate, &["push", "-q"]);
    s.ws.read_with(cx, |w, _| w.team_sync(&s.root))
        .unwrap()
        .remote_moved(None);
}

fn seeded(
    cx: &mut TestAppContext,
    sb: &Sandbox,
    col: &tempfile::TempDir,
) -> (Started, std::sync::Arc<apinata_core::MemoryStore>) {
    shared_origin(sb, col.path());
    http_ready(cx);
    let (s, secrets) = team_started(cx, sb, col, |_, _| {});
    let c = apinata_core::Collection::open(col.path()).unwrap();
    secrets.set(&c.secret_key("dev", "token"), TOKEN).unwrap();
    secrets
        .set(&c.secret_key("dev", "host_key"), HOST_KEY)
        .unwrap();
    start_team_as(cx, sb, &s, Role::Write, FAST);
    (s, secrets)
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_pulled_rename_offers_its_secrets_and_moving_them_makes_requests_resolve(
    cx: &mut TestAppContext,
) {
    let (sb, col) = (Sandbox::new(), secrets_collection());
    let (s, secrets) = seeded(cx, &sb, &col);
    let staging = DEV
        .replace("dev", "staging")
        .replace("name: token", "name: api_token");
    mate_renames(cx, &sb, &s, &staging);
    wait_for(cx, "the offer", |cx| open_rows(cx, &s) == 2);
    let view = open(cx, &s);
    assert_eq!(
        rename_lines(cx, &view),
        [
            "dev / host_key → staging / host_key",
            "dev / token → staging / api_token (renamed? check before moving)"
        ]
    );
    assert!(
        !shown(cx, s.win, "team-rename-move-all"),
        "only one row is sure"
    );
    press(cx, &s, ("team-rename-move", 0usize));
    wait_for(cx, "the first move", |cx| open_rows(cx, &s) == 1);
    press(cx, &s, ("team-rename-move", 0usize));
    wait_for(cx, "the second move", |cx| open_rows(cx, &s) == 0);
    assert_eq!(
        resolved(col.path(), "staging", secrets.clone()),
        format!("http://x/{TOKEN}/{HOST_KEY}")
    );
    let sync = s.ws.read_with(cx, |w, _| w.team_sync(&s.root)).unwrap();
    wait_for(cx, "the range to clear", |_| {
        sync.pending_renames().is_none()
    });
}

#[cfg(unix)]
#[gpui_kit::test]
fn an_environment_rename_alone_moves_all_with_one_click(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), secrets_collection());
    let (s, secrets) = seeded(cx, &sb, &col);
    mate_renames(cx, &sb, &s, &DEV.replace("dev", "staging"));
    wait_for(cx, "the offer", |cx| open_rows(cx, &s) == 2);
    let _view = open(cx, &s);
    assert!(shown(cx, s.win, "team-rename-move-all"));
    press(cx, &s, "team-rename-move-all");
    wait_for(cx, "both moved", |cx| open_rows(cx, &s) == 0);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    assert_eq!(
        secrets
            .get(&c.secret_key("staging", "token"))
            .unwrap()
            .as_deref(),
        Some(TOKEN)
    );
    assert_eq!(secrets.get(&c.secret_key("dev", "host_key")).unwrap(), None);
}

// A collection whose first load has not arrived has no Environments tab to open, so typing a
// value again cannot start there and the row must stay open rather than hide and hold the range.
#[cfg(unix)]
#[gpui_kit::test]
fn reentering_where_no_environments_tab_opens_keeps_the_row_open(cx: &mut TestAppContext) {
    use apinata_app::team_renames::RenameRows;
    use apinata_core::team::{Identity, TeamAuth, TeamConfig};
    use apinata_core::{PullRange, SecretName, SecretRename, SecretRenames, TargetState};
    let (sb, col) = (Sandbox::new(), collection());
    let s = app(cx, &sb, &col);
    let unloaded = write_tree(&[("opencollection.yml", ROOT)]);
    let root = unloaded.path().to_path_buf();
    let config = TeamConfig::new(
        root.clone(),
        sb.path().join("unloaded-copies"),
        TeamAuth::new("http://127.0.0.1:9", "plk_test"),
        Identity::new("Ada", "ada@example.com"),
        Role::Write,
    )
    .base_options(sb.options());
    let (from, to) = (
        SecretName::new("dev", "token"),
        SecretName::new("staging", "token"),
    );
    let found = SecretRenames::new(
        vec![SecretRename::new(
            from.clone(),
            vec![(to.clone(), TargetState::Free)],
            true,
        )],
        0,
    );
    // All in one update, so the collection's first snapshot cannot land in between.
    let (loaded, open) = s.ws.update(cx, |w, cx| {
        w.open_path(root.clone(), cx);
        w.start_team(&root, config, None, cx);
        w.set_team_renames(&root, RenameRows::new(PullRange::new("a", "b"), found));
        w.reenter_secret(&root, from, to, cx);
        let open = w.team_renames(&root).map(|r| r.open_rows().len());
        (w.snapshot(&root, cx).is_some(), open)
    });
    assert!(!loaded);
    assert_eq!(open, Some(1));
}

/// A store that refuses every `set`, or every `list`, while its flag is on.
#[derive(Default)]
struct Faulty {
    inner: apinata_core::MemoryStore,
    no_set: std::sync::atomic::AtomicBool,
    no_list: std::sync::atomic::AtomicBool,
}

impl apinata_core::SecretStore for Faulty {
    fn get(
        &self,
        key: &apinata_core::SecretKey,
    ) -> Result<Option<String>, apinata_core::SecretError> {
        self.inner.get(key)
    }
    fn set(
        &self,
        key: &apinata_core::SecretKey,
        value: &str,
    ) -> Result<(), apinata_core::SecretError> {
        if self.no_set.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(apinata_core::SecretError::Unavailable {
                message: "the store is locked".into(),
            });
        }
        self.inner.set(key, value)
    }
    fn delete(&self, key: &apinata_core::SecretKey) -> Result<(), apinata_core::SecretError> {
        self.inner.delete(key)
    }
    fn list(
        &self,
        root: &std::path::Path,
    ) -> Result<Vec<apinata_core::SecretKey>, apinata_core::SecretError> {
        if self.no_list.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(apinata_core::SecretError::Unavailable {
                message: "the store cannot list".into(),
            });
        }
        self.inner.list(root)
    }
}

/// `seeded`, on a `Faulty` store.
fn seeded_faulty(
    cx: &mut TestAppContext,
    sb: &Sandbox,
    col: &tempfile::TempDir,
) -> (Started, std::sync::Arc<Faulty>) {
    shared_origin(sb, col.path());
    http_ready(cx);
    let store = std::sync::Arc::new(Faulty::default());
    let s = team_started_on(cx, sb, col, |_| {}, store.clone());
    let c = apinata_core::Collection::open(col.path()).unwrap();
    store.set(&c.secret_key("dev", "token"), TOKEN).unwrap();
    store
        .set(&c.secret_key("dev", "host_key"), HOST_KEY)
        .unwrap();
    start_team_as(cx, sb, &s, Role::Write, FAST);
    (s, store)
}

/// Every accessible label the window drew, which is all the dialog's text and buttons.
fn drawn_labels(cx: &mut TestAppContext, s: &Started) -> Vec<String> {
    use gpui_kit::test::TestQueryExt as _;
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.elements()
            .iter()
            .filter_map(|e| e.label().map(str::to_owned))
            .collect()
    })
    .unwrap()
}

fn label_of(
    cx: &mut TestAppContext,
    s: &Started,
    id: impl Into<gpui_kit::ElementId>,
) -> Option<String> {
    frame(cx, s.win);
    let id = id.into();
    cx.update_window(s.win, |_, w, _| {
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn assert_no_values(labels: &[String]) {
    for value in [TOKEN, HOST_KEY, "0ther-s3cret"] {
        assert!(
            labels.iter().all(|l| !l.contains(value)),
            "{value} shown in {labels:?}"
        );
    }
}

fn stored(
    store: &dyn apinata_core::SecretStore,
    col: &std::path::Path,
    env: &str,
    var: &str,
) -> Option<String> {
    let c = apinata_core::Collection::open(col).unwrap();
    store.get(&c.secret_key(env, var)).unwrap()
}

const QA: &str = "name: qa\nvariables:\n  - name: token\n    secret: true\n  - name: host_key\n    secret: true\n  - name: region\n    value: eu\n";

#[cfg(unix)]
#[gpui_kit::test]
fn an_ambiguous_row_offers_one_move_per_open_target_and_moves_to_the_pick(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), secrets_collection());
    let (s, secrets) = seeded(cx, &sb, &col);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    secrets
        .set(&c.secret_key("staging", "token"), "0ther-s3cret")
        .unwrap();
    // dev goes and two environments arrive, so each old name has two places it may have gone.
    let mate = sb.path().join("mate");
    write(&mate, "environments/qa.yml", QA);
    mate_renames(cx, &sb, &s, &DEV.replace("dev", "staging"));
    wait_for(cx, "the offer", |cx| open_rows(cx, &s) == 2);
    let rows = s.ws.read_with(cx, |w, _| w.team_renames(&s.root)).unwrap();
    let token = rows.open_rows()[1].clone();
    assert_eq!(token.from, apinata_core::SecretName::new("dev", "token"));
    let at = |env: &str| {
        token
            .to
            .iter()
            .position(|(to, _)| to.environment == env)
            .unwrap()
    };
    let (qa, staging) = (at("qa"), at("staging"));
    let _view = open(cx, &s);
    assert_eq!(
        label_of(
            cx,
            &s,
            gpui_kit::SharedString::from(format!("team-rename-pick-1-{qa}"))
        )
        .as_deref(),
        Some("Move to qa / token")
    );
    assert!(!shown(
        cx,
        s.win,
        gpui_kit::SharedString::from(format!("team-rename-pick-1-{staging}"))
    ));
    assert!(!shown(cx, s.win, ("team-rename-move", 1usize)));
    assert_no_values(&drawn_labels(cx, &s));
    press(
        cx,
        &s,
        gpui_kit::SharedString::from(format!("team-rename-pick-1-{qa}")),
    );
    wait_for(cx, "the pick to move", |cx| open_rows(cx, &s) == 1);
    assert_eq!(stored(&*secrets, col.path(), "dev", "token"), None);
    assert_eq!(
        stored(&*secrets, col.path(), "qa", "token").as_deref(),
        Some(TOKEN)
    );
    assert_eq!(
        stored(&*secrets, col.path(), "staging", "token").as_deref(),
        Some("0ther-s3cret")
    );
}

#[cfg(unix)]
#[gpui_kit::test]
fn dismissing_a_taken_row_hides_it_and_keeps_the_old_value(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), secrets_collection());
    let (s, secrets) = seeded(cx, &sb, &col);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    secrets
        .set(&c.secret_key("staging", "token"), "0ther-s3cret")
        .unwrap();
    mate_renames(cx, &sb, &s, &DEV.replace("dev", "staging"));
    wait_for(cx, "the offer", |cx| open_rows(cx, &s) == 2);
    let _view = open(cx, &s);
    assert!(!shown(cx, s.win, ("team-rename-move", 1usize)));
    assert_no_values(&drawn_labels(cx, &s));
    press(cx, &s, ("team-rename-dismiss", 1usize));
    wait_for(cx, "the row to go", |cx| open_rows(cx, &s) == 1);
    assert_eq!(
        rename_lines(cx, &_view),
        ["dev / host_key → staging / host_key"]
    );
    assert_eq!(
        stored(&*secrets, col.path(), "dev", "token").as_deref(),
        Some(TOKEN)
    );
    assert_eq!(
        stored(&*secrets, col.path(), "staging", "token").as_deref(),
        Some("0ther-s3cret")
    );
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_move_the_store_refuses_says_why_under_its_row(cx: &mut TestAppContext) {
    use std::sync::atomic::Ordering;
    let (sb, col) = (Sandbox::new(), secrets_collection());
    let (s, store) = seeded_faulty(cx, &sb, &col);
    mate_renames(cx, &sb, &s, &DEV.replace("dev", "staging"));
    wait_for(cx, "the offer", |cx| open_rows(cx, &s) == 2);
    let _view = open(cx, &s);
    store.no_set.store(true, Ordering::SeqCst);
    press(cx, &s, ("team-rename-move", 0usize));
    let from = apinata_core::SecretName::new("dev", "host_key");
    wait_for(cx, "the error", |cx| {
        s.ws.read_with(cx, |w, _| {
            w.team_renames(&s.root)
                .is_some_and(|r| r.error(&from).is_some())
        })
    });
    let error = s.ws.read_with(cx, |w, _| {
        w.team_renames(&s.root)
            .unwrap()
            .error(&from)
            .unwrap()
            .to_owned()
    });
    assert_eq!(
        label_of(cx, &s, ("team-rename-note", 0usize)),
        Some(format!("Could not move it: {error}"))
    );
    assert!(error.contains("the store is locked"), "{error}");
    assert_eq!(open_rows(cx, &s), 2);
    assert_no_values(&drawn_labels(cx, &s));
    assert_eq!(
        stored(&*store, col.path(), "dev", "host_key").as_deref(),
        Some(HOST_KEY)
    );
    assert_eq!(stored(&*store, col.path(), "staging", "host_key"), None);
}

#[cfg(unix)]
#[gpui_kit::test]
fn an_ask_the_store_cannot_answer_says_so_in_the_dialog(cx: &mut TestAppContext) {
    use std::sync::atomic::Ordering;
    let (sb, col) = (Sandbox::new(), secrets_collection());
    let (s, store) = seeded_faulty(cx, &sb, &col);
    store.no_list.store(true, Ordering::SeqCst);
    mate_renames(cx, &sb, &s, &DEV.replace("dev", "staging"));
    wait_for(cx, "the failed ask", |cx| {
        s.ws.read_with(cx, |w, _| {
            w.team_renames(&s.root)
                .is_some_and(|r| r.failed().is_some())
        })
    });
    let why = s.ws.read_with(cx, |w, _| {
        w.team_renames(&s.root)
            .unwrap()
            .failed()
            .unwrap()
            .to_owned()
    });
    assert!(why.contains("the store cannot list"), "{why}");
    let _view = open(cx, &s);
    assert_eq!(
        label_of(cx, &s, "team-rename-failed"),
        Some(format!("Could not check for secrets to move: {why}"))
    );
    assert_no_values(&drawn_labels(cx, &s));
}

/// The Environments tab of `s`, if one is open.
fn env_tab(cx: &mut TestAppContext, s: &Started) -> Option<apinata_app::tabs::TabId> {
    use apinata_app::tabs::TabKind;
    s.ws.read_with(cx, |w, _| {
        (w.tabs().entries().iter())
            .find(|e| e.root == s.root && e.kind == TabKind::Environments)
            .map(|e| e.id)
    })
}

/// A pulled environment rename with "Enter again" pressed on the `host_key` row, which hides.
fn reentering(
    cx: &mut TestAppContext,
    sb: &Sandbox,
    col: &tempfile::TempDir,
) -> (Started, std::sync::Arc<apinata_core::MemoryStore>) {
    let (s, secrets) = seeded(cx, sb, col);
    mate_renames(cx, sb, &s, &DEV.replace("dev", "staging"));
    wait_for(cx, "the offer", |cx| open_rows(cx, &s) == 2);
    let _view = open(cx, &s);
    press(cx, &s, ("team-rename-enter", 0usize));
    assert!(env_tab(cx, &s).is_some(), "Enter again opens the tab");
    assert_eq!(open_rows(cx, &s), 1);
    (s, secrets)
}

#[cfg(unix)]
#[gpui_kit::test]
fn closing_the_tab_without_a_value_brings_the_row_back(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), secrets_collection());
    let (s, secrets) = reentering(cx, &sb, &col);
    let tab = env_tab(cx, &s).unwrap();
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.close(tab)));
    wait_for(cx, "the row to return", |cx| open_rows(cx, &s) == 2);
    assert_eq!(
        stored(&*secrets, col.path(), "dev", "host_key").as_deref(),
        Some(HOST_KEY)
    );
}

#[cfg(unix)]
#[gpui_kit::test]
fn saving_a_value_finishes_the_row_and_removes_the_old_entry(cx: &mut TestAppContext) {
    use apinata_app::command::{Command, EnvOp};
    let (sb, col) = (Sandbox::new(), secrets_collection());
    let (s, secrets) = reentering(cx, &sb, &col);
    let op = EnvOp::SetSecretValue {
        env: "staging".into(),
        var: "host_key".into(),
        value: "k2".into(),
    };
    // The pull's new environment file reaches the actor through the watcher, after the ask.
    wait_for(cx, "staging to load", |cx| {
        s.ws.read_with(cx, |w, cx| {
            w.environments(&s.root, cx).contains(&"staging".into())
        })
    });
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::Env(op), cx));
    wait_for(cx, "the old entry to go", |_| {
        stored(&*secrets, col.path(), "dev", "host_key").is_none()
    });
    let tab = env_tab(cx, &s).unwrap();
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.close(tab)));
    wait_for(cx, "the row to finish", |cx| {
        s.ws.read_with(cx, |w, _| {
            w.team_renames(&s.root)
                .is_none_or(|r| r.reentering().is_empty())
        })
    });
    assert_eq!(open_rows(cx, &s), 1);
    assert_eq!(
        stored(&*secrets, col.path(), "staging", "host_key").as_deref(),
        Some("k2")
    );
}

#[cfg(unix)]
#[gpui_kit::test]
fn picking_a_candidate_holding_the_same_value_deletes_the_old_entry_without_writing(
    cx: &mut TestAppContext,
) {
    use std::sync::atomic::Ordering;
    let (sb, col) = (Sandbox::new(), secrets_collection());
    let (s, store) = seeded_faulty(cx, &sb, &col);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    store.set(&c.secret_key("staging", "token"), TOKEN).unwrap();
    let mate = sb.path().join("mate");
    write(&mate, "environments/qa.yml", QA);
    mate_renames(cx, &sb, &s, &DEV.replace("dev", "staging"));
    wait_for(cx, "the offer", |cx| open_rows(cx, &s) == 2);
    let rows = s.ws.read_with(cx, |w, _| w.team_renames(&s.root)).unwrap();
    let token = rows.open_rows()[1].clone();
    let staging = (token.to.iter())
        .position(|(to, state)| {
            to.environment == "staging" && *state == apinata_core::TargetState::Same
        })
        .expect("staging holds the same value");
    let _view = open(cx, &s);
    // Any write now fails, so the move can only succeed by not writing.
    store.no_set.store(true, Ordering::SeqCst);
    press(
        cx,
        &s,
        gpui_kit::SharedString::from(format!("team-rename-pick-1-{staging}")),
    );
    wait_for(cx, "the pick to move", |cx| open_rows(cx, &s) == 1);
    assert_eq!(stored(&*store, col.path(), "dev", "token"), None);
    assert_eq!(
        stored(&*store, col.path(), "staging", "token").as_deref(),
        Some(TOKEN)
    );
    assert_eq!(stored(&*store, col.path(), "qa", "token"), None);
}
