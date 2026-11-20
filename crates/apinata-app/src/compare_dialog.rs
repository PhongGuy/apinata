//! The Compare dialog on a changed-on-disk banner: the file as the disk holds it against the
//! draft as the writer would save it, read-only until Reload or Overwrite. The same dialog over
//! two fixed texts settles a team clash: the file now against the saved local side.

use crate::ui::SolidRing as _;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use apinata_core::{FileTexts, text_hunks};
use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Entity, ListHorizontalSizingBehavior, Subscription, Task, TestSupportExt,
    UniformListScrollHandle, Window, div, uniform_list,
};

use crate::banner::Choice;
use crate::command::{Command, Target};
use crate::diff_doc::{DiffHunk, DiffRow, rows};
use crate::diff_rows::{DiffPalette, diff_row};
use crate::ui::dialog_footer;
use crate::workspace::{ClashAnswer, Workspace};

/// Answers the banner as the editor does; the dialog sends no command itself.
pub type Answer = Rc<dyn Fn(Choice, &mut Window, &mut App)>;

/// The open dialog's state: both texts and the rows between them.
pub struct CompareDialog {
    workspace: Entity<Workspace>,
    root: PathBuf,
    // None when the dialog shows fixed texts, which it never asks the actor for again.
    target: Option<Target>,
    note: &'static str,
    disk: String,
    draft: String,
    rows: Vec<DiffRow>,
    open: bool,
    // The target's view when the texts were asked for; a new view means the file moved on.
    seen: usize,
    asked: u64,
    scroll: UniformListScrollHandle,
    task: Option<Task<()>>,
    _watch: Subscription,
}

impl CompareDialog {
    /// The file's text on disk; empty when the file is gone.
    pub fn disk(&self) -> &str {
        &self.disk
    }

    /// The draft as the writer would save it.
    pub fn draft(&self) -> &str {
        &self.draft
    }

    /// The diff rows, disk lines removed and draft lines added.
    pub fn rows(&self) -> &[DiffRow] {
        &self.rows
    }

    /// Whether the dialog is still showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    fn new(
        workspace: Entity<Workspace>,
        root: PathBuf,
        target: Target,
        cx: &mut Context<Self>,
    ) -> Self {
        // A republished view of the target is the signal to ask again. Asking publishes a
        // snapshot too, but reuses the target's view, so the watch does not loop.
        let watch = cx.observe(&workspace, |this, _, cx| {
            if this.open && this.view_key(cx) != this.seen {
                this.refresh(cx);
            }
        });
        let mut this = Self {
            workspace,
            root,
            target: Some(target),
            note: "Lines starting with - are on disk, lines starting with + are in your draft.",
            disk: String::new(),
            draft: String::new(),
            rows: Vec::new(),
            open: true,
            seen: 0,
            asked: 0,
            scroll: UniformListScrollHandle::new(),
            task: None,
            _watch: watch,
        };
        this.refresh(cx);
        this
    }

    fn fixed(workspace: Entity<Workspace>, root: PathBuf, disk: String, mine: String) -> Self {
        Self {
            workspace,
            root,
            target: None,
            note: "Lines starting with - are on disk, lines starting with + are your saved side.",
            rows: diff_rows(&disk, &mine),
            disk,
            draft: mine,
            open: true,
            seen: 0,
            asked: 0,
            scroll: UniformListScrollHandle::new(),
            task: None,
            _watch: Subscription::new(|| {}),
        }
    }

    // The address of the target's current view, which changes whenever the actor rebuilds it.
    fn view_key(&self, cx: &App) -> usize {
        let Some(snapshot) = self.workspace.read(cx).snapshot(&self.root, cx) else {
            return 0;
        };
        let content = &snapshot.content;
        let Some(target) = &self.target else {
            return 0;
        };
        match target {
            Target::Request(p) => content.requests.get(p).map(Arc::as_ptr).map(|p| p as usize),
            Target::Folder(p) => content.folders.get(p).map(Arc::as_ptr).map(|p| p as usize),
            Target::Collection => Some(Arc::as_ptr(&content.collection) as usize),
            Target::Environment(name) => content
                .environments
                .iter()
                .find(|e| &e.name == name)
                .map(|e| Arc::as_ptr(e) as usize),
        }
        .unwrap_or(0)
    }

    // Asks the actor for both texts; only the newest answer is kept.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.target.clone() else {
            return;
        };
        self.seen = self.view_key(cx);
        let (reply, answer) = async_channel::bounded(1);
        let command = Command::FileTexts { target, reply };
        let root = self.root.clone();
        if self
            .workspace
            .update(cx, |w, cx| w.send(&root, command, cx))
            .is_none()
        {
            return;
        }
        self.asked += 1;
        let asked = self.asked;
        self.task = Some(cx.spawn(async move |this, cx| {
            let Ok(texts) = answer.recv().await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.asked == asked {
                    this.set(texts, cx);
                }
            });
        }));
    }

    fn set(&mut self, texts: Option<FileTexts>, cx: &mut Context<Self>) {
        let (disk, draft) =
            texts.map_or_else(Default::default, |t| (t.disk.unwrap_or_default(), t.draft));
        self.rows = diff_rows(&disk, &draft);
        (self.disk, self.draft) = (disk, draft);
        cx.notify();
    }
}

