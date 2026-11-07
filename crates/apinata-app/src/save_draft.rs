//! The Save dialog for a draft request: asks for a name and a folder before the draft is
//! written as a file.

use crate::ui::LockButton as _;
use apinata_core::ItemPath;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme as _, IndexPath, WindowExt, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Entity, Global, PathPromptOptions, Result, SharedString, Subscription, TestSupportExt,
    Window, div,
};

use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::command::{Command, TreeOp};
use crate::snapshot::{Content, RowKind};
use crate::style::{SHORT_FORM_LABEL_WIDTH, SPACE_2};
use crate::ui;
use crate::workspace::Workspace;

/// What the user chose: the request's name and the folder it goes in.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveAs {
    /// The request name, trimmed and never blank.
    pub name: String,
    /// The folder to save into; the root for the collection itself.
    pub parent: ItemPath,
    /// The collection to write into when the draft lives elsewhere (a scratch draft); `None`
    /// saves into the draft's own collection.
    pub target: Option<PathBuf>,
}

/// The folder picker's choices: the collection root first, then every folder by item path in
/// row order.
pub fn folders(content: &Content) -> Vec<(ItemPath, SharedString)> {
    let folders = content
        .rows
        .iter()
        .filter(|r| r.kind == RowKind::Folder)
        .map(|r| (r.path.clone(), r.path.as_str().to_owned().into()));
    std::iter::once((ItemPath::root(), "Collection root".into()))
        .chain(folders)
        .collect()
}

/// Replaces the native folder prompt of the open-or-new entry; the closure returns the folder
/// the user "picked", or `None` for a cancel. Tests set it, the app never does.
#[derive(Clone)]
pub struct FolderPickSeam(pub std::rc::Rc<dyn Fn() -> Option<PathBuf>>);
impl Global for FolderPickSeam {}

/// The label of the collection select's first entry, which opens or creates a collection.
const OPEN_ENTRY: &str = "Open or new collection...";

/// The collection choice of a scratch draft's dialog: the open-or-new entry, then the
/// open user collections.
struct Choice {
    workspace: Entity<Workspace>,
    roots: Vec<PathBuf>,
    labels: Vec<SharedString>,
    select: Entity<SelectState<Vec<SharedString>>>,
    picked: Option<usize>,
    // A collection just opened whose snapshot has not arrived: its folders are listed then.
    waiting: Option<PathBuf>,
    _confirm: Option<Subscription>,
    _watch: Option<Subscription>,
}

/// The dialog's body: a name input, an optional collection picker and a folder picker.
pub struct SaveDraftForm {
    folders: Vec<ItemPath>,
    name: Entity<InputState>,
    folder: Entity<SelectState<Vec<SharedString>>>,
    choice: Option<Choice>,
    _name_changes: Subscription,
}

impl SaveDraftForm {
    /// A form showing `name` with `parent` picked; a `parent` missing from `folders` picks the
    /// root.
    pub fn new(
        folders: Vec<(ItemPath, SharedString)>,
        parent: ItemPath,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let at = folders.iter().position(|(p, _)| *p == parent).unwrap_or(0);
        let (paths, labels): (Vec<_>, Vec<_>) = folders.into_iter().unzip();
        let name = cx.new(|cx| InputState::new(window, cx).default_value(name.to_owned()));
        let folder = cx.new(|cx| SelectState::new(labels, Some(IndexPath::new(at)), window, cx));
        // The Save button's enabled state follows the name, so each keystroke re-renders.
        let name_changes = cx.subscribe(&name, |_, _, _: &InputEvent, cx| cx.notify());
        SaveDraftForm {
            folders: paths,
            name,
            folder,
            choice: None,
            _name_changes: name_changes,
        }
    }

    /// A form for a scratch draft: it also asks for the collection, among the open user
    /// collections and an entry that opens or creates one. Save waits until one is picked.
    pub fn for_scratch(
        workspace: &Entity<Workspace>,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let roots: Vec<PathBuf> = (workspace.read(cx).user_collections().into_iter())
            .map(|c| c.root().to_path_buf())
            .collect();
        let mut form = Self::new(Vec::new(), ItemPath::root(), name, window, cx);
        let picked = (!roots.is_empty()).then_some(0);
        form.choice = Some(Choice {
            workspace: workspace.clone(),
            roots: Vec::new(),
            labels: Vec::new(),
            select: cx.new(|cx| SelectState::new(Vec::new(), None, window, cx)),
            picked: None,
            waiting: None,
            _confirm: None,
            _watch: None,
        });
        form.set_roots(roots, picked, window, cx);
        // A collection opened from the dialog loads later; its folders are listed on arrival.
        let watch = cx.observe_in(workspace, window, |this, _, window, cx| {
            this.settle_waiting(window, cx)
        });
        if let Some(choice) = form.choice.as_mut() {
            choice._watch = Some(watch);
        }
        form
    }

