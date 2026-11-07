//! The close guard of a dialog holding unsaved edits: Escape, the x, a backdrop press or a
//! window close first asks Save, Don't save or Cancel, so an edit is never dropped unasked.
//!
//! A dialog makes one [`Guarded`] before it opens, applies it in its builder and marks it
//! opened; the builder owns it, so it lives exactly as long as the dialog does.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::Dialog;
use gpui_kit::{App, FocusHandle, Global, SharedString, Window, WindowId};

use crate::confirm;

/// What a save started from the prompt reports: `Ok` once stored, else why it failed.
pub type SaveOutcome = async_channel::Receiver<Result<(), String>>;

type Waiters = Rc<RefCell<Vec<(Then, Then)>>>;

/// A step run once a prompt is settled.
pub type Then = Box<dyn FnOnce(&mut Window, &mut App)>;
type Save = Box<dyn Fn(&mut Window, &mut App) -> SaveOutcome>;
type Discard = Box<dyn Fn(&mut Window, &mut App)>;

/// How a guarded dialog tells whether it holds edits, names them and saves or drops them.
pub struct Guard {
    dirty: Box<dyn Fn(&App) -> bool>,
    title: Box<dyn Fn(&App) -> SharedString>,
    save: Save,
    discard: Discard,
}

impl Guard {
    /// `dirty` says whether anything would be lost, `title` is the prompt's question, `save`
    /// stores the edits and `discard` drops them before the dialog goes.
    pub fn new(
        dirty: impl Fn(&App) -> bool + 'static,
        title: impl Fn(&App) -> SharedString + 'static,
        save: impl Fn(&mut Window, &mut App) -> SaveOutcome + 'static,
        discard: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Guard {
            dirty: Box::new(dirty),
            title: Box::new(title),
            save: Box::new(save),
            discard: Box::new(discard),
        }
    }
}

/// A save outcome that is already known, for a save that ran synchronously.
pub fn settled(outcome: Result<(), String>) -> SaveOutcome {
    let (tx, rx) = async_channel::bounded(1);
    let _ = tx.try_send(outcome);
    rx
}

struct State {
    guard: Guard,
    // Set from the moment a prompt is asked for until its answer is settled, save included,
    // so a further close neither asks again nor closes.
    busy: Cell<bool>,
    // Set once the guard closes its dialog, so a window close never asks for it again.
    closed: Cell<bool>,
    // The kit dialog's own focus node, focused as it opens: holding focus means on top, and
    // a guard that never saw its dialog open is never on top.
    host: RefCell<Option<FocusHandle>>,
    // Window closes waiting on the prompt under way: (went through, kept). Shared with the
    // tasks that settle them, so they still run if the dialog goes away first.
    waiters: Waiters,
}

impl State {
    fn on_top(&self, window: &Window, cx: &App) -> bool {
        (self.host.borrow().as_ref()).is_some_and(|h| h.contains_focused(window, cx))
    }
}

/// The guard one dialog owns; dropping the dialog's builder drops it.
#[derive(Clone)]
pub struct Guarded(Rc<State>);

/// A handle on a guard that does not keep its dialog's guard alive.
#[derive(Clone)]
pub struct GuardRef(Weak<State>);

// The guards of each window's open dialogs, in opening order.
#[derive(Default)]
struct Guards(HashMap<WindowId, Vec<Weak<State>>>);

impl Global for Guards {}

/// Makes the guard for a dialog about to open over `window`.
pub fn guarded(guard: Guard, window: &Window, cx: &mut App) -> Guarded {
    let state = Rc::new(State {
        guard,
        busy: Cell::new(false),
        closed: Cell::new(false),
        host: RefCell::new(None),
        waiters: Waiters::default(),
    });
    let id = window.window_handle().window_id();
    let list = cx.default_global::<Guards>().0.entry(id).or_default();
    list.retain(|w| w.strong_count() > 0);
    list.push(Rc::downgrade(&state));
    Guarded(state)
}

/// The guard of the topmost guarded dialog still open over `window`.
pub(crate) fn top(window: &Window, cx: &App) -> Option<GuardRef> {
    let id = window.window_handle().window_id();
    let list = cx.try_global::<Guards>()?.0.get(&id)?;
    list.iter()
        .rev()
        .find(|w| w.upgrade().is_some_and(|s| !s.closed.get()))
        .map(|w| GuardRef(Weak::clone(w)))
}

impl Guarded {
    /// Routes the dialog's cancel (Escape, the x, a backdrop press) through the guard.
    pub fn apply(&self, dialog: Dialog) -> Dialog {
        let weak = Rc::downgrade(&self.0);
        dialog.on_cancel(move |_, window, cx| {
            let Some(state) = weak.upgrade() else {
                return true;
            };
            if state.busy.get() {
                return false;
            }
            if !(state.guard.dirty)(cx) {
                return true;
            }
            ask_to_close(&state, window, cx, Box::new(|_, _| {}), Box::new(|_, _| {}));
            false
        })
    }

    /// Records the dialog's focus node; call right after `open_dialog`, which focuses it.
    pub fn opened(&self, window: &Window, cx: &App) {
        let host = window.focused(cx);
        debug_assert!(host.is_some(), "a dialog opens focused");
        *self.0.host.borrow_mut() = host;
    }

    /// A handle that does not keep the guard alive.
    pub fn downgrade(&self) -> GuardRef {
        GuardRef(Rc::downgrade(&self.0))
    }
}

