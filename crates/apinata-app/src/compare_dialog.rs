//! The Compare dialog on a changed-on-disk banner: the file as the disk holds it against the
//! draft as the writer would save it, read-only until Reload or Overwrite.

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
use crate::workspace::Workspace;

/// Answers the banner as the editor does; the dialog sends no command itself.
pub type Answer = Rc<dyn Fn(Choice, &mut Window, &mut App)>;

/// The open dialog's state: both texts and the rows between them.
pub struct CompareDialog {
    workspace: Entity<Workspace>,
    root: PathBuf,
    target: Target,
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
            target,
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

    // The address of the target's current view, which changes whenever the actor rebuilds it.
    fn view_key(&self, cx: &App) -> usize {
        let Some(snapshot) = self.workspace.read(cx).snapshot(&self.root, cx) else {
            return 0;
        };
        let content = &snapshot.content;
        match &self.target {
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
        self.seen = self.view_key(cx);
        let (reply, answer) = async_channel::bounded(1);
        let command = Command::FileTexts {
            target: self.target.clone(),
            reply,
        };
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
        let note = "Lines starting with - are on disk, lines starting with + are in your draft.";
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
    let state = dialog.clone();
    window.defer(cx, move |window, cx| {
        window.open_dialog(cx, move |frame, window, _| {
            let frame = crate::dialog_frame::place(frame, window);
            let close = {
                let state = state.clone();
                move |_: &_, window: &mut Window, cx: &mut App| {
                    state.update(cx, |d, _| d.open = false);
                    window.close_dialog(cx);
                }
            };
            let answering = |id: &'static str, label: &'static str, choice: Choice| {
                let (state, answer) = (state.clone(), Rc::clone(&answer));
                Button::new(id).label(label).on_click(move |_, window, cx| {
                    state.update(cx, |d, _| d.open = false);
                    answer(choice, window, cx);
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
            frame
                .title("Compare with the file on disk")
                .close_button(false)
                .on_ok(dismissed.clone())
                .on_cancel(dismissed)
                .child(v_flex().gap_2().child(state.clone()).child(dialog_footer(
                    Some(Button::new("compare-close").label("Close").on_click(close)),
                    [answering("compare-reload", "Reload", Choice::Primary)],
                    answering("compare-overwrite", "Overwrite", Choice::Secondary),
                )))
        });
    });
    dialog
}