    /// The collection select's entries in display order; empty for a form without one.
    pub fn collection_labels(&self) -> Vec<SharedString> {
        self.choice
            .as_ref()
            .map(|c| c.labels.clone())
            .unwrap_or_default()
    }

    // Rebuilds the collection select over `roots` with `picked` chosen, then lists that
    // collection's folders.
    fn set_roots(
        &mut self,
        roots: Vec<PathBuf>,
        picked: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(choice) = self.choice.as_mut() else {
            return;
        };
        // The open entry leads, so it is found without a list to scan; a root sits one row later.
        let labels: Vec<SharedString> = std::iter::once(SharedString::from(OPEN_ENTRY))
            .chain((roots.iter()).map(|r| choice.workspace.read(cx).collection_name(r, cx)))
            .collect();
        let row = picked.map(|p| IndexPath::new(p + 1));
        let select = cx.new(|cx| SelectState::new(labels.clone(), row, window, cx));
        let confirm = cx.subscribe_in(
            &select,
            window,
            |this, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                if let SelectEvent::Confirm(Some(_)) = event {
                    this.collection_confirmed(window, cx);
                }
            },
        );
        choice.labels = labels;
        choice.roots = roots;
        choice.select = select;
        choice.picked = picked;
        choice._confirm = Some(confirm);
        self.relist(window, cx);
    }

    // The collection select changed: a collection re-lists the folders, the first entry asks
    // for a directory and puts the previous choice back meanwhile.
    fn collection_confirmed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(choice) = self.choice.as_mut() else {
            return;
        };
        let at = choice.select.read(cx).selected_index(cx).map(|ix| ix.row);
        match at {
            Some(at) if at > 0 => {
                choice.picked = Some(at - 1);
                self.relist(window, cx);
            }
            Some(_) => {
                let back = choice.picked.map(|p| IndexPath::new(p + 1));
                choice
                    .select
                    .update(cx, |s, cx| s.set_selected_index(back, window, cx));
                self.prompt_open(window, cx);
            }
            None => {}
        }
    }

    // Asks for a directory, creates a collection in it when it is empty, opens it and picks it.
    fn prompt_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(choice) = self.choice.as_ref() else {
            return;
        };
        let workspace = choice.workspace.clone();
        // The seam, when set, answers in place of the native prompt.
        let seam = cx.try_global::<FolderPickSeam>().map(|s| Rc::clone(&s.0));
        let native = seam.is_none().then(|| {
            cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Open or create collection".into()),
            })
        });
        cx.spawn_in(window, async move |this, cx| {
            let picked = match (seam, native) {
                (Some(pick), _) => pick(),
                (None, Some(reply)) => match reply.await {
                    Ok(Ok(Some(paths))) => paths.into_iter().next(),
                    _ => None,
                },
                (None, None) => None,
            };
            let Some(dir) = picked else {
                return;
            };
            let made = dir.clone();
            let job = cx.background_spawn(async move { create_if_empty(&made) });
            if let Err(e) = job.await {
                // A silent return would leave the user wondering why nothing opened.
                let _ = this.update_in(cx, |_, window, cx| {
                    window.push_notification(ui::info_toast(e), cx);
                });
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                workspace.update(cx, |w, cx| w.open_path(dir.clone(), cx));
                this.pick_opened(dir, window, cx);
            });
        })
        .detach();
    }

    // Adds a just-opened collection to the select if it is new, and picks it.
    fn pick_opened(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(choice) = self.choice.as_ref() else {
            return;
        };
        let mut roots = choice.roots.clone();
        let at = match roots.iter().position(|r| *r == dir) {
            Some(at) => at,
            None => {
                roots.push(dir.clone());
                roots.len() - 1
            }
        };
        self.set_roots(roots, Some(at), window, cx);
        if let Some(choice) = self.choice.as_mut() {
            choice.waiting = Some(dir);
        }
    }

    // Once a waited-for collection has its snapshot, its folders and label replace the stubs.
    fn settle_waiting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(choice) = self.choice.as_mut() else {
            return;
        };
        let Some(root) = choice.waiting.clone() else {
            return;
        };
        if choice.workspace.read(cx).snapshot(&root, cx).is_none() {
            return;
        }
        choice.waiting = None;
        let (roots, picked) = (choice.roots.clone(), choice.picked);
        self.set_roots(roots, picked, window, cx);
    }

    // Lists the folders of the picked collection, or only the root when none has loaded.
    fn relist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(choice) = self.choice.as_ref() else {
            return;
        };
        let listed = (choice.picked.and_then(|at| choice.roots.get(at)))
            .and_then(|root| choice.workspace.read(cx).snapshot(root, cx))
            .map(|snapshot| folders(&snapshot.content))
            .unwrap_or_else(|| vec![(ItemPath::root(), "Collection root".into())]);
        let (paths, labels): (Vec<_>, Vec<_>) = listed.into_iter().unzip();
        self.folders = paths;
        self.folder = cx.new(|cx| SelectState::new(labels, Some(IndexPath::new(0)), window, cx));
        cx.notify();
    }

    /// The name input.
    pub fn name_input(&self) -> &Entity<InputState> {
        &self.name
    }

    /// The picked folder.
    pub fn folder(&self, cx: &App) -> ItemPath {
        self.folder
            .read(cx)
            .selected_index(cx)
            .and_then(|ix| self.folders.get(ix.row).cloned())
            .unwrap_or_else(ItemPath::root)
    }

    /// Picks `parent` in the folder picker; ignored when it is not listed.
    pub fn set_folder(&mut self, parent: &ItemPath, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(at) = self.folders.iter().position(|p| p == parent) {
            self.folder.update(cx, |s, cx| {
                s.set_selected_index(Some(IndexPath::new(at)), window, cx)
            });
        }
    }

    /// The trimmed name and picked folder; `None` while the name is blank.
    pub fn answer(&self, cx: &App) -> Option<SaveAs> {
        let name = self.name.read(cx).value().trim().to_owned();
        // A scratch draft needs a collection; without one there is nothing to save into.
        let target = match &self.choice {
            Some(c) => Some(c.roots.get(c.picked?)?.clone()),
            None => None,
        };
        (!name.is_empty()).then(|| SaveAs {
            name,
            parent: self.folder(cx),
            target,
        })
    }
}

