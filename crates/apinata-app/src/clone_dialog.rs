//! The clone dialog: a URL, a parent folder and a folder name in; a cloned collection out.
//! The form rules live in `clone_form`, the git work in `clone_job`.

use crate::ui::LockButton as _;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Entity, FocusHandle, Focusable, Global, PathPromptOptions, Pixels, Subscription,
    TestSupportExt, WeakEntity, Window, canvas, div, relative,
};

use crate::app::AppState;
use crate::clone_form::{self, Opened};
use crate::clone_job::{CloneEvent, CloneJob};
use crate::form_drafts;
use crate::path_display;
use crate::style::{SHORT_FORM_LABEL_WIDTH, SPACE_1, SPACE_2};
use crate::ui;
use crate::workspace::Workspace;
use apinata_core::{CloneProgress, CloneRequest, GitOptions};

/// Which screen the dialog shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Collecting the URL, parent and name.
    Form,
    /// A clone is in flight.
    Running,
    /// Cancel was asked for; the dialog closes once core has cleaned up.
    Cancelling,
}

/// The dialog's body and state.
pub struct CloneView {
    workspace: Entity<Workspace>,
    url: Entity<InputState>,
    name: Entity<InputState>,
    parent: Option<PathBuf>,
    // The home directory, found once when the dialog opens.
    home: Option<PathBuf>,
    // The name the dialog last wrote itself; any other text in the field was typed by the user.
    filled: String,
    name_edited: bool,
    phase: Phase,
    error: Option<String>,
    job: Option<CloneJob>,
    // The folder the running clone creates; the only one a discarded clone may remove.
    created: Option<PathBuf>,
    progress: Option<CloneProgress>,
    shown_url: Option<String>,
    // Holds focus while a clone runs, so Escape still reaches the dialog after the form is gone.
    running_focus: FocusHandle,
    // The folder cell's width and text size as last laid out; None before its first frame.
    parent_room: Rc<Cell<Option<(Pixels, Pixels)>>>,
    _subscriptions: Vec<Subscription>,
}

// The most recently opened dialog's view; it lapses once the dialog lets go of it.
struct OpenView(WeakEntity<CloneView>);
impl Global for OpenView {}

/// The open dialog's view, for tests and the pilot; None when no dialog is open.
pub fn current(cx: &App) -> Option<Entity<CloneView>> {
    cx.try_global::<OpenView>().and_then(|v| v.0.upgrade())
}

/// Opens the clone dialog over `window`.
pub fn open(workspace: &Entity<Workspace>, window: &mut Window, cx: &mut App) -> Entity<CloneView> {
    let saved = cx
        .try_global::<AppState>()
        .and_then(|s| s.settings.read(cx).get().clone_parent.clone());
    let home = dirs::home_dir();
    let parent = clone_form::default_parent(saved.as_deref(), home.as_deref());
    let view = cx.new(|cx| {
        let url =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://host/owner/repo.git"));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Folder name"));
        let typed = |this: &mut CloneView,
                     _: &Entity<InputState>,
                     e: &InputEvent,
                     window: &mut Window,
                     cx: &mut Context<CloneView>| {
            match e {
                InputEvent::Change => this.sync(window, cx),
                // Enter in a field clones as the button does; a blank URL is no request yet.
                InputEvent::PressEnter { .. } if !this.url(cx).trim().is_empty() => {
                    this.start(window, cx)
                }
                _ => {}
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&url, window, typed),
            cx.subscribe_in(&name, window, typed),
        ];
        CloneView {
            workspace: workspace.clone(),
            url,
            name,
            parent,
            home,
            filled: String::new(),
            name_edited: false,
            phase: Phase::Form,
            error: None,
            job: None,
            created: None,
            progress: None,
            shown_url: None,
            running_focus: cx.focus_handle().tab_stop(true),
            parent_room: Rc::default(),
            _subscriptions: subscriptions,
        }
    });
    // A form closed before cloning comes back as it was left.
    if let Some(draft) = form_drafts::clone_draft(cx) {
        view.update(cx, |v, cx| {
            v.set_url(&draft.url, window, cx);
            v.set_name(&draft.name, window, cx);
        });
    }
    cx.set_global(OpenView(view.downgrade()));
    let body = view.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        // Escape cancels a running clone and keeps the dialog until core has cleaned up. Enter
        // never closes it: the fields start the clone themselves.
        let view = body.clone();
        crate::dialog_frame::place(dialog, window)
            .title("Clone repository")
            .close_button(false)
            .on_ok(|_, _, _| false)
            .on_cancel(move |_, window, cx| view.update(cx, |v, cx| v.dismiss(window, cx)))
            .child(body.clone())
    });
    view.read(cx)
        .url
        .read(cx)
        .focus_handle(cx)
        .focus(window, cx);
    view
}

