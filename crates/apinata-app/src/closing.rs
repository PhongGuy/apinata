//! What counts as unsaved work, and the prompt that guards a window close or a collection
//! close: save everything in scope, discard it, or cancel.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use apinata_core::ItemPath;
use gpui_kit::{App, AsyncWindowContext, Entity, Global, WeakEntity, Window, WindowId};

use crate::command::{Command, NoticeKind, Target};
use crate::runner_model::RunnerModel;
use crate::shell::Shell;
use crate::snapshot::Content;
use crate::tab_area::TabArea;
use crate::workspace::{Workspace, WorkspaceEvent};
use crate::{confirm, dialog_guard, draft_store, save_draft};

/// The unsaved work of one open collection.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Unsaved {
    /// The collection's root.
    pub root: PathBuf,
    /// Its dirty documents, in `dirty_targets` order.
    pub targets: Vec<Target>,
    /// Commands sent to it and not yet applied.
    pub pending: u64,
}

impl Unsaved {
    /// The unsaved state of the collection at `root`.
    pub fn new(root: PathBuf, targets: Vec<Target>, pending: u64) -> Self {
        Self {
            root,
            targets,
            pending,
        }
    }
}

/// What a close removes.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum CloseScope {
    /// The whole window.
    Window,
    /// One collection, by root.
    Collection(PathBuf),
}

/// The dirty documents of `content`: requests, then folders (each sorted by path), then the
/// collection, then environments in order. A dirty draft counts as a request; an untouched
/// one is clean, so a close discards it silently.
pub fn dirty_targets(content: &Content) -> Vec<Target> {
    let mut requests: Vec<_> = content.requests.iter().filter(|(_, v)| v.dirty).collect();
    requests.sort_by(|a, b| a.0.cmp(b.0));
    let mut folders: Vec<_> = content.folders.iter().filter(|(_, v)| v.dirty).collect();
    folders.sort_by(|a, b| a.0.cmp(b.0));
    let requests = requests
        .into_iter()
        .map(|(p, _)| Target::Request(p.clone()));
    let folders = folders.into_iter().map(|(p, _)| Target::Folder(p.clone()));
    let collection = content.collection.dirty.then_some(Target::Collection);
    let environments = content.environments.iter().filter(|e| e.dirty);
    requests
        .chain(folders)
        .chain(collection)
        .chain(environments.map(|e| Target::Environment(e.name.clone())))
        .collect()
}

/// Routes the window's close button through `confirm_close` for the whole window: a clean
/// window closes, otherwise the prompt opens and the platform close is refused.
pub fn install(
    workspace: Entity<Workspace>,
    tab_area: Entity<TabArea>,
    window: &mut Window,
    cx: &mut App,
) {
    let id = window.window_handle().window_id();
    cx.default_global::<TabAreas>()
        .0
        .insert(id, tab_area.downgrade());
    forget_on_close(cx);
    window.on_window_should_close(cx, move |window, cx| {
        // A close asked for while a tab close settles or asks waits for that to end.
        let id = window.window_handle().window_id();
        if cx.default_global::<Holding>().0.contains(&id) {
            cx.default_global::<Queued>()
                .0
                .insert(id, workspace.clone());
            return false;
        }
        request_window_close(&workspace, window, cx)
    });
}

// The window's close button: true when the platform may close now, false when a prompt or a
// run's stop flow took over.
fn request_window_close(workspace: &Entity<Workspace>, window: &mut Window, cx: &mut App) -> bool {
    // Drafts written now are kept for after restart, so they need no prompt.
    draft_store::flush_now(None, cx);
    if prompting(window, cx) {
        return false;
    }
    if let Some(guard) = dialog_guard::top(window, cx)
        && guard.pending(cx)
    {
        ask_for_dialog(guard, workspace, window, cx);
        return false;
    }
    if actor_behind(workspace, None, cx) || holding(&CloseScope::Window, window, cx).is_some() {
        confirm_close(workspace.clone(), CloseScope::Window, window, cx);
        return false;
    }
    if unsaved_in(workspace, &CloseScope::Window, window, cx).is_empty() {
        if runs_in_flight(window, cx).is_empty() {
            return true;
        }
        proceed(workspace, &CloseScope::Window, window, cx, &Kept::default());
        return false;
    }
    confirm_close(workspace.clone(), CloseScope::Window, window, cx);
    false
}