// Numbered rows turning the disk text into the draft text; empty when they agree.
fn diff_rows(disk: &str, draft: &str) -> Vec<DiffRow> {
    let hunks: Vec<DiffHunk> = text_hunks(disk, draft)
        .iter()
        .map(|h| DiffHunk {
            old_start: h.old_start,
            new_start: h.new_start,
            lines: h.lines.iter().map(|l| (l.kind, l.text.clone())).collect(),
            unterminated: crate::diff_doc::unterminated(&h.lines),
        })
        .collect();
    rows(&hunks)
}

impl Render for CompareDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().input;
        let radius = cx.theme().radius;
        let note = self.note;
        let body = if self.rows.is_empty() {
            div()
                .id("compare-same")
                .test_support()
                .child("The disk and your draft hold the same text.")
                .into_any_element()
        } else {
            let list_rows = self.rows.clone();
            let palette = DiffPalette::new(&list_rows, window, cx);
            let widest = (0..list_rows.len())
                .max_by_key(|&ix| list_rows[ix].text.chars().count())
                .unwrap_or(0);
            let room = crate::dialog_frame::titled_body_room(window);
            let list = uniform_list(
                "compare-diff-lines",
                list_rows.len(),
                move |range: Range<usize>, _: &mut Window, _: &mut App| {
                    range
                        .map(|ix| diff_row(ix, &list_rows[ix], &palette))
                        .collect::<Vec<_>>()
                },
            )
            .with_width_from_item(Some(widest))
            .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
            .track_scroll(&self.scroll)
            .size_full();
            div()
                .id("compare-diff")
                .test_support()
                .relative()
                .w_full()
                .h((room * 0.6).max(crate::style::resolve(
                    crate::style::COMPARE_DIFF_MIN_HEIGHT,
                    window,
                )))
                .border_1()
                .border_color(border)
                .rounded(radius)
                .overflow_hidden()
                .child(list)
                .vertical_scrollbar(&self.scroll)
                .into_any_element()
        };
        v_flex()
            .id("compare-dialog")
            .test_support()
            .gap_2()
            .child(div().text_color(muted).child(note))
            .child(body)
    }
}

/// Opens the dialog over `window` for `target`; Reload and Overwrite call `answer` with the
/// banner's `Primary` and `Secondary` choice and close it. The returned handle reads its state.
pub fn open(
    workspace: Entity<Workspace>,
    root: PathBuf,
    target: Target,
    answer: Answer,
    window: &mut Window,
    cx: &mut App,
) -> Entity<CompareDialog> {
    let dialog = cx.new(|cx| CompareDialog::new(workspace, root, target, cx));
    let answering = move |id: &'static str, label: &'static str, choice: Choice| {
        let answer = Rc::clone(&answer);
        (
            id,
            label,
            Rc::new(move |window: &mut Window, cx: &mut App| answer(choice, window, cx)) as Act,
        )
    };
    let (reload, overwrite) = (
        answering("compare-reload", "Reload", Choice::Primary),
        answering("compare-overwrite", "Overwrite", Choice::Secondary),
    );
    present(
        &dialog,
        "Compare with the file on disk",
        move |_| (vec![reload.clone()], overwrite.clone()),
        window,
        cx,
    );
    dialog
}

