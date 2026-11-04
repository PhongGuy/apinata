//! The Variables overview dialog: the variables the active request sees, grouped by the
//! scope that defines them. A row a higher scope overrides is dimmed, struck through and
//! labelled "shadowed"; a secret shows a mask, never its value, and a runtime value the
//! editable dialog masks is masked here too, with an eye to reveal it.

use crate::env_table::SECRET_HIDDEN;
use crate::runtime_vars::Masking;
use crate::shell::OpenRuntimeVariables;
use crate::style;
use crate::ui::{self, RingRoom as _, RingSides};
use apinata_core::{ScopedVariable, VarScope};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::BTreeSet;

// The name column every row shares, so the value columns start at one x.
const NAME_WIDTH: Rems = rems(8.);

// Highest scope first, as a send reads them.
const ORDER: [VarScope; 7] = [
    VarScope::Runtime,
    VarScope::IterationData,
    VarScope::Request,
    VarScope::Folder,
    VarScope::Environment,
    VarScope::Collection,
    VarScope::GlobalEnvironment,
];

/// The heading a scope's rows sit under.
pub fn heading(scope: VarScope) -> &'static str {
    match scope {
        VarScope::Runtime => "Runtime",
        VarScope::IterationData => "Data row",
        VarScope::Request => "Request",
        VarScope::Folder => "Folder",
        VarScope::Environment => "Environment",
        VarScope::Collection => "Collection",
        VarScope::GlobalEnvironment => "Global environment",
        _ => "Other",
    }
}

/// One scope's heading and its rows by name.
pub struct Section {
    /// The scope the rows come from.
    pub scope: VarScope,
    /// The rows, sorted by name.
    pub rows: Vec<ScopedVariable>,
}

/// `rows` grouped by scope, highest first; a scope with no rows has no section.
pub fn sections(rows: &[ScopedVariable]) -> Vec<Section> {
    let mut out = Vec::new();
    for scope in ORDER {
        let mut group: Vec<_> = rows
            .iter()
            .filter(|r| r.scope() == scope)
            .cloned()
            .collect();
        if !group.is_empty() {
            group.sort_by(|a, b| a.name().cmp(b.name()));
            out.push(Section { scope, rows: group });
        }
    }
    out
}

/// The dialog body: the overview of one request, or `None` when no request is active.
pub struct VariablesOverview {
    sections: Option<Vec<Section>>,
    failed: bool,
    masking: Option<Masking>,
    // Rows (by scope and name, so a changed list keeps them) whose masked value is shown.
    revealed: BTreeSet<(String, String)>,
}

impl Render for VariablesOverview {
    // The list, then a footer button that leads on to the editable runtime variables.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.list(window, cx);
        // The overview holds no input, so every button in it takes Enter as its press.
        v_flex()
            .key_context(ui::DIALOG_FOOTER_CONTEXT)
            .gap_2()
            .child(list)
            .child(
                // The scroll box clips flush with this full-width button on three sides.
                div()
                    .pt_2()
                    .ring_room(RingSides::LEFT | RingSides::RIGHT | RingSides::BOTTOM)
                    .child(
                        Button::new("var-overview-edit-runtime")
                            .label("Edit runtime variables")
                            .on_click(|_, window, cx| {
                                window.close_dialog(cx);
                                window.defer(cx, |window, cx| {
                                    window.dispatch_action(Box::new(OpenRuntimeVariables), cx)
                                });
                            }),
                    ),
            )
    }
}

impl VariablesOverview {
    fn list(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tainted = self
            .masking
            .as_ref()
            .map(Masking::tainted)
            .unwrap_or_default();
        let root = v_flex().id("variables-overview").gap_1().test_support();
        let muted = cx.theme().muted_foreground;
        let look = crate::theme::shadowed_look(cx.theme());
        if self.failed {
            return root.child(
                div()
                    .id("var-overview-failed")
                    .test_support()
                    .child("The variables could not be read. Check the environment and try again."),
            );
        }
        let Some(sections) = &self.sections else {
            return root.child(
                div()
                    .id("var-overview-empty")
                    .test_support()
                    .child("Open a request to see its variables."),
            );
        };
        if sections.is_empty() {
            return root.child(
                div()
                    .id("var-overview-empty")
                    .test_support()
                    .child("No variables in scope."),
            );
        }
        let mut ix = 0;
        let mut children = Vec::new();
        for (g, section) in sections.iter().enumerate() {
            children.push(
                div()
                    .id(("var-overview-heading", g))
                    .test_support()
                    .pt_2()
                    .text_color(muted)
                    .aria_label(heading(section.scope))
                    .child(heading(section.scope))
                    .into_any_element(),
            );
            for row in &section.rows {
                let shown = self.shown(row, &tainted);
                children.push(
                    self.row_element(ix, row, shown, muted, look, cx)
                        .into_any_element(),
                );
                ix += 1;
            }
        }
        // Rows stop short of the overlay scrollbar so it never covers the "shadowed" label.
        root.pr(style::SCROLLBAR_GUTTER).children(children)
    }
}