// A dialog with unsaved edits answers first, holding the window close queued: Save or Don't
// save closes the dialog and lets the close run on, Cancel or a failed save drops it. A close
// asked while its prompt is up waits on that answer; one asked while another dialog sits over
// it is refused, as while a close prompt shows.
fn ask_for_dialog(
    guard: dialog_guard::GuardRef,
    workspace: &Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    let busy = guard.busy();
    if !busy && !guard.on_top(window, cx) {
        return;
    }
    let id = window.window_handle().window_id();
    hold(window, cx);
    cx.default_global::<Queued>()
        .0
        .insert(id, workspace.clone());
    let kept: dialog_guard::Then = Box::new(move |window, cx| {
        cx.default_global::<Queued>().0.remove(&id);
        unhold(window, cx);
    });
    if busy {
        guard.after_answer(window, cx, Box::new(unhold), kept);
    } else {
        guard.ask_to_close(window, cx, Box::new(unhold), kept);
    }
}

// Windows whose close is settling or whose tab prompt is open, and the window closes asked
// for meanwhile.
#[derive(Default)]
struct Holding(HashSet<WindowId>);

impl Global for Holding {}

#[derive(Default)]
struct Queued(std::collections::HashMap<WindowId, Entity<Workspace>>);

impl Global for Queued {}

/// Marks the window's tab close as under way: a window close asked for now is queued.
pub(crate) fn hold(window: &Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    cx.default_global::<Holding>().0.insert(id);
}

/// Ends `hold`, then runs a window close that was asked for in the meantime.
pub(crate) fn unhold(window: &mut Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    cx.default_global::<Holding>().0.remove(&id);
    // Deferred, since the tab area calls this from its own update and the close reads it.
    window.defer(cx, release_queued);
}

/// Whether a window close asked for during a hold is still waiting to run in window `id`.
pub fn close_queued(id: WindowId, cx: &App) -> bool {
    cx.try_global::<Queued>()
        .is_some_and(|q| q.0.contains_key(&id))
}

// A window gone by any route ends every close flow it had, so its queued close, which holds the
// workspace, and its marks go with it.
fn forget_on_close(cx: &mut App) {
    if cx.has_global::<Forgetting>() {
        return;
    }
    cx.set_global(Forgetting);
    cx.on_window_closed(|cx, id| {
        cx.default_global::<Queued>().0.remove(&id);
        cx.default_global::<Holding>().0.remove(&id);
        cx.default_global::<Prompting>().0.remove(&id);
        cx.default_global::<TabAreas>().0.remove(&id);
    })
    .detach();
}

struct Forgetting;

impl Global for Forgetting {}

// Runs the window close asked for during a hold, once no prompt is showing. A prompt still
// open leaves it queued for that prompt's end.
fn release_queued(window: &mut Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    if prompting(window, cx) || cx.default_global::<Holding>().0.contains(&id) {
        return;
    }
    let Some(workspace) = cx.default_global::<Queued>().0.remove(&id) else {
        return;
    };
    if request_window_close(&workspace, window, cx) {
        window.remove_window();
    }
}

// A `kept` callback shared by every step of one close flow, called at most once.
type Kept = Rc<RefCell<Option<Box<dyn FnOnce(&mut App)>>>>;

fn call_kept(kept: &Kept, cx: &mut App) {
    if let Some(f) = kept.borrow_mut().take() {
        f(cx);
    }
}

/// How long a close waits for an actor to apply the edits typed just before it.
pub const SETTLE: std::time::Duration = std::time::Duration::from_millis(750);