/// Opens the dialog over `window` for the team clash at `rel`: `disk` (the file now) against
/// `mine` (the saved local side). Keep theirs and Use mine settle the clash and close it; Use
/// mine is not offered while the collection is read-only.
pub fn open_copy(
    workspace: Entity<Workspace>,
    root: PathBuf,
    rel: String,
    disk: String,
    mine: String,
    window: &mut Window,
    cx: &mut App,
) -> Entity<CompareDialog> {
    let dialog = cx.new(|_| CompareDialog::fixed(workspace.clone(), root.clone(), disk, mine));
    let settling = move |id: &'static str, label: &'static str, answer: ClashAnswer| {
        let (workspace, root, rel) = (workspace.clone(), root.clone(), rel.clone());
        let act: Act = Rc::new(move |window: &mut Window, cx: &mut App| {
            crate::team_sync_dialog::settle(&workspace, &root, &rel, answer, window, cx)
        });
        (id, label, act)
    };
    let keep = settling(
        "compare-keep-theirs",
        "Keep theirs",
        ClashAnswer::KeepTheirs,
    );
    let mine = settling("compare-use-mine", "Use mine", ClashAnswer::UseMine);
    let state = dialog.clone();
    present(
        &dialog,
        "Compare with your saved side",
        move |cx| {
            let d = state.read(cx);
            if d.workspace.read(cx).is_team_read_only(&d.root) {
                (Vec::new(), keep.clone())
            } else {
                (vec![keep.clone()], mine.clone())
            }
        },
        window,
        cx,
    );
    dialog
}

// A footer button's id, label and action; the button marks the dialog closed around it.
type Act = Rc<dyn Fn(&mut Window, &mut App)>;
type Action = (&'static str, &'static str, Act);

// Shows `dialog` titled `title`, with Close and the buttons `buttons` picks on each render:
// the others, then the primary one.
fn present(
    dialog: &Entity<CompareDialog>,
    title: &'static str,
    buttons: impl Fn(&App) -> (Vec<Action>, Action) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let state = dialog.clone();
    let buttons = Rc::new(buttons);
    window.defer(cx, move |window, cx| {
        // Over another dialog, such as team sync, it sits lower so the one beneath keeps its title.
        let nested = window.has_active_dialog(cx);
        crate::ui::open_dialog(window, cx, move |frame, window, cx| {
            let frame = if nested {
                crate::dialog_frame::place_nested(frame, window)
            } else {
                crate::dialog_frame::place(frame, window)
            };
            let close = {
                let state = state.clone();
                move |_: &_, window: &mut Window, cx: &mut App| {
                    state.update(cx, |d, _| d.open = false);
                    window.close_dialog(cx);
                }
            };
            let button = |(id, label, act): Action| {
                let state = state.clone();
                Button::new(id)
                    .solid_ring(cx)
                    .label(label)
                    .on_click(move |_, window, cx| {
                        state.update(cx, |d, _| d.open = false);
                        act(window, cx);
                        window.close_dialog(cx);
                    })
            };
            let dismissed = {
                let state = state.clone();
                move |_: &_, _: &mut Window, cx: &mut App| {
                    state.update(cx, |d, _| d.open = false);
                    true
                }
            };
            let (others, primary) = buttons(cx);
            frame
                .title(title)
                .close_button(false)
                .on_ok(dismissed.clone())
                .on_cancel(dismissed)
                .child(
                    v_flex().gap_2().child(state.clone()).child(dialog_footer(
                        Some(
                            Button::new("compare-close")
                                .solid_ring(cx)
                                .label("Close")
                                .on_click(close),
                        ),
                        others.into_iter().map(button),
                        button(primary),
                    )),
                )
        });
    });
}
