//! `{{name}}` placeholders in text fields: a tint per placeholder by what it resolves to,
//! and a tooltip with its scope and value while the pointer is over it.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use apinata_core::{VarScope, VariableInfo, VariableSet, placeholders, prompt_label};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{EditorState, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, ContentMask, ElementId, Entity, Focusable, MouseMoveEvent, Pixels,
    SharedString, StyleRefinement, TestSupportExt, Window, anchored, canvas, deferred, div, fill,
};

use crate::header_complete::Hint;
use crate::style;
use crate::theme::{VarKind, var_tint};
use crate::var_complete;

/// What a secret's tooltip shows in place of its value.
pub const SECRET_DOTS: &str = "••••••";
/// The tooltip on a `{{?label}}` prompt.
pub const PROMPT_TIP: &str = "Prompt · asked on send";
/// The longest value a tooltip shows before cutting it with an ellipsis, in characters.
pub const TIP_VALUE_CHARS: usize = 120;

/// One placeholder in a field's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Byte range of `{{...}}`, braces included.
    pub range: Range<usize>,
    /// What it resolves to.
    pub kind: VarKind,
    /// The tooltip text.
    pub tip: String,
}

/// Every placeholder in `text`, classified against `vars`.
pub fn tokens(text: &str, vars: &VariableSet) -> Vec<Token> {
    placeholders(text)
        .into_iter()
        .map(|(range, name)| {
            // A valid prompt is asked for on send, so no scope's value applies to it.
            if prompt_label(name).is_some() {
                return Token {
                    range,
                    kind: VarKind::Prompt,
                    tip: PROMPT_TIP.to_owned(),
                };
            }
            let info = vars.get(name);
            let kind = match &info {
                None => VarKind::Undefined,
                Some(i) if i.scope() == VarScope::Dynamic => VarKind::Dynamic,
                Some(_) => VarKind::Defined,
            };
            Token {
                range,
                kind,
                tip: tooltip(info.as_ref()),
            }
        })
        .collect()
}

/// The label for a scope in a tooltip.
pub fn scope_label(scope: VarScope) -> &'static str {
    match scope {
        VarScope::Collection => "Collection",
        VarScope::Environment => "Environment",
        VarScope::GlobalEnvironment => "Global environment",
        VarScope::Folder => "Folder",
        VarScope::Request => "Request",
        VarScope::IterationData => "Iteration data",
        VarScope::Runtime => "Runtime",
        VarScope::ProcessEnv => "process.env",
        VarScope::Dynamic => "Dynamic",
        VarScope::OAuth2 => "OAuth 2.0",
        _ => "Variable",
    }
}

/// A placeholder's tooltip: scope and value, dots for a secret, or why it fails.
pub fn tooltip(info: Option<&VariableInfo>) -> String {
    let Some(info) = info else {
        return "Undefined variable".to_owned();
    };
    let scope = scope_label(info.scope());
    if info.scope() == VarScope::Dynamic {
        return format!("{scope} · new value on each send");
    }
    if info.is_secret() {
        return format!("{scope} · {SECRET_DOTS}");
    }
    let value = info.value().unwrap_or_default();
    if value.is_empty() {
        return format!("{scope} · (empty)");
    }
    let mut shown: String = value.chars().take(TIP_VALUE_CHARS).collect();
    if value.chars().count() > TIP_VALUE_CHARS {
        shown.push('…');
    }
    format!("{scope} · {shown}")
}

/// Where a [`VarField`] reads its text and geometry.
#[derive(Clone)]
pub enum Source {
    /// A single-line input.
    Input(Entity<InputState>),
    /// A code editor.
    Editor(Entity<EditorState>),
}

/// The tokens last computed for one field, kept until its text, its variable set or the
/// set's runtime revision changes, so frames and mouse moves do not re-resolve them.
struct TokenCache {
    text: SharedString,
    vars: Arc<VariableSet>,
    revision: u64,
    tokens: Rc<[Token]>,
}

/// Per-field state: the hovered token and the token cache.
#[derive(Default)]
struct FieldState {
    hovered: Option<Token>,
    cache: Option<TokenCache>,
}