// Whether any collection in scope has a command the actor has not yet applied.
fn actor_behind(workspace: &Entity<Workspace>, only: Option<&std::path::Path>, cx: &App) -> bool {
    (workspace.read(cx).unsaved(only, cx).iter()).any(|u| u.pending > 0)
}

/// Whether any of `roots` has a command its actor has not yet applied.
pub fn roots_behind(workspace: &Entity<Workspace>, roots: &[PathBuf], cx: &App) -> bool {
    (workspace.read(cx).unsaved(None, cx).iter()).any(|u| u.pending > 0 && roots.contains(&u.root))
}

/// Holds a tab close while the actors of `roots` catch up, up to `SETTLE`, then calls `then`
/// on what they published. Window and collection closes settle the same way.
pub fn settle_roots(
    workspace: Entity<Workspace>,
    roots: Vec<PathBuf>,
    window: &mut Window,
    cx: &mut App,
    then: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    settle(workspace, Reach::Roots(roots), window, cx, then);
}

// Which collections a settle watches.
enum Reach {
    Scope(CloseScope),
    Roots(Vec<PathBuf>),
}

impl Reach {
    fn behind(&self, workspace: &Entity<Workspace>, cx: &App) -> bool {
        match self {
            Reach::Scope(scope) => actor_behind(workspace, only(scope), cx),
            Reach::Roots(roots) => roots_behind(workspace, roots, cx),
        }
    }
}

// Holds a close while the actor catches up, then runs the close flow once on what it
// published. A slow actor is not waited for past `SETTLE`; the prompt then covers the rest.
fn settle_then(
    workspace: Entity<Workspace>,
    scope: CloseScope,
    window: &mut Window,
    cx: &mut App,
    then: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    settle(workspace, Reach::Scope(scope), window, cx, then);
}

fn settle(
    workspace: Entity<Workspace>,
    reach: Reach,
    window: &mut Window,
    cx: &mut App,
    then: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    let id = window.window_handle().window_id();
    cx.default_global::<Prompting>().0.insert(id);
    // A window close settling already owns the wait; only the other closes hold the window.
    if !matches!(reach, Reach::Scope(CloseScope::Window)) {
        cx.default_global::<Holding>().0.insert(id);
    }
    let (tx, rx) = async_channel::unbounded::<bool>();
    let changed = tx.clone();
    let observer = cx.observe(&workspace, move |_, _| {
        changed.try_send(false).ok();
    });
    let timer = cx.background_executor().timer(SETTLE);
    let deadline = window.spawn(cx, async move |_| {
        timer.await;
        tx.try_send(true).ok();
    });
    window
        .spawn(cx, async move |cx| {
            let _hold = (observer, deadline);
            loop {
                let behind = cx.update(|_, cx| reach.behind(&workspace, cx));
                if !matches!(behind, Ok(true)) {
                    break;
                }
                // A deadline or a closed channel ends the wait; any other wake looks again.
                if !matches!(rx.recv().await, Ok(false)) {
                    break;
                }
            }
            cx.update(|window, cx| {
                cx.default_global::<Prompting>().0.remove(&id);
                cx.default_global::<Holding>().0.remove(&id);
                then(window, cx);
                release_queued(window, cx);
            })
            .ok();
        })
        .detach();
}

/// Closes `scope` at once when nothing in it is unsaved; otherwise asks to save all, discard
/// or cancel. Does nothing while this window already shows a close prompt.
pub fn confirm_close(
    workspace: Entity<Workspace>,
    scope: CloseScope,
    window: &mut Window,
    cx: &mut App,
) {
    confirm_close_or_keep(workspace, scope, window, cx, |_| {});
}

/// As `confirm_close`, and calls `kept` when the flow ends with the scope still open: Cancel,
/// a dismissed prompt, a failed or incomplete save, or a prompt already showing.
pub fn confirm_close_or_keep(
    workspace: Entity<Workspace>,
    scope: CloseScope,
    window: &mut Window,
    cx: &mut App,
    kept: impl FnOnce(&mut App) + 'static,
) {
    if !prompting(window, cx) && actor_behind(&workspace, only(&scope), cx) {
        let (w, s) = (workspace.clone(), scope.clone());
        settle_then(workspace, scope, window, cx, move |window, cx| {
            ask_to_close(w, s, window, cx, kept)
        });
        return;
    }
    ask_to_close(workspace, scope, window, cx, kept);
}

