//! The Default headers group under a request's headers: the headers core adds, each with a
//! toggle that omits it from this request's `settings.omitHeaders`.

use apinata_core::{KeyValue, default_headers};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::prelude::*;
use gpui_kit::{App, EventEmitter, SharedString, Styled, TestSupportExt, Window, div};

use crate::command::Edit;
use crate::kv_grid;
use crate::kv_table::KvColumns;
use crate::section::SectionEdit;
use crate::style::{KV_ACTION_WIDTH, KV_GRIP_WIDTH, SPACE_1, SPACE_2, TEXT_SMALL};
use crate::ui;

/// Read-only rows for the default headers, over the headers the user set themselves.
///
/// A toggle sends the last loaded or sent omit list with one name changed, so hand-written
/// names for headers this group does not list stay in the file.
pub struct DefaultHeadersSection {
    // The omit list last sent or loaded; the base each toggle builds on.
    omitted: Vec<String>,
    // Lower-cased names of the user's enabled headers, which win over a default of that name.
    own: Vec<String>,
}

impl EventEmitter<SectionEdit> for DefaultHeadersSection {}

impl DefaultHeadersSection {
    /// A group with nothing omitted and no headers of the user's own.
    pub fn new(_: &mut Window, _: &mut Context<Self>) -> Self {
        DefaultHeadersSection {
            omitted: Vec::new(),
            own: Vec::new(),
        }
    }

    /// Shows `omitted` from a snapshot without emitting an edit.
    pub fn load(&mut self, omitted: &[String], cx: &mut Context<Self>) {
        self.omitted = omitted.to_vec();
        cx.notify();
    }

    /// Takes the request's own headers, so a default it also sets reads as overridden.
    pub fn set_own(&mut self, headers: &[KeyValue], cx: &mut Context<Self>) {
        self.own = headers
            .iter()
            .filter(|h| h.enabled)
            .map(|h| h.name.trim().to_ascii_lowercase())
            .collect();
        cx.notify();
    }

    /// Whether `edit`, which this group sent, was built from an omit list a load has since
    /// replaced.
    pub fn lags(&self, edit: &Edit) -> bool {
        matches!(edit, Edit::OmitHeaders(sent) if *sent != self.omitted)
    }

    fn is_omitted(&self, name: &str) -> bool {
        self.omitted.iter().any(|o| o.eq_ignore_ascii_case(name))
    }

    fn set_sent(&mut self, name: &str, sent: bool, cx: &mut Context<Self>) {
        if sent {
            self.omitted.retain(|o| !o.eq_ignore_ascii_case(name));
        } else if !self.is_omitted(name) {
            self.omitted.push(name.to_owned());
        }
        cx.emit(SectionEdit::new(Edit::OmitHeaders(self.omitted.clone())));
        cx.notify();
    }
}

/// The id slug of a header name: `User-Agent` is `user-agent`.
fn slug(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// Styles a header name or value cell as the header table draws its cells: small mono text.
pub fn header_cell<E: Styled>(el: E, cx: &App) -> E {
    el.text_size(TEXT_SMALL)
        .font_family(cx.theme().mono_font_family.clone())
}

impl Render for DefaultHeadersSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let columns = KvColumns::DESCRIBED.describe(("", ""));
        let rows = default_headers().into_iter().map(|header| {
            let slug = slug(&header.name);
            let overridden = self.own.contains(&slug);
            let sent = overridden || !self.is_omitted(&header.name);
            let name = header.name.clone();
            // The HTTP stack always sends some Accept, so an omitted one reads as what goes out.
            let shown = if !sent && header.name.eq_ignore_ascii_case("Accept") {
                "sends */*".to_owned()
            } else {
                header.value.clone()
            };
            let toggle = ui::checkbox(
                SharedString::from(format!("default-header-send-{slug}")),
                sent,
            )
            .disabled(overridden)
            .accessible_name(format!("Send {}", header.name))
            .on_click(cx.listener(move |t, on: &bool, _, cx| t.set_sent(&name, *on, cx)));
            // The headers table's slots and columns, so name and value sit under its headings.
            let [name_col, value_col, note_col] = &columns[..] else {
                unreachable!("a described table has three columns")
            };
            h_flex()
                .id(SharedString::from(format!("default-header-{slug}")))
                .test_support()
                .gap(SPACE_2)
                .items_center()
                .child(div().flex_none().w(KV_GRIP_WIDTH))
                .child(kv_grid::check_slot(toggle))
                .child(
                    name_col.size(
                        header_cell(div(), cx)
                            .id(SharedString::from(format!("default-header-{slug}-name")))
                            .test_support()
                            .truncate()
                            .text_color(if sent { cx.theme().foreground } else { muted })
                            .child(header.name.clone()),
                    ),
                )
                .child(
                    value_col.size(
                        header_cell(div(), cx)
                            .id(SharedString::from(format!("default-header-{slug}-value")))
                            .test_support()
                            .aria_label(shown.clone())
                            .truncate()
                            .text_color(muted)
                            .child(shown),
                    ),
                )
                .child(note_col.size(div()).when(overridden, |d| {
                    d.child(
                        div()
                            .id(SharedString::from(format!(
                                "default-header-{slug}-overridden"
                            )))
                            .test_support()
                            .text_size(TEXT_SMALL)
                            .text_color(muted)
                            .child("set above"),
                    )
                }))
                .child(div().flex_none().w(KV_ACTION_WIDTH))
        });
        v_flex()
            .id("default-headers")
            .test_support()
            .gap(SPACE_1)
            .w_full()
            .child(
                div()
                    .id("default-headers-heading")
                    .test_support()
                    .text_size(TEXT_SMALL)
                    .text_color(muted)
                    .child("Default headers"),
            )
            .children(rows)
    }
}