/// The tokens of `text` against `vars`, from `state`'s cache when it is still fresh.
fn cached_tokens(
    state: &Entity<FieldState>,
    text: &SharedString,
    vars: &Arc<VariableSet>,
    cx: &mut App,
) -> Rc<[Token]> {
    let revision = vars.revision();
    if let Some(c) = &state.read(cx).cache
        && c.text == *text
        && Arc::ptr_eq(&c.vars, vars)
        && c.revision == revision
    {
        return c.tokens.clone();
    }
    let found: Rc<[Token]> = tokens(text, vars).into();
    state.update(cx, |s, _| {
        s.cache = Some(TokenCache {
            text: text.clone(),
            vars: vars.clone(),
            revision,
            tokens: found.clone(),
        })
    });
    found
}

impl Source {
    pub(crate) fn text(&self, cx: &App) -> SharedString {
        match self {
            Source::Input(s) => s.read(cx).value(),
            Source::Editor(s) => s.read(cx).value(),
        }
    }

    /// The cursor while the field has focus and nothing selected.
    pub(crate) fn caret(&self, window: &Window, cx: &App) -> Option<usize> {
        let (focus, selected) = match self {
            Source::Input(s) => (s.focus_handle(cx), s.read(cx).selected_range()),
            Source::Editor(s) => (s.focus_handle(cx), s.read(cx).selected_range()),
        };
        (focus.is_focused(window) && selected.is_empty()).then_some(selected.end)
    }

    /// Replace `range` with `text` as one undoable edit, then put the cursor at `cursor`.
    pub(crate) fn edit(
        &self,
        range: Range<usize>,
        text: String,
        cursor: usize,
        window: &mut Window,
        cx: &mut App,
    ) {
        match self {
            Source::Input(s) => s.update(cx, |s, cx| {
                s.set_selected_range(range, cx);
                s.replace(text, window, cx);
                s.set_selected_range(cursor..cursor, cx);
            }),
            Source::Editor(s) => s.update(cx, |s, cx| {
                s.set_selected_range(range, cx);
                s.replace(text, window, cx);
                s.set_selected_range(cursor..cursor, cx);
            }),
        }
    }

    pub(crate) fn bounds(&self, range: &Range<usize>, cx: &App) -> Option<Bounds<Pixels>> {
        match self {
            Source::Input(s) => s.read(cx).range_to_bounds(range),
            Source::Editor(s) => s.read(cx).range_to_bounds(range),
        }
    }
}

/// A text field wrapped so its placeholders are tinted and explained on hover. Without
/// variables it is the field alone. Hit testing reads the same bounds the tint uses, and
/// the hover listeners take only a non-blocking hitbox, so clicks and drags reach the field.
#[derive(IntoElement)]
pub struct VarField {
    id: ElementId,
    source: Source,
    field: AnyElement,
    vars: Option<Arc<VariableSet>>,
    tint_only: bool,
    hints: Option<Hint>,
    style: StyleRefinement,
}

impl VarField {
    /// A single-line field; its tints are painted over the text.
    pub fn new(
        id: impl Into<ElementId>,
        state: &Entity<InputState>,
        field: impl IntoElement,
        vars: Option<Arc<VariableSet>>,
    ) -> Self {
        Self::with(id, Source::Input(state.clone()), field, vars)
    }

    /// A code editor; it paints its own tints as range decorations, so only hover is added.
    pub fn editor(
        id: impl Into<ElementId>,
        state: &Entity<EditorState>,
        field: impl IntoElement,
        vars: Option<Arc<VariableSet>>,
    ) -> Self {
        Self::with(id, Source::Editor(state.clone()), field, vars)
    }

    fn with(
        id: impl Into<ElementId>,
        source: Source,
        field: impl IntoElement,
        vars: Option<Arc<VariableSet>>,
    ) -> Self {
        VarField {
            id: id.into(),
            source,
            field: field.into_any_element(),
            vars,
            tint_only: false,
            hints: None,
            style: StyleRefinement::default(),
        }
    }