// The close flow proper, on the published state after the actor has had its say.
fn ask_to_close(
    workspace: Entity<Workspace>,
    scope: CloseScope,
    window: &mut Window,
    cx: &mut App,
    kept: impl FnOnce(&mut App) + 'static,
) {
    let kept: Kept = Rc::new(RefCell::new(Some(Box::new(kept))));
    draft_store::flush_now(only(&scope), cx);
    if prompting(window, cx) {
        call_kept(&kept, cx);
        return;
    }
    // Held typing is in no snapshot, so it goes through the Save check before anything else.
    if let Some(area) = holding(&scope, window, cx) {
        let id = window.window_handle().window_id();
        cx.default_global::<Prompting>().0.insert(id);
        let root = only(&scope).map(Path::to_path_buf);
        area.update(cx, |a, cx| {
            a.release_held(root.as_deref(), window, cx, move |released, window, cx| {
                cx.default_global::<Prompting>().0.remove(&id);
                if released {
                    // Boxed, since this flow instantiates itself.
                    let kept: Box<dyn FnOnce(&mut App)> = Box::new(move |cx| call_kept(&kept, cx));
                    confirm_close_or_keep(workspace, scope, window, cx, kept);
                } else {
                    call_kept(&kept, cx);
                }
                release_queued(window, cx);
            })
        });
        return;
    }
    let unsaved = unsaved_in(&workspace, &scope, window, cx);
    if unsaved.is_empty() {
        proceed(&workspace, &scope, window, cx, &kept);
        return;
    }
    let listed = listed_drafts(&workspace, &unsaved, cx);
    let id = window.window_handle().window_id();
    cx.default_global::<Prompting>().0.insert(id);
    let answer = confirm::ask(
        confirm::save_ask(
            "Save changes before closing?".into(),
            Some(unsaved_detail(workspace.read(cx), &unsaved, &scope).into()),
            "Discard",
            "Save all",
        ),
        window,
        cx,
    );
    window
        .spawn(cx, async move |cx| {
            // A dialog dismissed unanswered errs, which keeps the scope like Cancel.
            let answer = answer.recv().await;
            // Save all asks about stored values first, still prompting, as the tab area's does.
            let answer = match answer {
                Ok(2) if !check_before_save(&scope, cx).await => Err(()),
                Ok(n) => Ok(n),
                Err(_) => Err(()),
            };
            cx.update(|_, cx| cx.default_global::<Prompting>().0.remove(&id))
                .ok();
            match answer {
                Ok(2) => {
                    save_all(workspace.clone(), scope.clone(), Rc::clone(&kept), cx).await;
                    // A closed window cannot update, and a closed collection is no longer
                    // listed, so only a scope still open reports itself as kept.
                    // A run prompt now open decides for itself.
                    cx.update(|window, cx| {
                        if still_open(&workspace, &scope, cx) && !prompting(window, cx) {
                            call_kept(&kept, cx)
                        }
                    })
                    .ok();
                }
                Ok(1) => {
                    // A listed draft's file, even one written while the prompt was open, would
                    // otherwise bring it back.
                    cx.update(|window, cx| {
                        draft_store::discard_listed(&listed, cx);
                        proceed(&workspace, &scope, window, cx, &kept)
                    })
                    .ok();
                }
                _ => {
                    cx.update(|_, cx| call_kept(&kept, cx)).ok();
                }
            }
            cx.update(release_queued).ok();
        })
        .detach();
}

// The windows showing a close prompt, so a second close request opens no second prompt.
#[derive(Default)]
struct Prompting(HashSet<WindowId>);

impl Global for Prompting {}