impl GuardRef {
    /// Whether the dialog holds edits, or is already asking about them.
    pub fn pending(&self, cx: &App) -> bool {
        self.0
            .upgrade()
            .is_some_and(|s| s.busy.get() || (s.guard.dirty)(cx))
    }

    /// Whether a prompt or its save is under way.
    pub fn busy(&self) -> bool {
        self.0.upgrade().is_some_and(|s| s.busy.get())
    }

    /// Whether the dialog is the top one, holding focus.
    pub(crate) fn on_top(&self, window: &Window, cx: &App) -> bool {
        self.0.upgrade().is_some_and(|s| s.on_top(window, cx))
    }

    /// Asks about the edits and, once saved or dropped, runs `then` with the dialog left
    /// open; Cancel or a failed save leaves everything as it was.
    pub fn ask_then(&self, window: &mut Window, cx: &mut App, then: Then) {
        if let Some(state) = self.0.upgrade()
            && !state.busy.get()
        {
            ask(&state, window, cx, then, Box::new(|_, _| {}));
        }
    }

    /// Asks about the edits; once saved or dropped the dialog closes and `closed` runs, and
    /// Cancel or a failed save runs `kept` instead.
    pub(crate) fn ask_to_close(&self, window: &mut Window, cx: &mut App, closed: Then, kept: Then) {
        match self.0.upgrade() {
            Some(state) if !state.busy.get() => ask_to_close(&state, window, cx, closed, kept),
            _ => kept(window, cx),
        }
    }

    /// Waits on the prompt under way: `through` once it is answered and settled, `kept` on
    /// Cancel or a failed save. With no prompt under way, `kept` runs now.
    pub(crate) fn after_answer(
        &self,
        window: &mut Window,
        cx: &mut App,
        through: Then,
        kept: Then,
    ) {
        match self.0.upgrade() {
            Some(state) if state.busy.get() => state.waiters.borrow_mut().push((through, kept)),
            _ => kept(window, cx),
        }
    }
}

// The prompt, answered by closing the dialog once it is the top one again.
fn ask_to_close(state: &Rc<State>, window: &mut Window, cx: &mut App, closed: Then, kept: Then) {
    let weak = Rc::downgrade(state);
    let through: Then = Box::new(move |window, cx| close_when_top(weak, window, cx, closed));
    ask(state, window, cx, through, kept);
}

fn ask(state: &Rc<State>, window: &mut Window, cx: &mut App, through: Then, kept: Then) {
    state.busy.set(true);
    let title = (state.guard.title)(cx);
    let answer = confirm::ask(
        confirm::save_ask(title, None, "Don't save", "Save"),
        window,
        cx,
    );
    let weak = Rc::downgrade(state);
    let waiters = Rc::clone(&state.waiters);
    window
        .spawn(cx, async move |cx| {
            // A prompt dismissed unanswered errs, which keeps the dialog like Cancel.
            let answer = answer.recv().await;
            let saved = match answer {
                Ok(2) => {
                    let outcome = cx.update(|window, cx| {
                        let state = weak.upgrade()?;
                        Some((state.guard.save)(window, cx))
                    });
                    match outcome {
                        Ok(Some(outcome)) => matches!(outcome.recv().await, Ok(Ok(()))),
                        _ => false,
                    }
                }
                Ok(1) => cx
                    .update(|window, cx| {
                        weak.upgrade()
                            .map(|state| (state.guard.discard)(window, cx))
                            .is_some()
                    })
                    .unwrap_or(false),
                _ => false,
            };
            cx.update(|window, cx| {
                if let Some(state) = weak.upgrade() {
                    state.busy.set(false);
                }
                let next = if saved { through } else { kept };
                next(window, cx);
                // A close still waiting for its turn leaves the waiters to it.
                if !weak.upgrade().is_some_and(|state| state.busy.get()) {
                    run_waiters(&waiters, saved, window, cx);
                }
            })
            .ok();
        })
        .detach();
}

fn run_waiters(waiters: &Waiters, through: bool, window: &mut Window, cx: &mut App) {
    for (on_through, on_kept) in waiters.take() {
        let next = if through { on_through } else { on_kept };
        next(window, cx);
    }
}

// Closes the guarded dialog, waiting while another dialog sits over it; a dialog already gone
// is not waited for.
fn close_when_top(weak: Weak<State>, window: &mut Window, cx: &mut App, closed: Then) {
    let Some(state) = weak.upgrade() else {
        return closed(window, cx);
    };
    if state.on_top(window, cx) {
        state.closed.set(true);
        window.close_dialog(cx);
        return closed(window, cx);
    }
    state.busy.set(true);
    let waiters = Rc::clone(&state.waiters);
    drop(state);
    window
        .spawn(cx, async move |cx| {
            // Some(true) once this closed the dialog, Some(false) once it went another way.
            let mut through = None;
            while through.is_none() {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let polled = cx.update(|window, cx| match weak.upgrade() {
                    None => Some(false),
                    Some(state) if state.on_top(window, cx) => {
                        state.closed.set(true);
                        window.close_dialog(cx);
                        Some(true)
                    }
                    Some(_) => None,
                });
                // A window gone with its dialog leaves nothing to settle.
                let Ok(polled) = polled else { return };
                through = polled;
            }
            // Waiting closes are kept when the dialog went without this closing it.
            cx.update(|window, cx| {
                closed(window, cx);
                run_waiters(&waiters, through == Some(true), window, cx);
            })
            .ok();
        })
        .detach();
}