    /// Offers header names or values for the text typed, unless a `{{` is open at the cursor.
    pub fn hints(mut self, hints: Option<Hint>) -> Self {
        self.hints = hints;
        self
    }

    /// Paints the tints only: no hover tooltip and no completion list.
    pub fn tint_only(mut self) -> Self {
        self.tint_only = true;
        self
    }
}

impl Styled for VarField {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for VarField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut root = div().id(self.id.clone()).relative();
        *root.style() = self.style;
        let Some(vars) = self.vars else {
            let root = match &self.hints {
                Some(hint) => {
                    var_complete::attach_hints(root, &self.id, &self.source, hint, window, cx).0
                }
                None => root,
            };
            return root.child(self.field);
        };
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| FieldState::default());
        let text = self.source.text(cx);
        let found = cached_tokens(&state, &text, &vars, cx);
        let tinted = found.clone();
        let fills = match &self.source {
            Source::Input(input) => {
                let input = input.clone();
                Some(
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, cx| {
                            let clip = input.read(cx).input_bounds();
                            window.with_content_mask(
                                Some(ContentMask { bounds: clip }),
                                |window| {
                                    for t in tinted.iter() {
                                        if let Some(b) = input.read(cx).range_to_bounds(&t.range) {
                                            let color = var_tint(t.kind, cx);
                                            window.paint_quad(
                                                fill(b, color).corner_radii(style::RADIUS),
                                            );
                                        }
                                    }
                                },
                            );
                        },
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
            }
            Source::Editor(_) => None,
        };
        // A tint-only field shows its references without explaining or completing them.
        if self.tint_only {
            return root.child(self.field).children(fills);
        }
        let (root, listing) = var_complete::attach(root, &self.id, &self.source, &vars, window, cx);
        // A header list never shows with the variable list: `{{` at the cursor closes it.
        let (root, hinting) = match &self.hints {
            Some(hint) => {
                var_complete::attach_hints(root, &self.id, &self.source, hint, window, cx)
            }
            None => (root, false),
        };
        // A hovered token the text no longer holds is gone; drop its tooltip. The list,
        // while it shows, replaces the tooltip.
        let tip = state
            .read(cx)
            .hovered
            .clone()
            .filter(|t| !listing && !hinting && found.contains(t))
            .and_then(|t| Some((self.source.bounds(&t.range, cx)?, t.tip)));
        let source = self.source.clone();
        let hit_vars = vars.clone();
        let on_move = {
            let state = state.clone();
            move |e: &MouseMoveEvent, _: &mut Window, cx: &mut App| {
                let text = source.text(cx);
                let found = cached_tokens(&state, &text, &hit_vars, cx);
                let hit = found
                    .iter()
                    .find(|t| {
                        source
                            .bounds(&t.range, cx)
                            .is_some_and(|b| b.contains(&e.position))
                    })
                    .cloned();
                if state.read(cx).hovered != hit {
                    state.update(cx, |s, cx| {
                        s.hovered = hit;
                        cx.notify();
                    });
                }
            }
        };
        let on_hover = {
            let state = state.clone();
            move |inside: &bool, _: &mut Window, cx: &mut App| {
                if !inside && state.read(cx).hovered.is_some() {
                    state.update(cx, |s, cx| {
                        s.hovered = None;
                        cx.notify();
                    });
                }
            }
        };
        root.on_mouse_move(on_move)
            .on_hover(on_hover)
            .child(self.field)
            .children(fills)
            .when_some(tip, |d, (bounds, tip)| {
                let tip = SharedString::from(tip);
                d.child(deferred(
                    anchored()
                        .position(bounds.bottom_left())
                        .snap_to_window()
                        .child(
                            div()
                                .id("var-tooltip")
                                .test_support()
                                .aria_label(tip.clone())
                                .max_w(style::VAR_TIP_MAX_WIDTH)
                                .px(style::SPACE_2)
                                .py(style::SPACE_1)
                                .rounded(style::POPOVER_RADIUS)
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(cx.theme().popover)
                                .text_color(cx.theme().popover_foreground)
                                .text_size(style::TEXT_SMALL)
                                .child(tip),
                        ),
                ))
            })
    }
}