impl CloneView {
    /// Replaces the home directory used to abbreviate the folder; tests inject theirs.
    pub fn set_home(&mut self, home: Option<PathBuf>) {
        self.home = home;
    }

    /// The current screen.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The URL field's text.
    pub fn url(&self, cx: &App) -> String {
        self.url.read(cx).value().to_string()
    }

    /// The folder name field's text.
    pub fn name(&self, cx: &App) -> String {
        self.name.read(cx).value().to_string()
    }

    /// The chosen parent folder.
    pub fn parent(&self) -> Option<&Path> {
        self.parent.as_deref()
    }

    /// The latest progress line of the running clone.
    pub fn progress(&self) -> Option<&CloneProgress> {
        self.progress.as_ref()
    }

    /// The URL being cloned, as it may be shown (no credentials); set once the clone starts.
    pub fn shown_url(&self) -> Option<&str> {
        self.shown_url.as_deref()
    }

    /// Whether the URL field has keyboard focus.
    pub fn url_focused(&self, window: &Window, cx: &App) -> bool {
        self.url.read(cx).focus_handle(cx).is_focused(window)
    }

    /// The message under the form, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Sets the URL; the name follows it unless the user edited the name.
    pub fn set_url(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.url.update(cx, |i, cx| i.set_value(url, window, cx));
        self.sync(window, cx);
    }

