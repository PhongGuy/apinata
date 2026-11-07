//! The collection's Presets section: the request type and URL a new request starts with.

use apinata_core::CollectionPresets;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, EventEmitter, Subscription, Window};

use crate::command::Edit;
use crate::section::SectionEdit;
use crate::style;
use crate::ui;

/// The request types the app can create, as segment labels and the values they write.
const TYPES: [(&str, &str); 2] = [("HTTP", "http"), ("GraphQL", "graphql")];

/// The Presets section.
pub struct PresetsSection {
    url: Entity<InputState>,
    // The type as the file holds it. A value the app cannot create, such as `grpc`, shows no
    // segment and is kept until a segment is picked.
    request_type: String,
    _sub: Subscription,
}

impl EventEmitter<SectionEdit> for PresetsSection {}

impl PresetsSection {
    /// An empty section.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("https://api.example.com"));
        let sub = cx.subscribe(&url, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.emit(cx);
            }
        });
        PresetsSection {
            url,
            request_type: String::new(),
            _sub: sub,
        }
    }

    /// Shows `presets` from a snapshot without emitting an edit. The URL text is replaced only
    /// when it differs, since a replacement resets the caret and an echo of typing must not.
    pub fn load(
        &mut self,
        presets: &CollectionPresets,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_type.clone_from(&presets.request_type);
        crate::ui::sync_input_text(&self.url, &presets.request_url, window, cx);
        cx.notify();
    }

    /// The URL typed.
    pub fn url(&self, cx: &App) -> String {
        self.url.read(cx).value().to_string()
    }

    /// The URL field.
    pub fn url_input(&self) -> &Entity<InputState> {
        &self.url
    }

    /// The request type held.
    pub fn request_type(&self) -> String {
        self.request_type.clone()
    }

    /// Whether `edit`, which this section sent, was built from presets a load has since
    /// replaced: the type or URL it would send now differs.
    pub fn lags(&self, edit: &Edit, cx: &App) -> bool {
        matches!(edit, Edit::Presets(sent) if *sent != self.current(cx))
    }

    fn current(&self, cx: &App) -> CollectionPresets {
        CollectionPresets::new(self.request_type.clone(), self.url(cx))
    }

    fn pick(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some((_, value)) = TYPES.get(ix) {
            self.request_type = (*value).into();
            self.emit(cx);
        }
    }

    fn emit(&mut self, cx: &mut Context<Self>) {
        cx.emit(SectionEdit::new(Edit::Presets(self.current(cx))));
        cx.notify();
    }
}

impl Render for PresetsSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        // An empty type means HTTP, as the reference reader treats it.
        let selected = match self.request_type.as_str() {
            "" => 0,
            other => TYPES
                .iter()
                .position(|(_, value)| *value == other)
                .unwrap_or(usize::MAX),
        };
        let types = ui::segmented(
            "presets-type",
            TYPES.map(|(label, _)| label),
            selected,
            move |ix, _, cx| this.update(cx, |s, cx| s.pick(*ix, cx)),
        );
        v_flex()
            .gap(style::SPACE_2)
            .child(ui::form_row("presets-type-row", "Request type", types))
            .child(
                ui::form_row(
                    "presets-url-row",
                    "Base URL",
                    ui::padded_input(
                        Input::new(&self.url)
                            .id("presets-url")
                            .w(style::WIDE_INPUT_WIDTH)
                            .bg(ui::field_fill(false, cx)),
                        &self.url,
                    ),
                )
                .hint(
                    "presets-url-note",
                    "New requests in this collection start with this URL.",
                ),
            )
    }
}