impl Render for SaveDraftForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // One-word labels take a short column, so both fields fill the rest of the row.
        let collection = self.choice.as_ref().map(|c| {
            ui::form_row(
                "save-draft-collection-row",
                "Collection",
                ui::dialog_select(
                    Select::new(&c.select)
                        .id("save-draft-collection")
                        .placeholder("Choose a collection")
                        .w_full(),
                )
                .w_full(),
            )
            .label_width(SHORT_FORM_LABEL_WIDTH)
        });
        // With nothing open the select holds only the open entry; say why.
        let none_open = self
            .choice
            .as_ref()
            .is_some_and(|c| c.roots.is_empty())
            .then(|| {
                div()
                    .id("save-draft-no-collection")
                    .test_support()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No collection is open. Open or create one to save into.")
            });
        v_flex()
            .id("save-draft-form")
            .gap(SPACE_2)
            .children(none_open)
            .child(
                ui::form_row(
                    "save-draft-name-row",
                    "Name",
                    ui::padded_input(
                        Input::new(&self.name)
                            .id("save-draft-name")
                            .w_full()
                            .bg(ui::field_fill(false, cx)),
                        &self.name,
                    ),
                )
                .label_width(SHORT_FORM_LABEL_WIDTH),
            )
            .children(collection)
            .child(
                ui::form_row(
                    "save-draft-folder-row",
                    "Folder",
                    ui::dialog_select(Select::new(&self.folder).id("save-draft-folder").w_full())
                        .w_full(),
                )
                .label_width(SHORT_FORM_LABEL_WIDTH),
            )
    }
}

/// Opens the "Save request" dialog over `window` on the next effect cycle with the name
/// selected for typing over. The receiver yields once: the answer on Save or Enter, `None`
/// on Cancel or Escape; it errs when the dialog goes away unanswered.
pub fn ask(
    form: Entity<SaveDraftForm>,
    window: &mut Window,
    cx: &mut App,
) -> async_channel::Receiver<Option<SaveAs>> {
    // A one-slot channel and try_send: whichever answer lands first wins, later ones drop.
    let (tx, rx) = async_channel::bounded(1);
    window.defer(cx, move |window, cx| {
        let input = form.read(cx).name.clone();
        let builder_form = form.clone();
        window.open_dialog(cx, move |dialog, window, cx| {
            let dialog = crate::dialog_frame::place(dialog, window);
            let answer = builder_form.read(cx).answer(cx);
            // Enter and the Save button both save, and only while the name is not blank.
            let save = {
                let (tx, form) = (tx.clone(), builder_form.clone());
                move |cx: &mut App| match form.read(cx).answer(cx) {
                    Some(answer) => {
                        let _ = tx.try_send(Some(answer));
                        true
                    }
                    None => false,
                }
            };
            let on_ok = save.clone();
            let cancel_tx = tx.clone();
            let click_tx = tx.clone();
            let buttons = ui::dialog_footer(
                Some(Button::new("save-draft-cancel").label("Cancel").on_click(
                    move |_, window, cx| {
                        let _ = click_tx.try_send(None);
                        window.close_dialog(cx);
                    },
                )),
                [],
                Button::new("save-draft-save")
                    .label("Save")
                    .primary()
                    .lock_filled(answer.is_none(), cx)
                    .on_click(move |_, window, cx| {
                        if save(cx) {
                            window.close_dialog(cx);
                        }
                    }),
            );
            dialog
                .title("Save request")
                .close_button(false)
                .on_ok(move |_, _, cx| on_ok(cx))
                .on_cancel(move |_, _, _| {
                    let _ = cancel_tx.try_send(None);
                    true
                })
                .child(
                    v_flex()
                        .id("save-draft-dialog")
                        .test_support()
                        .child(builder_form.clone())
                        .child(buttons),
                )
        });
        // The dialog takes focus as it opens; the name input takes it back, ready to retype.
        input.update(cx, |i, cx| {
            i.focus(window, cx);
            i.select_all(window, cx);
        });
    });
    rx
}

