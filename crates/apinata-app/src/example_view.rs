//! An example tab: a saved example's request and response, read-only.

use std::path::PathBuf;
use std::sync::Arc;

use apinata_core::{
    BodySpec, Example, ExampleRequest, ExampleResponse, ItemPath, MultipartValue, Outcome,
    RequestResult,
};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Div, Entity, FocusHandle, Focusable, ScrollHandle, Subscription, TestSupportExt, Window,
    div,
};

use crate::response_pane::{ResponsePane, header_table};
use crate::send_model::SendModel;
use crate::snapshot::RequestView;
use crate::style;
use crate::tabs::{TabId, TabKind};
use crate::workspace::Workspace;

/// The view of one example tab; it follows its tab, so an index shifted by a delete or a
/// request moved by a rename keeps showing the same example.
pub struct ExampleView {
    workspace: Entity<Workspace>,
    tab: TabId,
    send: Entity<SendModel>,
    pane: Entity<ResponsePane>,
    request_scroll: ScrollHandle,
    shown: Option<(String, Option<ExampleResponse>)>,
    // The tab's Tab stop: the first thing focus lands on, ahead of the controls.
    focus: FocusHandle,
    _observer: Subscription,
}

impl ExampleView {
    /// Shows the example tab `tab` of `workspace`.
    pub fn new(
        workspace: Entity<Workspace>,
        tab: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The pane reads a send model that never sends; it only shows the saved response.
        let send = cx.new(|_| SendModel::new(workspace.clone()));
        let pane = cx.new(|cx| {
            ResponsePane::new(send.clone(), window, cx)
                .without_retry()
                .without_save_example()
                .without_elapsed()
                .without_exchange_tabs()
                .without_compare()
                .without_body_edit(cx)
        });
        let observer = cx.observe(&workspace, |this, _, cx| this.sync(cx));
        let mut view = ExampleView {
            workspace,
            tab,
            send,
            pane,
            request_scroll: ScrollHandle::new(),
            shown: None,
            focus: cx.focus_handle().tab_stop(true),
            _observer: observer,
        };
        view.sync(cx);
        view
    }

    /// The never-sending model the response pane shows.
    pub fn send_model(&self) -> &Entity<SendModel> {
        &self.send
    }

    // The tab's collection root, the request view holding its example, and the example's index.
    fn target(&self, cx: &App) -> Option<(PathBuf, ItemPath, Arc<RequestView>, usize)> {
        let ws = self.workspace.read(cx);
        let entry = ws.tabs().get(self.tab)?;
        let TabKind::Example { item, index } = &entry.kind else {
            return None;
        };
        let view = ws.request(&entry.root, item, cx)?;
        (*index < view.examples.len()).then(|| (entry.root.clone(), item.clone(), view, *index))
    }

    // Re-shows the saved response only when the example's name or response changed, so an
    // unsaved edit to its request or an index shift leaves the pane's state alone.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let Some((root, item, view, index)) = self.target(cx) else {
            return;
        };
        let example = &view.examples[index];
        let same = self
            .shown
            .as_ref()
            .is_some_and(|(name, response)| *name == example.name && *response == example.response);
        if !same {
            if let Some(response) = &example.response {
                let result = RequestResult::from_parts(
                    item,
                    example.name.clone(),
                    Outcome::Passed,
                    Some(response.to_response()),
                    Vec::new(),
                );
                self.send
                    .update(cx, |m, cx| m.show_at(&root, Arc::new(result), cx));
            }
            self.shown = Some((example.name.clone(), example.response.clone()));
        }
        cx.notify();
    }
}