    /// Sets the folder name; text other than the suggestion counts as edited.
    pub fn set_name(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |i, cx| i.set_value(name, window, cx));
        self.sync(window, cx);
    }

    /// Sets the parent folder.
    pub fn set_parent(&mut self, parent: PathBuf, cx: &mut Context<Self>) {
        self.parent = Some(parent);
        cx.notify();
    }

    // The one path for typed and programmatic changes (`set_value` emits no event). Text the
    // dialog did not write marks the name as edited; otherwise the name follows the URL.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.name(cx);
        if typed != self.filled && typed != clone_form::suggested_name(self.url(cx).trim()) {
            self.name_edited = true;
        }
        if !self.name_edited {
            let suggested = clone_form::suggested_name(self.url(cx).trim());
            if typed != suggested {
                self.name
                    .update(cx, |i, cx| i.set_value(&suggested, window, cx));
            }
            self.filled = suggested;
        }
        self.error = None;
        form_drafts::set_clone(&self.url(cx), &self.name(cx), cx);
        cx.notify();
    }

    /// Checks the form and starts the clone; a problem shows as the error and stays on the form.
    pub fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_with(CloneJob::start, window, cx);
    }

    /// `start` with the job built by `make`, so tests can script the event stream.
    pub fn start_with(
        &mut self,
        make: impl FnOnce(CloneRequest, GitOptions, &App) -> CloneJob,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.phase != Phase::Form {
            return;
        }
        let req = match clone_form::check(&self.url(cx), self.parent.as_deref(), &self.name(cx)) {
            Ok(req) => req,
            Err(e) => {
                self.error = Some(e.message());
                cx.notify();
                return;
            }
        };
        if let Err(e) = clone_form::ensure_parent(&req.parent) {
            self.error = Some(e.to_string());
            cx.notify();
            return;
        }
        let opts = self
            .workspace
            .read(cx)
            .git_options()
            .cloned()
            .unwrap_or_default();
        form_drafts::clear_clone(cx);
        self.shown_url = Some(clone_form::shown_url(&req.url));
        self.progress = None;
        self.created = Some(req.parent.join(&req.name));
        let job = make(req, opts, cx);
        let events = job.events().clone();
        self.job = Some(job);
        self.phase = Phase::Running;
        self.error = None;
        self.running_focus.focus(window, cx);
        cx.notify();
        // Each event lands on the view; a channel that closes before `Finished` is a failure.
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let done = matches!(event, CloneEvent::Finished(_));
                let landed =
                    this.update_in(cx, |this, window, cx| this.on_event(event, window, cx));
                if done || landed.is_err() {
                    return;
                }
            }
            let _ = this.update_in(cx, |this, window, cx| this.events_lost(window, cx));
        })
        .detach();
    }

    /// Asks the running clone to stop; the dialog closes when core reports `Cancelled`.
    pub fn cancel(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.phase != Phase::Running {
            return;
        }
        if let Some(job) = &self.job {
            job.cancel();
        }
        self.phase = Phase::Cancelling;
        cx.notify();
    }

    // The dialog's cancel path: true closes it (the form), false keeps it while a clone winds down.
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.phase {
            Phase::Form => true,
            Phase::Running | Phase::Cancelling => {
                self.cancel(window, cx);
                false
            }
        }
    }

    /// The job's thread ended without a result: back to the form with a generic message.
    pub fn events_lost(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.phase == Phase::Form {
            return;
        }
        self.job = None;
        // The user asked to stop, so a lost job is not worth an error.
        if self.phase == Phase::Cancelling {
            window.close_dialog(cx);
            return;
        }
        self.back_to_form(
            "The clone stopped unexpectedly. Try again.".to_owned(),
            window,
            cx,
        );
    }

    // Shows `text` on the form and puts the cursor back in the first field.
    fn back_to_form(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.phase = Phase::Form;
        self.error = Some(text);
        self.url.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn on_event(&mut self, event: CloneEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            CloneEvent::Progress(p) => {
                self.progress = Some(p);
                cx.notify();
            }
            CloneEvent::Finished(Ok(root)) => self.opened(root, window, cx),
            CloneEvent::Finished(Err(e)) => {
                self.job = None;
                match clone_form::clone_error_text(&e) {
                    None => window.close_dialog(cx),
                    Some(text) => self.back_to_form(text, window, cx),
                }
            }
        }
    }

    // Remember the parent, close, then open the collection or say there is none.
    fn opened(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.job = None;
        if self.phase == Phase::Cancelling {
            self.discard(&root);
            window.close_dialog(cx);
            return;
        }
        if let (Some(parent), Some(state)) = (self.parent.clone(), cx.try_global::<AppState>()) {
            let settings = state.settings.clone();
            settings.update(cx, |p, cx| p.update(cx, |s| s.clone_parent = Some(parent)));
        }
        window.close_dialog(cx);
        match clone_form::classify(&root) {
            Opened::Collection(root) => self.workspace.update(cx, |w, cx| w.open_path(root, cx)),
            Opened::NoCollection(root) => {
                window.push_notification(ui::info_toast(clone_form::no_collection_text(&root)), cx)
            }
        }
    }

    // A clone that finished after Cancel: delete the folder this job made, and nothing else.
    // Both sides are compared canonicalized (core may report a resolved root), but the
    // created path itself must be a real directory, so a link is never followed.
    fn discard(&self, root: &Path) {
        let Some(created) = self.created.as_deref() else {
            return;
        };
        let real_dir = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir());
        if !real_dir(created) {
            return;
        }
        let (Ok(created), Ok(root)) = (created.canonicalize(), root.canonicalize()) else {
            return;
        };
        if created == root && real_dir(&root) {
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    // Asks for the parent folder; a cancelled or failed prompt changes nothing.
    fn browse(&mut self, cx: &mut Context<Self>) {
        let reply = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose folder".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = reply.await else {
                return;
            };
            if let Some(dir) = paths.into_iter().next() {
                let _ = this.update(cx, |this, cx| this.set_parent(dir, cx));
            }
        })
        .detach();
    }

    // Phase text, a bar and the URL; the Cancel button disables once cancelling.
    fn running(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let cancelling = self.phase == Phase::Cancelling;
        let text = if cancelling {
            "Cancelling…".to_owned()
        } else {
            self.progress
                .as_ref()
                .map_or_else(|| "Cloning…".to_owned(), |p| p.phase.clone())
        };
        // Without a percentage the bar shows a fixed partial segment, never an animation.
        let fill = self
            .progress
            .as_ref()
            .and_then(|p| p.percent)
            .map_or(0.25, |p| f32::from(p.min(100)) / 100.0);
        let bar = div()
            .h(SPACE_1)
            .w_full()
            .rounded_full()
            .bg(cx.theme().muted)
            .child(
                div()
                    .h_full()
                    .w(relative(fill))
                    .rounded_full()
                    .bg(cx.theme().primary),
            );
        v_flex()
            .track_focus(&self.running_focus)
            .gap(SPACE_2)
            .child(
                v_flex()
                    .id("clone-progress")
                    .test_support()
                    .gap(SPACE_2)
                    .child(text)
                    .child(bar)
                    .children(
                        self.shown_url
                            .clone()
                            .map(|url| div().text_color(cx.theme().muted_foreground).child(url)),
                    ),
            )
            .child(ui::dialog_footer(
                None,
                [],
                Button::new("clone-cancel")
                    .label("Cancel")
                    .lock(cancelling, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
            ))
    }

    fn form(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        // The folder is fitted to the room its cell got from layout, at the size it renders;
        // until the first layout it shows whole and clips.
        let home = self.home.as_deref();
        let (parent_text, parent_full, cut) = match &self.parent {
            Some(p) => {
                let shown = match self.parent_room.get() {
                    Some((room, size)) => path_display::display(
                        p,
                        home,
                        f32::from(room),
                        &path_display::measurer(window, size),
                    ),
                    None => path_display::tilde(p, home),
                };
                let cut = path_display::is_shortened(p, home, &shown);
                (shown, p.display().to_string(), cut)
            }
            None => (
                "Choose a folder".to_owned(),
                "Choose a folder".to_owned(),
                false,
            ),
        };
        // A changed room redraws the form once the frame is done.
        let (cell, me) = (Rc::clone(&self.parent_room), cx.entity().downgrade());
        let measure = canvas(
            move |bounds, window, cx| {
                let size = window.text_style().font_size.to_pixels(window.rem_size());
                let now = Some((bounds.size.width, size));
                if cell.replace(now) != now {
                    window.defer(cx, move |_, cx| {
                        me.update(cx, |_, cx| cx.notify()).ok();
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let note = clone_form::is_unencrypted(&self.url(cx)).then(|| {
            div()
                .id("clone-http-note")
                .test_support()
                .text_color(cx.theme().warning)
                .child("This URL is not encrypted")
        });
        let error = self.error.clone().map(|message| {
            div()
                .id("clone-error")
                .test_support()
                .text_color(cx.theme().danger)
                .child(message)
        });
        let field = |id: &'static str, label: &'static str, input: &Entity<InputState>| {
            ui::form_row(
                format!("{id}-row"),
                label,
                ui::padded_input(
                    Input::new(input)
                        .id(id)
                        .w_full()
                        .bg(ui::field_fill(false, cx)),
                    input,
                ),
            )
            .label_width(SHORT_FORM_LABEL_WIDTH)
        };
        let parent_row = ui::form_row(
            "clone-parent-row",
            "Folder",
            // The path clips with an ellipsis so Browse never leaves the dialog.
            h_flex()
                .w_full()
                .min_w_0()
                .gap(SPACE_2)
                .items_center()
                .child(
                    div()
                        .id("clone-parent")
                        .test_support()
                        .aria_label(parent_full.clone())
                        .aria_value(parent_text.clone())
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .truncate()
                        .when(cut, |d| {
                            d.tooltip(ui::text_tooltip("clone-parent-tooltip", parent_full))
                        })
                        .child(measure)
                        .child(parent_text),
                )
                .child(
                    // Browse sits beside no input, and takes Enter as its own press.
                    Button::new("clone-browse")
                        .key_context(ui::DIALOG_FOOTER_CONTEXT)
                        .flex_none()
                        .label("Browse…")
                        .on_click(cx.listener(|this, _, _, cx| this.browse(cx))),
                ),
        )
        .label_width(SHORT_FORM_LABEL_WIDTH);
        v_flex()
            .gap(SPACE_2)
            .child(field("clone-url", "URL", &self.url))
            .children(note)
            .child(parent_row)
            .child(field("clone-name", "Name", &self.name))
            .children(error)
            .child(ui::dialog_footer(
                Some(
                    Button::new("clone-cancel")
                        .label("Cancel")
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
                [],
                Button::new("clone-start")
                    .label("Clone")
                    .primary()
                    .on_click(cx.listener(|this, _, window, cx| this.start(window, cx))),
            ))
    }
}

impl Render for CloneView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.phase {
            Phase::Form => self.form(window, cx).into_any_element(),
            Phase::Running | Phase::Cancelling => self.running(cx).into_any_element(),
        };
        v_flex()
            .id("clone-dialog")
            .test_support()
            .gap(SPACE_2)
            .child(body)
    }
}