// The identity a reveal is remembered under: the layer and name of the row.
fn reveal_key(row: &ScopedVariable) -> (String, String) {
    (format!("{:?}", row.scope()), row.name().to_owned())
}

// What a row shows: its text when plain or revealed, else the mask.
struct Shown {
    text: String,
    // Whether an eye is offered: the value is masked and the app can read it.
    eye: bool,
    revealed: bool,
}

impl VariablesOverview {
    // Applies the dialog's masking rule to `row`. A runtime value the core hides as secret is
    // read back from the store, so the eye can reveal it; other secrets have no value to show.
    fn shown(&self, row: &ScopedVariable, tainted: &[String]) -> Shown {
        let runtime = row.scope() == VarScope::Runtime;
        let text = match (row.value(), &self.masking) {
            (Some(text), _) => Some(text.to_owned()),
            (None, Some(m)) if runtime => m.captured_text(row.name()),
            _ => None,
        };
        let hidden_by_rule = self.masking.as_ref().is_some_and(|m| {
            if runtime {
                m.captured(row.name(), text.as_deref(), tainted)
            } else {
                let global = row.scope() == VarScope::GlobalEnvironment;
                m.scripted(row.name(), text.as_deref(), tainted, global)
            }
        });
        let masked = row.is_secret() || hidden_by_rule;
        let readable = text.is_some();
        let revealed = masked && readable && self.revealed.contains(&reveal_key(row));
        let text = match text {
            Some(text) if !masked || revealed => text,
            _ => SECRET_HIDDEN.to_owned(),
        };
        Shown {
            text,
            eye: masked && readable,
            revealed,
        }
    }

    // One row: name, value (masked when sensitive) and a "shadowed" label when a higher scope wins.
    fn row_element(
        &self,
        ix: usize,
        row: &ScopedVariable,
        shown: Shown,
        muted: Hsla,
        look: (f32, Option<Hsla>),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let shadowed = row.is_shadowed();
        let value = shown.text.clone();
        let (id, icon, words) = if shown.revealed {
            ("var-overview-hide", IconName::EyeOff, "Hide value")
        } else {
            ("var-overview-reveal", IconName::Eye, "Show value")
        };
        let now = shown.revealed;
        let key = reveal_key(row);
        h_flex()
            .id(("var-overview-row", ix))
            .test_support()
            .items_start()
            .gap_2()
            .when(shadowed, |d| {
                d.opacity(look.0)
                    .when_some(look.1, |d, color| d.text_color(color))
            })
            .child(
                div()
                    .id(("var-overview-name", ix))
                    .test_support()
                    .flex_none()
                    .w(NAME_WIDTH)
                    .min_w_0()
                    .whitespace_normal()
                    .aria_label(row.name().to_owned())
                    .child(row.name().to_owned()),
            )
            .child(
                div()
                    .id(("var-overview-value", ix))
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .whitespace_normal()
                    .when(shadowed, |d| d.line_through())
                    .aria_label(value.clone())
                    .child(value),
            )
            .when(shown.eye, |d| {
                d.child(
                    ui::swap_icon_button(("var-overview-eye", ix), (id, ix), icon, words).on_click(
                        cx.listener(move |t, _, _, cx| {
                            if now {
                                t.revealed.remove(&key);
                            } else {
                                t.revealed.insert(key.clone());
                            }
                            cx.notify();
                        }),
                    ),
                )
            })
            .when(shadowed, |d| {
                d.child(
                    div()
                        .id(("var-overview-shadowed", ix))
                        .test_support()
                        .flex_none()
                        .text_size(style::TEXT_SMALL)
                        .text_color(muted)
                        .child("shadowed"),
                )
            })
    }
}

/// Opens the dialog over `window` for `rows`, or its "open a request" note for `None`.
pub fn open(
    rows: Option<Vec<ScopedVariable>>,
    masking: Option<Masking>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<VariablesOverview> {
    show(rows, false, masking, window, cx)
}

/// Opens the dialog with a note that the request's variables could not be read.
pub fn open_failed(window: &mut Window, cx: &mut App) -> Entity<VariablesOverview> {
    show(None, true, None, window, cx)
}

fn show(
    rows: Option<Vec<ScopedVariable>>,
    failed: bool,
    masking: Option<Masking>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<VariablesOverview> {
    let view = cx.new(|_| VariablesOverview {
        sections: rows.as_deref().map(sections),
        failed,
        masking,
        revealed: BTreeSet::new(),
    });
    let body = view.clone();
    let scroll = ScrollHandle::new();
    window.open_dialog(cx, move |d, window, _| {
        let room = crate::dialog_frame::titled_body_room(window);
        // Enter acts on the focused eye or button only; with focus on the dialog itself it
        // would confirm and close the list. Escape and the x still close it.
        ui::with_close(crate::dialog_frame::place(d, window), "Variables")
            .on_ok(|_, _, _| false)
            .child(
                div()
                    .relative()
                    .child(
                        div()
                            .id("variables-overview-scroll")
                            .test_support()
                            .max_h(room.max(px(0.)))
                            .overflow_y_scroll()
                            .track_scroll(&scroll)
                            .child(body.clone()),
                    )
                    .vertical_scrollbar(&scroll),
            )
    });
    view
}