/// The body as text: JSON, XML or text as stored, form fields as `name: value` lines, file
/// paths one per line, and "No body" for none.
fn body_text(body: &BodySpec) -> String {
    let lines = |pairs: Vec<String>| pairs.join("\n");
    match body {
        BodySpec::Json(t) | BodySpec::Xml(t) | BodySpec::Text(t) | BodySpec::Sparql(t) => t.clone(),
        BodySpec::FormUrlEncoded(fields) => lines(
            fields
                .iter()
                .map(|f| format!("{}: {}", f.name, f.value))
                .collect(),
        ),
        BodySpec::Multipart(fields) => lines(
            fields
                .iter()
                .map(|f| match &f.value {
                    MultipartValue::Text(t) => format!("{}: {t}", f.name),
                    MultipartValue::Files(paths) => format!("{}: {}", f.name, paths.join(", ")),
                    _ => format!("{}:", f.name),
                })
                .collect(),
        ),
        BodySpec::File(files) => lines(files.iter().map(|f| f.path.clone()).collect()),
        BodySpec::Unsupported { kind } => format!("A {kind} body"),
        _ => "No body".into(),
    }
}

// The request block: method and URL on one line, then params and headers when present,
// then the body.
fn request_block(request: &ExampleRequest, cx: &App) -> Div {
    let mono = cx.theme().mono_font_family.clone();
    let muted = cx.theme().muted_foreground;
    let table = |id: &'static str, ids, pairs: Vec<(String, String)>| {
        (!pairs.is_empty()).then(|| {
            v_flex()
                .id(id)
                .test_support()
                .gap(style::SPACE_1)
                .child(header_table(ids, &pairs))
        })
    };
    let params = request
        .params
        .iter()
        .map(|p| (p.name.clone(), p.value.clone()))
        .collect();
    let headers = request
        .headers
        .iter()
        .map(|h| (h.name.clone(), h.value.clone()))
        .collect();
    let line = h_flex()
        .gap(style::SPACE_2)
        .font_family(mono.clone())
        .child(
            div()
                .id("example-method")
                .test_support()
                .child(request.method.clone()),
        )
        .child(
            div()
                .id("example-url")
                .test_support()
                .min_w_0()
                .child(request.url.clone()),
        );
    let body = div()
        .id("example-body")
        .test_support()
        .font_family(mono)
        .whitespace_normal()
        .child(body_text(&request.body));
    v_flex()
        .gap(style::SPACE_2)
        .child(line)
        .children(table(
            "example-params",
            ["example-param", "example-param-name", "example-param-value"],
            params,
        ))
        .children(table(
            "example-headers",
            [
                "example-header",
                "example-header-name",
                "example-header-value",
            ],
            headers,
        ))
        .child(div().text_color(muted).child("Body"))
        .child(body)
}

impl Render for ExampleView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Below the room for the header, the request and a usable response, the view scrolls
        // as one page rather than squeezing the response away.
        let root = v_flex()
            .id("example-view")
            .test_support()
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .gap(style::SPACE_2)
            .p(style::PANE_INSET)
            .pl(style::tab_content_inset(window))
            .pt(style::tab_content_inset(window));
        let Some((_, _, view, index)) = self.target(cx) else {
            return root;
        };
        let muted = cx.theme().muted_foreground;
        let example: &Example = &view.examples[index];
        let header = v_flex()
            .gap(style::SPACE_1)
            .child(
                div()
                    .text_size(style::TEXT_TITLE)
                    .child(example.name.clone()),
            )
            .children(
                example
                    .description
                    .clone()
                    .map(|d| div().text_color(muted).child(d)),
            );
        let request = match &example.request {
            Some(request) => request_block(request, cx),
            None => div().text_color(muted).child("No request saved"),
        };
        let response = match example.response {
            Some(_) => div()
                .flex_1()
                .min_h(style::PANE_MIN)
                .child(self.pane.clone())
                .into_any_element(),
            None => div()
                .id("example-no-response")
                .test_support()
                .text_color(muted)
                .child("No response saved")
                .into_any_element(),
        };
        root.child(header)
            .child(
                // A plain scroll box with a tracked handle: the kit's scrollbar wrapper would
                // clamp the content to max_h too, leaving nothing to scroll. Auto height keeps
                // a short request from reserving the response's room.
                div()
                    .relative()
                    .flex_none()
                    .child(
                        div()
                            .id("example-request")
                            .test_support()
                            .max_h(style::EDITOR_HEIGHT)
                            .overflow_y_scroll()
                            .track_scroll(&self.request_scroll)
                            .child(request),
                    )
                    .vertical_scrollbar(&self.request_scroll),
            )
            .child(response)
    }
}

impl Focusable for ExampleView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