/// Whether any close prompt, a tab's included, is open or settling in this window.
pub(crate) fn asking(window: &Window, cx: &App) -> bool {
    let id = window.window_handle().window_id();
    prompting(window, cx)
        || cx
            .try_global::<Holding>()
            .is_some_and(|h| h.0.contains(&id))
}

pub(crate) fn prompting(window: &Window, cx: &App) -> bool {
    let id = window.window_handle().window_id();
    cx.try_global::<Prompting>()
        .is_some_and(|p| p.0.contains(&id))
}

// The scripts typed in the editor of `target`, for the save that is about to be sent.
fn typed_for(window: &Window, root: &Path, target: &Target, cx: &App) -> Vec<String> {
    let shell = (window.root::<gpui_kit::base::Root>().flatten())
        .and_then(|root| root.read(cx).view().clone().downcast::<Shell>().ok());
    shell
        .map(|s| s.read(cx).tab_area().read(cx).typed_for(root, target, cx))
        .unwrap_or_default()
}

// The scripts typed in the draft being saved; approved once its write is confirmed.
fn draft_texts(window: &Window, root: &Path, draft: &ItemPath, cx: &App) -> Vec<String> {
    let shell = (window.root::<gpui_kit::base::Root>().flatten())
        .and_then(|root| root.read(cx).view().clone().downcast::<Shell>().ok());
    shell
        .map(|shell| {
            shell
                .read(cx)
                .tab_area()
                .read(cx)
                .draft_texts(root, draft, cx)
        })
        .unwrap_or_default()
}

// The drafts among `unsaved`, as `(root, item, key)`.
fn listed_drafts(
    workspace: &Entity<Workspace>,
    unsaved: &[Unsaved],
    cx: &App,
) -> Vec<(PathBuf, ItemPath, ItemPath)> {
    let w = workspace.read(cx);
    (unsaved.iter())
        .flat_map(|u| u.targets.iter().map(move |t| (&u.root, t)))
        .filter_map(|(root, target)| match target {
            Target::Request(item) if item.is_draft() => {
                Some((root.clone(), item.clone(), w.draft_key(root, item)?))
            }
            _ => None,
        })
        .collect()
}

fn only(scope: &CloseScope) -> Option<&std::path::Path> {
    match scope {
        CloseScope::Window => None,
        CloseScope::Collection(root) => Some(root),
    }
}

fn still_open(workspace: &Entity<Workspace>, scope: &CloseScope, cx: &App) -> bool {
    match scope {
        CloseScope::Window => true,
        CloseScope::Collection(root) => workspace
            .read(cx)
            .collections()
            .iter()
            .any(|c| c.root() == root),
    }
}

fn proceed(
    workspace: &Entity<Workspace>,
    scope: &CloseScope,
    window: &mut Window,
    cx: &mut App,
    kept: &Kept,
) {
    match scope {
        CloseScope::Window => {
            let runs = runs_in_flight(window, cx);
            if runs.is_empty() {
                window.remove_window();
            } else {
                ask_to_stop_runs(runs, Rc::clone(kept), window, cx);
            }
        }
        CloseScope::Collection(root) => workspace.update(cx, |w, cx| w.close(root, cx)),
    }
}

// The runs still going in this window's runner tabs; a single send does not count. The tab
// area comes from a registry, since a close may start inside an update of the shell.
fn runs_in_flight(window: &Window, cx: &App) -> Vec<Entity<RunnerModel>> {
    tab_area(window, cx).map_or_else(Vec::new, |a| a.read(cx).running_runs(cx))
}

fn tab_area(window: &Window, cx: &App) -> Option<Entity<TabArea>> {
    let id = window.window_handle().window_id();
    cx.try_global::<TabAreas>()?.0.get(&id)?.upgrade()
}

// The tab area when `scope` has variable typing only the table holds.
fn holding(scope: &CloseScope, window: &Window, cx: &App) -> Option<Entity<TabArea>> {
    tab_area(window, cx).filter(|a| a.read(cx).holds_typing_in(only(scope), cx))
}

