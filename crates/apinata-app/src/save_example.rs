//! The "Save response as example" dialog and the rules for what can be saved, including the
//! check that swaps the request's secret values for placeholders before they reach the file.

use crate::ui::LockButton as _;
use apinata_core::{
    ExampleBodyKind, ExampleResponse, ItemPath, NamedSecret, RequestKind, Response,
    bytes_hold_secret, replace_secrets, request_secrets as core_request_secrets,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use std::future::Future;
use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::AppState;
use crate::command::{Command, NoticeKind, TreeOp};
use crate::confirm::{self, Ask, Choice};
use crate::form_drafts::{self, ExampleDraft};
use crate::send_model::plan_message;
use crate::snapshot::RequestView;
use crate::style::{SHORT_FORM_LABEL_WIDTH, SPACE_2, SPACE_3};
use crate::tabs::TabEntry;
use crate::ui;
use crate::workspace::{Workspace, WorkspaceEvent};

/// Bodies at or above this size cannot be saved as an example (5 MiB).
pub const BODY_LIMIT: u64 = ExampleResponse::BODY_LIMIT;

/// The note shown in the dialog, since an example lands in a committed file.
const NOTE: &str = "Examples are saved in the request file and committed with the collection, \
                    response headers and body included. Remove anything secret before saving.";

/// The `dismissed_tips` id of the commit note, stored once the user ticks "Don't show again".
pub const NOTE_TIP: &str = "example-commit-note";

/// `example`, else the first free `example (n)` from 1 up, against the request's example names.
pub fn initial_name(existing: &[String]) -> String {
    let taken = |name: &str| existing.iter().any(|e| e == name);
    if !taken("example") {
        return "example".into();
    }
    (1..)
        .map(|n| format!("example ({n})"))
        .find(|name| !taken(name))
        .expect("a free name")
}

/// Whether a request may save examples at all: not a draft, which has no file yet, and not
/// a GraphQL request, whose file does not hold examples.
pub fn offered(view: &RequestView) -> bool {
    !view.draft && view.kind == RequestKind::Http
}

/// Why a draft cannot hold examples, for the button's tooltip and the command's toast.
pub const DRAFT_BLOCKED: &str = "Save the request before saving examples";

/// Why a GraphQL request cannot hold examples.
pub const GRAPHQL_BLOCKED: &str = "GraphQL requests cannot hold examples";

/// The reason `view` cannot hold examples, or `None` exactly when [`offered`] is true.
pub fn blocked(view: &RequestView) -> Option<&'static str> {
    if view.draft {
        Some(DRAFT_BLOCKED)
    } else if view.kind != RequestKind::Http {
        Some(GRAPHQL_BLOCKED)
    } else {
        None
    }
}

/// Whether `response` can become an example: present, not truncated, under `BODY_LIMIT`.
pub fn can_save(response: Option<&Response>) -> bool {
    response.is_some_and(ExampleResponse::savable)
}

/// The example response for `response`; core re-indents a JSON body.
pub fn example_response(response: &Response) -> ExampleResponse {
    ExampleResponse::from_response(response)
}

/// Labels of the `secrets` found in `response`'s header values, text body and binary body, deduplicated,
/// in `secrets` order.
pub fn secrets_in(response: &ExampleResponse, secrets: &[NamedSecret]) -> Vec<String> {
    let mut texts: Vec<String> = response.headers.iter().map(|(_, v)| v.clone()).collect();
    texts.extend(text_body(response).map(str::to_owned));
    let mut found: Vec<String> = Vec::new();
    // Replace in redaction order, so a secret inside a longer one counts only where it
    // stands alone.
    for secret in secrets {
        let mut hit = false;
        for text in &mut texts {
            let next = replace_secrets(text, std::slice::from_ref(secret));
            hit |= next != *text;
            *text = next;
        }
        if hit && !found.iter().any(|l| l == secret.label()) {
            found.push(secret.label().to_owned());
        }
    }
    // A binary body is checked as bytes; its hits join the list in `secrets` order.
    let binary = binary_secrets(response, secrets);
    for secret in secrets {
        let label = secret.label();
        if binary.iter().any(|l| l == label) && !found.iter().any(|l| l == label) {
            found.push(label.to_owned());
        }
    }
    found
}

