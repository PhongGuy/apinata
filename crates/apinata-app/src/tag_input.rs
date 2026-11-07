//! A compact chip input for a list of tags: chips in a wrapping row, each with a remove
//! control, and a text input after them.

use apinata_core::normalize_tags;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Entity, EventEmitter, Focusable, KeyDownEvent, Subscription, TestSupportExt, Window, div,
};

use crate::style::{self, FORM_INPUT_WIDTH, RADIUS, SPACE_1};
use crate::ui;

/// The chips and the text typed after them, without a window. Every method that can change
/// the chips returns whether it did, so the caller emits an edit only for a real change.
#[derive(Debug, Default, Clone)]
pub struct TagChips {
    chips: Vec<String>,
    pending: String,
}

impl TagChips {
    /// The committed tags, normalised.
    pub fn chips(&self) -> &[String] {
        &self.chips
    }

    /// The text typed and not yet committed.
    pub fn pending(&self) -> &str {
        &self.pending
    }

    /// Replaces the chips with the normalised `tags`, as a snapshot shows them, and leaves the
    /// text being typed alone.
    pub fn load(&mut self, tags: Vec<String>) {
        self.chips = normalize_tags(tags);
    }

    /// The input's whole text after a keystroke: each part before a comma becomes a chip and
    /// the last part stays pending.
    pub fn type_text(&mut self, text: &str) -> bool {
        let mut parts: Vec<&str> = text.split(',').collect();
        let rest = parts.pop().unwrap_or_default();
        let mut changed = false;
        for part in parts {
            changed |= self.commit(part);
        }
        self.pending = rest.to_owned();
        changed
    }

    /// Enter: the pending text becomes a chip.
    pub fn enter(&mut self) -> bool {
        let text = std::mem::take(&mut self.pending);
        self.commit(&text)
    }

    /// The input lost focus: the pending text becomes a chip.
    pub fn blur(&mut self) -> bool {
        self.enter()
    }

    /// Backspace: with nothing pending, the last chip goes.
    pub fn backspace(&mut self) -> bool {
        if !self.pending.is_empty() {
            return false;
        }
        self.chips.pop().is_some()
    }

    /// Removes the chip at `ix`.
    pub fn remove(&mut self, ix: usize) -> bool {
        if ix >= self.chips.len() {
            return false;
        }
        self.chips.remove(ix);
        true
    }

    // Adds `text` unless it is blank or already a chip.
    fn commit(&mut self, text: &str) -> bool {
        let mut next = self.chips.clone();
        next.push(text.to_owned());
        let next = normalize_tags(next);
        let changed = next.len() != self.chips.len();
        self.chips = next;
        changed
    }
}

/// The tags after a change the user made.
pub struct TagsChanged(pub Vec<String>);

/// The chip input view; it emits [`TagsChanged`] for the user's edits and never for `load`.
pub struct TagField {
    model: TagChips,
    input: Entity<InputState>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<TagsChanged> for TagField {}

impl TagField {
    /// An empty chip input.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Add a tag"));
        let sub = cx.subscribe_in(&input, window, |this, state, e: &InputEvent, window, cx| {
            let changed = match e {
                InputEvent::Change => {
                    let text = state.read(cx).value().to_string();
                    let changed = this.model.type_text(&text);
                    // Only a comma moved text into a chip; the input keeps the remainder.
                    if text.contains(',') {
                        this.show_pending(window, cx);
                    }
                    changed
                }
                InputEvent::PressEnter { .. } => this.model.enter(),
                InputEvent::Blur => this.model.blur(),
                _ => return,
            };
            if !matches!(e, InputEvent::Change) {
                this.show_pending(window, cx);
            }
            if changed {
                cx.emit(TagsChanged(this.model.chips().to_vec()));
            }
            cx.notify();
        });
        TagField {
            model: TagChips::default(),
            input,
            _subs: vec![sub],
        }
    }

    /// The committed tags.
    pub fn tags(&self) -> &[String] {
        self.model.chips()
    }

    /// Whether `sent`, which this field emitted, was built from tags a load has since
    /// replaced.
    pub fn lags(&self, sent: &[String]) -> bool {
        sent != self.tags()
    }

    /// The text typed and not yet committed.
    pub fn pending(&self) -> &str {
        self.model.pending()
    }

    /// Shows `tags` from a snapshot without emitting an edit.
    pub fn load(&mut self, tags: &[String], cx: &mut Context<Self>) {
        self.model.load(tags.to_vec());
        cx.notify();
    }

    /// Removes the chip at `ix` and emits the new tags.
    pub fn remove(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.model.remove(ix) {
            cx.emit(TagsChanged(self.model.chips().to_vec()));
            cx.notify();
        }
    }

    /// The input's input state, for focusing it and typing in tests.
    pub fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    fn show_pending(&self, window: &mut Window, cx: &mut Context<Self>) {
        crate::kv_table::set_text(&self.input, self.model.pending(), window, cx);
    }

    // Backspace in an empty input takes the last chip back, as in a token field.
    fn key_down(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let empty = self.input.read(cx).value().is_empty();
        if e.keystroke.key == "backspace"
            && !e.keystroke.modifiers.modified()
            && empty
            && self.model.backspace()
        {
            cx.emit(TagsChanged(self.model.chips().to_vec()));
            cx.notify();
        }
    }
}

impl Render for TagField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let ringed = self.input.focus_handle(cx).is_focused(window);
        let chips = self.model.chips().iter().enumerate().map(|(ix, tag)| {
            h_flex()
                .id(("tag-chip", ix))
                .test_support()
                .items_center()
                .gap(SPACE_1)
                .pl(style::SPACE_2)
                .rounded(RADIUS)
                .bg(theme.secondary)
                .text_size(style::TEXT_SMALL)
                .child(tag.clone())
                .child(
                    ui::icon_button(("tag-remove", ix), IconName::Close, "Remove tag", None)
                        .tab_stop(false)
                        .on_click(cx.listener(move |t, _, _, cx| t.remove(ix, cx))),
                )
        });
        h_flex()
            .id("tag-input")
            .test_support()
            .relative()
            .w_full()
            .flex_wrap()
            .items_center()
            .gap(SPACE_1)
            .p(SPACE_1)
            .border_1()
            .border_color(theme.border)
            .rounded(RADIUS)
            .bg(ui::field_fill(false, cx))
            .capture_key_down(cx.listener(|this, e, _, cx| this.key_down(e, cx)))
            .children(chips)
            // The frame is the input's only edge, so focus inside it rings the frame.
            .children(ringed.then(|| {
                div()
                    .absolute()
                    .inset_0()
                    .rounded(RADIUS)
                    .border_2()
                    .border_color(crate::focus_mode::accent_ring(theme))
            }))
            .child(
                div().min_w(FORM_INPUT_WIDTH).flex_1().child(
                    ui::padded_input(
                        Input::new(&self.input)
                            .id("tag-text")
                            .appearance(false)
                            .bg(ui::field_fill(false, cx)),
                        &self.input,
                    )
                    .frameless(),
                ),
            )
    }
}