// The unsaved work in scope, counting an OAuth secret typed but not yet sent as its tab's
// target, so the prompt asks about it while the typing stays out of the store.
fn unsaved_in(
    workspace: &Entity<Workspace>,
    scope: &CloseScope,
    window: &Window,
    cx: &App,
) -> Vec<Unsaved> {
    let mut unsaved = workspace.read(cx).unsaved(only(scope), cx);
    let typed = tab_area(window, cx)
        .map(|a| a.read(cx).typed_secret_targets(only(scope), cx))
        .unwrap_or_default();
    for (root, target) in typed {
        match unsaved.iter_mut().find(|u| u.root == root) {
            Some(u) if u.targets.contains(&target) => {}
            Some(u) => u.targets.push(target),
            None => unsaved.push(Unsaved::new(root, vec![target], 0)),
        }
    }
    unsaved
}

// Each window's tab area, for the run check at close.
#[derive(Default)]
struct TabAreas(std::collections::HashMap<WindowId, WeakEntity<TabArea>>);

impl Global for TabAreas {}

// Last step of a window close: stop the runs and close, or keep the window as it is.
fn ask_to_stop_runs(runs: Vec<Entity<RunnerModel>>, kept: Kept, window: &mut Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    cx.default_global::<Prompting>().0.insert(id);
    let answer = confirm::ask(confirm::stop_run_ask(), window, cx);
    window
        .spawn(cx, async move |cx| {
            let answer = answer.recv().await;
            cx.update(|window, cx| {
                cx.default_global::<Prompting>().0.remove(&id);
                if answer == Ok(1) {
                    for run in &runs {
                        run.update(cx, |m, cx| m.cancel(cx));
                    }
                    window.remove_window();
                } else {
                    call_kept(&kept, cx);
                }
                release_queued(window, cx);
            })
            .ok();
        })
        .detach();
}

/// The close prompt's detail line for `unsaved` when closing `scope`, naming the scratch pad as
/// itself rather than counting it among the collections, and one closing collection as this one.
pub fn unsaved_detail(workspace: &Workspace, unsaved: &[Unsaved], scope: &CloseScope) -> String {
    let docs: usize = unsaved.iter().map(|u| u.targets.len()).sum();
    if let CloseScope::Collection(root) = scope
        && !workspace.is_scratch(root)
    {
        return detail_in(docs, "this collection");
    }
    let scratch = unsaved.iter().any(|u| workspace.is_scratch(&u.root));
    let collections = unsaved
        .iter()
        .filter(|u| !workspace.is_scratch(&u.root))
        .count();
    close_detail(docs, collections, scratch)
}

/// The close prompt's detail line for `documents` unsaved changes across `collections`
/// collections and, when `scratch`, the scratch pad, with nouns agreeing in number; zero
/// documents means only pending writes.
pub fn close_detail(documents: usize, collections: usize, scratch: bool) -> String {
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut places = Vec::new();
    if collections > 0 || !scratch {
        places.push(plural(collections, "collection", "collections"));
    }
    if scratch {
        places.push("the scratch pad".to_owned());
    }
    detail_in(documents, &places.join(" and "))
}

// The detail line for `documents` unsaved changes in `places`.
fn detail_in(documents: usize, places: &str) -> String {
    match documents {
        0 => format!("Changes to {places} are still being applied."),
        1 => format!("1 unsaved change in {places}."),
        n => format!("{n} unsaved changes in {places}."),
    }
}

// The tab area's Save check over the environment tabs in scope, which then stores their secret
// typing; false on a Cancel, a failed read or a window gone meanwhile.
async fn check_before_save(scope: &CloseScope, cx: &mut AsyncWindowContext) -> bool {
    let (tx, rx) = async_channel::bounded::<bool>(1);
    let root = only(scope).map(Path::to_path_buf);
    let started = cx.update(|window, cx| match tab_area(window, cx) {
        Some(area) => area.update(cx, |a, cx| {
            a.check_before_save(root.as_deref(), window, cx, move |go, _, _| {
                tx.try_send(go).ok();
            })
        }),
        None => {
            tx.try_send(true).ok();
        }
    });
    started.is_ok() && rx.recv().await.unwrap_or(false)
}