/// Opens the Save dialog for the draft `draft` of the collection at `root`, prefilled with
/// its name and its own folder, as `ask` does; `None` with nothing shown when it is gone.
pub fn ask_for(
    workspace: &Entity<Workspace>,
    root: &Path,
    draft: ItemPath,
    window: &mut Window,
    cx: &mut App,
) -> Option<async_channel::Receiver<Option<SaveAs>>> {
    let snapshot = workspace.read(cx).snapshot(root, cx)?;
    let view = snapshot.content.requests.get(&draft)?;
    let form = if workspace.read(cx).is_scratch(root) {
        cx.new(|cx| SaveDraftForm::for_scratch(workspace, &view.name, window, cx))
    } else {
        let folders = folders(&snapshot.content);
        let parent = draft.parent().unwrap_or_else(ItemPath::root);
        cx.new(|cx| SaveDraftForm::new(folders, parent, &view.name, window, cx))
    };
    Some(ask(form, window, cx))
}

/// The refusal for a folder that holds other files: what to choose instead.
pub fn not_empty_refusal(dir: &Path) -> String {
    let name = dir.file_name().map_or_else(
        || dir.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    format!(
        "\"{name}\" is not empty. Choose an empty folder, or one that has an opencollection.yml."
    )
}

/// Creates a collection in `dir`, named for the folder, and returns its root. A refusal reads
/// the same here as in the Save dialog.
pub fn create_collection(dir: &Path) -> Result<std::path::PathBuf, String> {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    apinata_core::Collection::create(dir, &name)
        .map(|c| c.root().to_path_buf())
        .map_err(|e| match e {
            apinata_core::CreateError::NotEmpty { .. } => not_empty_refusal(dir),
            e => e.to_string(),
        })
}

/// Creates a collection in `dir` when it is an empty directory; any other directory is left
/// for `open_path` to open or refuse.
fn create_if_empty(dir: &Path) -> Result<(), String> {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned());
    // Core decides what counts as empty (a lone `.git` does). A folder that is already a
    // collection is left for `open_path`; any other non-empty one is refused with advice.
    match apinata_core::Collection::create(dir, &name.unwrap_or_default()) {
        Ok(_) => Ok(()),
        Err(apinata_core::CreateError::NotEmpty { .. })
            if dir.join("opencollection.yml").is_file() =>
        {
            Ok(())
        }
        Err(apinata_core::CreateError::NotEmpty { .. }) => Err(not_empty_refusal(dir)),
        Err(e) => Err(format!("Could not create a collection there: {e}")),
    }
}

/// Writes the draft under the chosen name and folder and returns the command id; the
/// actor's `Moved` answer carries any open tab over to the new file. A save into another
/// collection returns `None`: that collection's `Created` notice closes the tab.
pub fn send(
    workspace: &Entity<Workspace>,
    root: &Path,
    draft: ItemPath,
    save_as: SaveAs,
    cx: &mut App,
) -> Option<u64> {
    send_with(workspace, root, draft, save_as, Vec::new(), cx)
}

/// `send`, with the script `texts` typed in the draft: a save into another collection approves
/// them there once written; a save in place leaves approving to the caller.
pub fn send_with(
    workspace: &Entity<Workspace>,
    root: &Path,
    draft: ItemPath,
    save_as: SaveAs,
    texts: Vec<String>,
    cx: &mut App,
) -> Option<u64> {
    if let Some(target) = save_as.target {
        workspace.update(cx, |w, cx| {
            w.save_draft_into(
                root,
                draft,
                &target,
                save_as.parent,
                save_as.name,
                texts,
                cx,
            )
        });
        return None;
    }
    let op = TreeOp::SaveDraft {
        draft,
        parent: save_as.parent,
        name: save_as.name,
    };
    workspace.update(cx, |w, cx| w.send(root, Command::Tree(op), cx))
}
