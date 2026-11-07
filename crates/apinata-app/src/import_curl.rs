//! The Import cURL dialog: a pasted curl command becomes a new draft at the collection root.

use std::path::PathBuf;

use apinata_core::import::{ImportedRequest, RequestFormat, parse_request};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, Focusable, Result, SharedString, TestSupportExt, Window, div};

use crate::style::{self, SPACE_2};
use crate::ui::{self, FocusNextControl};
use crate::workspace::Workspace;

/// The error shown when the text does not start with `curl`.
const NOT_CURL: &str = "Not a curl command. Paste one that starts with curl.";

/// A pasted command whose body is a GraphQL query becomes a GraphQL request; any other stays HTTP.
pub fn graphql_or_http(request: ImportedRequest) -> ImportedRequest {
    request.as_graphql().unwrap_or(request)
}

/// The dialog's body: the command editor and the last parse error.
struct ImportCurlForm {
    editor: Entity<EditorState>,
    error: Option<SharedString>,
}

impl ImportCurlForm {
    // The parsed command, or the error to show; only a detected curl command is parsed.
    fn parse(&self, cx: &App) -> Result<ImportedRequest, SharedString> {
        let text = self.editor.read(cx).value().to_string();
        if RequestFormat::detect(&text).is_none() {
            return Err(NOT_CURL.into());
        }
        parse_request(None, &text)
            .map(|parsed| graphql_or_http(parsed.request))
            .map_err(|e| e.to_string().into())
    }
}

impl Render for ImportCurlForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let error = self.error.clone().map(|message| {
            div()
                .id("import-curl-error")
                .test_support()
                .text_color(cx.theme().danger)
                .child(message)
        });
        v_flex()
            .gap(SPACE_2)
            .child(
                div()
                    .id("import-curl-input")
                    .test_support()
                    .w_full()
                    .h(style::EDITOR_MIN_HEIGHT)
                    .child(ui::ringed_editor(
                        Editor::new(&self.editor).h_full(),
                        &self.editor,
                    )),
            )
            .children(error)
    }
}

/// Opens the "Import cURL" dialog over `window` for the collection at `root`. Import parses
/// the command into a new draft and closes; an error shows inline and keeps the dialog open.
pub fn open(workspace: &Entity<Workspace>, root: PathBuf, window: &mut Window, cx: &mut App) {
    let editor = cx.new(|cx| EditorState::new(window, cx).language("text"));
    let form = cx.new(|_| ImportCurlForm {
        editor: editor.clone(),
        error: None,
    });
    let workspace = workspace.clone();
    // Enter never closes it: the editor takes Enter as a newline and only the button imports,
    // so Enter elsewhere must not discard the pasted command.
    window.open_dialog(cx, move |dialog, window, cx| {
        let dialog = crate::dialog_frame::place(dialog, window);
        let (form_ok, workspace, root) = (form.clone(), workspace.clone(), root.clone());
        let buttons = ui::dialog_footer(
            Some(
                Button::new("import-curl-cancel")
                    .label("Cancel")
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            ),
            [],
            Button::new("import-curl-import")
                .label("Import")
                .primary()
                .on_click(move |_, window, cx| match form_ok.read(cx).parse(cx) {
                    Ok(request) => {
                        workspace.update(cx, |w, cx| w.new_draft_from_import(&root, request, cx));
                        window.close_dialog(cx);
                    }
                    Err(error) => form_ok.update(cx, |f, cx| {
                        f.error = Some(error);
                        cx.notify();
                    }),
                }),
        );
        dialog
            .title("Import cURL")
            .close_button(false)
            .on_ok(|_, _, _| false)
            .child(
                v_flex()
                    .id("import-curl-dialog")
                    .test_support()
                    // Ctrl+Tab from the editor lands on Import, so Ctrl+Tab then Enter imports:
                    // the first queued Tab reaches Cancel and the second steps past it to Import.
                    .capture_action({
                        let editor = form.read(cx).editor.clone();
                        move |_: &FocusNextControl, window, cx| {
                            if editor.read(cx).focus_handle(cx).is_focused(window) {
                                cx.stop_propagation();
                                ui::leave_editor(true, window, cx);
                                ui::leave_editor(true, window, cx);
                            }
                        }
                    })
                    .gap(SPACE_2)
                    .child(form.clone())
                    .child(buttons),
            )
    });
    // The dialog takes focus as it opens; the editor takes it back, ready for the paste.
    editor.read(cx).focus_handle(cx).focus(window, cx);
}