// Saves every dirty file in scope, then names each dirty draft in its Save dialog over its own
// tab, one at a time; a cancelled dialog keeps everything open. Then closes once nothing in
// scope is unsaved. It gives up, leaving everything open, on a Failed notice for one of its
// saves, or when nothing is pending any more and something is still dirty, as after a save
// conflict.
async fn save_all(
    workspace: Entity<Workspace>,
    scope: CloseScope,
    kept: Kept,
    cx: &mut AsyncWindowContext,
) {
    let (tx, rx) = async_channel::unbounded::<bool>();
    let ids = Rc::new(RefCell::new(HashSet::new()));
    let started = cx.update(|window, cx| {
        // Typed secrets become edits now, each ahead of the save that writes it.
        let unsaved = unsaved_in(&workspace, &scope, window, cx);
        if let Some(area) = tab_area(window, cx) {
            area.update(cx, |a, cx| a.send_pending_secrets(only(&scope), cx));
        }
        let mut drafts = Vec::new();
        for u in unsaved {
            for target in u.targets {
                // A draft has no file to save in place; `Command::Save` would only fail.
                if let Target::Request(item) = &target
                    && item.is_draft()
                {
                    drafts.push((u.root.clone(), item.clone()));
                    continue;
                }
                // Typed text is approved only once this write is confirmed.
                let texts = typed_for(window, &u.root, &target, cx);
                let id = workspace.update(cx, |w, cx| {
                    let id = w.send(&u.root, Command::Save(target.clone()), cx);
                    w.approve_when_saved(&u.root, id, Some(target), texts);
                    id
                });
                ids.borrow_mut().extend(id.map(|id| (u.root.clone(), id)));
            }
        }
        let changed = tx.clone();
        let observer = cx.observe(&workspace, move |_, _| {
            changed.try_send(false).ok();
        });
        let failed = tx.clone();
        let watched = Rc::clone(&ids);
        let notices = cx.subscribe(&workspace, move |_, e: &WorkspaceEvent, _| {
            if let WorkspaceEvent::Notices { root, notices, .. } = e
                && notices.iter().any(|n| {
                    matches!(n.kind, NoticeKind::Failed { .. })
                        && watched.borrow().contains(&(root.clone(), n.command))
                })
            {
                failed.try_send(true).ok();
            }
        });
        ([observer, notices], drafts)
    });
    let Ok((_subscriptions, drafts)) = started else {
        return;
    };
    for (root, draft) in drafts {
        let asked = cx.update(|window, cx| {
            workspace.update(cx, |w, cx| w.open_item(&root, draft.clone(), false, cx));
            save_draft::ask_for(&workspace, &root, draft.clone(), window, cx)
        });
        let Ok(Some(answer)) = asked else {
            return;
        };
        let Ok(Some(save_as)) = answer.recv().await else {
            return;
        };
        let sent = cx.update(|window, cx| {
            let texts = draft_texts(window, &root, &draft, cx);
            let id = save_draft::send_with(&workspace, &root, draft, save_as, texts.clone(), cx);
            workspace.update(cx, |w, _| w.approve_when_saved(&root, id, None, texts));
            id
        });
        ids.borrow_mut()
            .extend(sent.ok().flatten().map(|id| (root.clone(), id)));
    }
    // A stopped actor refuses its saves without a further update, so look once right away.
    tx.try_send(false).ok();
    drop(tx);
    while let Ok(failed) = rx.recv().await {
        if failed {
            return;
        }
        let verdict = cx.update(|window, cx| {
            let unsaved = workspace.read(cx).unsaved(only(&scope), cx);
            if unsaved.iter().any(|u| u.pending > 0) {
                return false;
            }
            if unsaved.is_empty() {
                proceed(&workspace, &scope, window, cx, &kept);
            }
            true
        });
        if !matches!(verdict, Ok(false)) {
            return;
        }
    }
}