/// The labels of the `secrets` found in `response`'s binary body, in `secrets` order.
/// The body is read as raw bytes; replacing inside them would corrupt the file.
pub fn binary_secrets(response: &ExampleResponse, secrets: &[NamedSecret]) -> Vec<String> {
    let Some(bytes) = binary_bytes(response) else {
        return Vec::new();
    };
    let mut found: Vec<String> = Vec::new();
    for secret in secrets {
        if bytes_hold_secret(&bytes, std::slice::from_ref(secret))
            && !found.iter().any(|l| l == secret.label())
        {
            found.push(secret.label().to_owned());
        }
    }
    found
}

// The bytes a binary body stands for; text that is not base64 is checked as it stands.
fn binary_bytes(response: &ExampleResponse) -> Option<Vec<u8>> {
    let body = (response.body.as_ref()).filter(|b| b.kind == ExampleBodyKind::Binary)?;
    Some(
        STANDARD
            .decode(&body.data)
            .unwrap_or_else(|_| body.data.clone().into_bytes()),
    )
}

/// The dialog line for secrets found, one of which sits in a binary body that will be withheld.
pub fn binary_secrets_note(labels: &[String]) -> String {
    format!(
        "Contains secret values: {}. Saving replaces them with placeholders; the binary body \
         will be withheld, since a secret inside it cannot be replaced.",
        labels.join(", ")
    )
}

/// The line added to an example's description when its body was withheld.
pub const WITHHELD_NOTE: &str = ExampleResponse::WITHHELD_NOTE;

/// The toast shown after saving an example whose body was withheld.
pub const WITHHELD_TOAST: &str =
    "Example saved without its body: the binary body held a secret value";

// Base64 of binary bytes holds no secret text, and a replacement would corrupt it.
fn text_body(response: &ExampleResponse) -> Option<&str> {
    (response.body.as_ref())
        .filter(|b| b.kind != ExampleBodyKind::Binary)
        .map(|b| b.data.as_str())
}

/// What the secret check found before the dialog opened.
#[derive(Debug, Clone, PartialEq)]
pub enum SecretCheck {
    /// The labels of the secret values the example holds; empty when it holds none.
    Found(Vec<String>),
    /// As `Found`, and a binary body holds one of the values, so saving withholds that body.
    FoundInBinary(Vec<String>),
    /// The request could not be resolved, so the example was not checked; the reason.
    Unchecked(String),
}

/// What the user typed: the example's name and its description, both trimmed.
#[derive(Debug, Clone, PartialEq)]
pub struct ExampleAnswer {
    /// The name, never blank.
    pub name: String,
    /// The description, possibly empty.
    pub description: String,
    /// True when the user chose to keep the secret values as received.
    pub keep_secrets: bool,
}

/// The dialog body: name input (`save-example-name`), description input
/// (`save-example-description`), the commit note (`save-example-note`) with its "Don't show
/// again" checkbox (`save-example-note-dismiss`) unless dismissed, and the secret check's
/// finding (`example-secrets` or `example-unchecked`) when there is one.
pub struct SaveExampleForm {
    name: Entity<InputState>,
    description: Entity<InputState>,
    check: SecretCheck,
    show_note: bool,
    dismissed: bool,
    // The collection root, request and offered name a draft is kept under, once asked to.
    draft: Option<(PathBuf, ItemPath, String)>,
    _changes: [Subscription; 2],
}

impl SaveExampleForm {
    /// A form showing `name`, an empty description and what `check` found.
    pub fn new(
        name: &str,
        check: SecretCheck,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).default_value(name.to_owned()));
        let description = cx.new(|cx| InputState::new(window, cx));
        // The Save button's enabled state follows the name, so each keystroke re-renders, and
        // typing in either field updates the kept draft.
        let typed = |this: &mut Self, _, e: &InputEvent, cx: &mut Context<Self>| {
            if matches!(e, InputEvent::Change) {
                this.store_draft(cx);
            }
            cx.notify();
        };
        let changes = [
            cx.subscribe(&name, typed),
            cx.subscribe(&description, typed),
        ];
        // The dismissal is read once, so a tick keeps the note in this dialog and hides it next time.
        let show_note = cx.try_global::<AppState>().is_none_or(|app| {
            let tips = &app.settings.read(cx).get().dismissed_tips;
            !tips.iter().any(|t| t == NOTE_TIP)
        });
        SaveExampleForm {
            name,
            description,
            check,
            show_note,
            dismissed: false,
            draft: None,
            _changes: changes,
        }
    }

    // Fills the form from the draft kept for `item` of the collection at `root`, and keeps
    // what is typed from now on as that draft.
    fn keep_draft(
        &mut self,
        root: &Path,
        item: &ItemPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offered = self.name.read(cx).value().to_string();
        if let Some(d) = form_drafts::example(root, item, cx) {
            self.name
                .update(cx, |i, cx| i.set_value(d.name, window, cx));
            self.description
                .update(cx, |i, cx| i.set_value(d.description, window, cx));
        }
        self.draft = Some((root.to_owned(), item.clone(), offered));
    }

    // A form still showing only the offered name keeps nothing.
    fn store_draft(&self, cx: &mut App) {
        let Some((root, item, offered)) = &self.draft else {
            return;
        };
        let name = self.name.read(cx).value().to_string();
        let description = self.description.read(cx).value().to_string();
        let kept = (name != *offered || !description.trim().is_empty())
            .then_some(ExampleDraft { name, description });
        form_drafts::set_example(root, item, kept, cx);
    }

    /// The name input.
    pub fn name_input(&self) -> &Entity<InputState> {
        &self.name
    }

    /// The description input.
    pub fn description_input(&self) -> &Entity<InputState> {
        &self.description
    }

    /// Trimmed name and description; `None` while the name is blank.
    pub fn answer(&self, cx: &App) -> Option<ExampleAnswer> {
        let name = self.name.read(cx).value().trim().to_owned();
        (!name.is_empty()).then(|| ExampleAnswer {
            name,
            description: self.description.read(cx).value().trim().to_owned(),
            keep_secrets: false,
        })
    }

    /// The line naming the secret values found, `None` when none were.
    pub fn secrets_note(&self) -> Option<String> {
        match &self.check {
            SecretCheck::Found(labels) if !labels.is_empty() => Some(format!(
                "Contains secret values: {}. Saving replaces them with placeholders.",
                labels.join(", ")
            )),
            SecretCheck::FoundInBinary(labels) => Some(binary_secrets_note(labels)),
            _ => None,
        }
    }

    // Adds or removes the note's tip in settings.json; the store saves it on its own schedule.
    fn set_dismissed(&mut self, on: bool, cx: &mut Context<Self>) {
        self.dismissed = on;
        if let Some(app) = cx.try_global::<AppState>() {
            let settings = app.settings.clone();
            settings.update(cx, |p, cx| {
                p.update(cx, |s| {
                    s.dismissed_tips.retain(|t| t != NOTE_TIP);
                    if on {
                        s.dismissed_tips.push(NOTE_TIP.to_owned());
                    }
                })
            });
        }
        cx.notify();
    }
}

impl Render for SaveExampleForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let finding = match (&self.check, self.secrets_note()) {
            (_, Some(note)) => Some(("example-secrets", note)),
            (SecretCheck::Unchecked(reason), None) => Some((
                "example-unchecked",
                format!("Could not check this example for secret values: {reason}"),
            )),
            _ => None,
        };
        v_flex()
            .id("save-example-form")
            .gap(SPACE_2)
            .child(field("save-example-name", "Name", &self.name))
            .child(field(
                "save-example-description",
                "Description",
                &self.description,
            ))
            .children(finding.map(|(id, text)| {
                div()
                    .id(id)
                    .test_support()
                    .text_color(cx.theme().warning)
                    .child(text)
            }))
            .when(self.show_note, |form| {
                form.child(
                    div()
                        .id("save-example-note")
                        .test_support()
                        .text_color(cx.theme().muted_foreground)
                        .child(NOTE),
                )
                .child(
                    ui::checkbox("save-example-note-dismiss", self.dismissed)
                        .label("Don't show again")
                        .on_click(
                            cx.listener(|this, on: &bool, _, cx| this.set_dismissed(*on, cx)),
                        ),
                )
            })
    }
}

/// Opens "Save response as example" over `window` (root id `save-example-dialog`, buttons
/// `save-example-cancel` / `save-example-save`, plus `example-keep-secrets` when secrets were
/// found); yields the answer on Save, Enter or Keep values, `None` on Cancel or Escape.
pub fn ask(
    form: Entity<SaveExampleForm>,
    window: &mut Window,
    cx: &mut App,
) -> async_channel::Receiver<Option<ExampleAnswer>> {
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
            let keep = builder_form.read(cx).secrets_note().is_some().then(|| {
                let (tx, form) = (tx.clone(), builder_form.clone());
                Button::new("example-keep-secrets")
                    .label("Keep values")
                    .lock(answer.is_none(), cx)
                    .on_click(move |_, window, cx| {
                        if let Some(answer) = form.read(cx).answer(cx) {
                            let _ = tx.try_send(Some(ExampleAnswer {
                                keep_secrets: true,
                                ..answer
                            }));
                            window.close_dialog(cx);
                        }
                    })
            });
            let buttons = h_flex()
                .key_context(ui::DIALOG_FOOTER_CONTEXT)
                .gap_2()
                .justify_end()
                .children(keep)
                .child(Button::new("save-example-cancel").label("Cancel").on_click(
                    move |_, window, cx| {
                        let _ = click_tx.try_send(None);
                        window.close_dialog(cx);
                    },
                ))
                .child(
                    Button::new("save-example-save")
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
                .title("Save response as example")
                .close_button(false)
                .on_ok(move |_, _, cx| on_ok(cx))
                .on_cancel(move |_, _, _| {
                    let _ = cancel_tx.try_send(None);
                    true
                })
                .child(
                    v_flex()
                        .id("save-example-dialog")
                        .test_support()
                        .gap(SPACE_3)
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

/// Every secret value `entry`'s request knows (drafts and the selected environment included,
/// as Send plans it) plus the `derived` values of its send, for redacting what leaves the app.
/// The error says why the request could not be checked, in which case nothing may be written.
pub fn request_secrets(
    workspace: &Entity<Workspace>,
    entry: &TabEntry,
    derived: &[String],
    cx: &mut App,
) -> impl Future<Output = Result<Vec<NamedSecret>, String>> + use<> {
    let item = entry.item().cloned();
    let root = entry.root.clone();
    let (reply, planned) = async_channel::bounded(1);
    let sent = item.and_then(|item| {
        let command = Command::Plan { item, reply };
        workspace.update(cx, |w, cx| w.send(&root, command, cx))
    });
    let derived = derived.to_vec();
    async move {
        let planned = match sent.map(|_| planned) {
            None => return Err("the collection is not open".to_owned()),
            Some(planned) => planned.recv().await,
        };
        match planned {
            Ok(Ok(planned)) => match planned.plan.requests().next() {
                Some(file) => core_request_secrets(file, &planned.context, &derived)
                    .map_err(|e| e.to_string()),
                None => Err("there is no request to check".into()),
            },
            Ok(Err(e)) => Err(plan_message(&e)),
            Err(_) => Err("the collection was closed".into()),
        }
    }
}

/// Checks `response` against every secret value `item`'s context knows (drafts and the
/// selected environment included, as Send plans it), asks for a name, then adds the example to the
/// request `entry` shows by then (the one it showed, once it has closed) with the secrets replaced
/// unless the user twice chose to keep them; a tab that is not a request tab does nothing.
pub fn save_as_example(
    workspace: &Entity<Workspace>,
    entry: &TabEntry,
    response: ExampleResponse,
    initial: String,
    derived: &[String],
    window: &mut Window,
    cx: &mut App,
) {
    let Some(item) = entry.item().cloned() else {
        return;
    };
    let (root, tab) = (entry.root.as_path(), entry.id);
    let secrets = request_secrets(workspace, entry, derived, cx);
    let (workspace, root) = (workspace.clone(), root.to_path_buf());
    window
        .spawn(cx, async move |cx| {
            let secrets = secrets.await;
            let check = match &secrets {
                Ok(secrets) => {
                    let labels = secrets_in(&response, secrets);
                    if binary_secrets(&response, secrets).is_empty() {
                        SecretCheck::Found(labels)
                    } else {
                        SecretCheck::FoundInBinary(labels)
                    }
                }
                Err(reason) => SecretCheck::Unchecked(reason.clone()),
            };
            let Ok(answer) = cx.update(|window, cx| {
                let form = cx.new(|cx| {
                    let mut form = SaveExampleForm::new(&initial, check, window, cx);
                    form.keep_draft(&root, &item, window, cx);
                    form
                });
                ask(form, window, cx)
            }) else {
                return;
            };
            let Ok(Some(answer)) = answer.recv().await else {
                return;
            };
            let mut response = response;
            let mut description = answer.description;
            let mut withheld = false;
            if answer.keep_secrets {
                if !confirm_keep(cx).await {
                    return;
                }
            } else {
                // Masking needs no secrets, so it also runs when the lookup failed; a withheld
                // body is named in the description.
                let scrubbed = response.scrub(secrets.as_deref().ok());
                withheld = scrubbed.withheld;
                description = scrubbed.description(&description);
            }
            let draft_key = item.clone();
            // The request may have been renamed while the dialog was open; its tab follows the
            // rename, so read the item back from there unless the tab has closed.
            let item = workspace
                .read_with(cx, |w, _| w.tabs().get(tab).and_then(|t| t.item().cloned()))
                .unwrap_or(item);
            let op = TreeOp::AddExample {
                item,
                name: answer.name,
                description,
                response,
            };
            let sent = workspace.update(cx, |w, cx| w.send(&root, Command::Tree(op), cx));
            if withheld {
                cx.update(|window, cx| {
                    window.push_notification(ui::info_toast(WITHHELD_TOAST), cx)
                })
                .ok();
            }
            if let Some(id) = sent {
                drop_draft_once_added(&workspace, root, draft_key, id, cx).await;
            }
        })
        .detach();
}

// Drops the example draft once command `id` reports the example written; a failed write or a
// closed collection keeps it.
async fn drop_draft_once_added(
    workspace: &Entity<Workspace>,
    root: PathBuf,
    item: ItemPath,
    id: u64,
    cx: &mut AsyncWindowContext,
) {
    let (tx, rx) = async_channel::bounded(1);
    let watched = cx.update(|_, cx| {
        let (done, closed) = (tx.clone(), tx);
        let (at, open) = (root.clone(), root.clone());
        let notices = cx.subscribe(workspace, move |_, e: &WorkspaceEvent, _| {
            if let WorkspaceEvent::Notices { root, notices, .. } = e
                && *root == at
                && let Some(n) = notices.iter().find(|n| n.command == id)
            {
                let added = matches!(n.kind, NoticeKind::ExampleAdded { .. });
                done.try_send(added).ok();
            }
        });
        let closing = cx.observe(workspace, move |w, cx| {
            if w.read(cx).snapshot(&open, cx).is_none() {
                closed.try_send(false).ok();
            }
        });
        [notices, closing]
    });
    let Ok(_watched) = watched else {
        return;
    };
    if rx.recv().await == Ok(true) {
        cx.update(|_, cx| form_drafts::set_example(&root, &item, None, cx))
            .ok();
    }
}

// Secret values reach a committed file only on an explicit yes; Cancel is the default.
async fn confirm_keep(cx: &mut AsyncWindowContext) -> bool {
    let ask = Ask {
        title: "Save secrets?".into(),
        detail: Some("This example keeps secret values in plain text in the request file.".into()),
        choices: vec![
            Choice {
                id: "confirm-save-secrets",
                label: "Save with secrets".into(),
                danger: true,
            },
            Choice {
                id: "confirm-cancel",
                label: "Cancel".into(),
                danger: false,
            },
        ],
        default: 1,
        cancel: 1,
    };
    let Ok(answer) = cx.update(|window, cx| confirm::ask(ask, window, cx)) else {
        return false;
    };
    answer.recv().await == Ok(0)
}
