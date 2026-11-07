//! The Inherited headers group under a request's headers: what the collection and folders
//! add, read-only, with the level that supplies each and a mark when it is overridden.

use apinata_core::{Collection, HeaderSource, InheritedHeader, ItemPath, KeyValue};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme as _, Sizable as _};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::prelude::*;
use gpui_kit::{EventEmitter, SharedString, TestSupportExt, Window, div};

use crate::section::SectionEdit;
use crate::style::{HEADER_NAME_WIDTH, SPACE_0, SPACE_1, SPACE_2, SPACE_3, TEXT_SMALL};

/// One inherited header with the display name of the level that supplies it.
#[derive(Clone, Debug, PartialEq)]
pub struct InheritedRow {
    header: InheritedHeader,
    source_name: String,
}

/// The rows for `item`, in inheritance order; a folder is named as the Auth section names it.
pub fn inherited_rows(collection: &Collection, item: &ItemPath) -> Vec<InheritedRow> {
    collection
        .inherited_headers(item)
        .into_iter()
        .map(|header| {
            let source_name = match header.source() {
                HeaderSource::Folder(path) => collection
                    .folder(path)
                    .map_or(path.file_name(), |f| f.name())
                    .to_owned(),
                _ => "collection".to_owned(),
            };
            InheritedRow {
                header,
                source_name,
            }
        })
        .collect()
}

/// The user clicked the name of the level that supplies a header.
pub struct OpenSource(pub HeaderSource);

/// Read-only rows for inherited headers, over the request's own headers.
pub struct InheritedHeadersSection {
    rows: Vec<InheritedRow>,
    own: Vec<KeyValue>,
}

impl EventEmitter<SectionEdit> for InheritedHeadersSection {}
impl EventEmitter<OpenSource> for InheritedHeadersSection {}

impl InheritedHeadersSection {
    /// An empty group.
    pub fn new(_: &mut Window, _: &mut Context<Self>) -> Self {
        InheritedHeadersSection {
            rows: Vec::new(),
            own: Vec::new(),
        }
    }

    /// Shows `rows` from a snapshot.
    pub fn load(&mut self, rows: &[InheritedRow], cx: &mut Context<Self>) {
        self.rows = rows.to_vec();
        cx.notify();
    }

    /// Takes the request's live own rows, so a same-named inherited header reads as overridden.
    pub fn set_own(&mut self, headers: &[KeyValue], cx: &mut Context<Self>) {
        self.own = headers.to_vec();
        cx.notify();
    }
}

impl Render for InheritedHeadersSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.rows.is_empty() {
            return div().into_any_element();
        }
        let muted = cx.theme().muted_foreground;
        let rows = self.rows.iter().enumerate().map(|(n, row)| {
            let overridden = row.header.overridden(&self.own);
            let color = if overridden {
                muted
            } else {
                cx.theme().foreground
            };
            let source = row.header.source().clone();
            h_flex()
                .id(SharedString::from(format!("inherited-header-{n}")))
                .test_support()
                .gap(SPACE_2)
                .items_center()
                .child(
                    div()
                        .id(SharedString::from(format!("inherited-header-{n}-name")))
                        .test_support()
                        .aria_label(row.header.name().to_owned())
                        .w(HEADER_NAME_WIDTH)
                        .text_size(TEXT_SMALL)
                        .text_color(color)
                        .when(overridden, |d| d.line_through())
                        .child(row.header.name().to_owned()),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("inherited-header-{n}-value")))
                        .test_support()
                        .aria_label(row.header.value().to_owned())
                        .min_w_0()
                        .truncate()
                        .text_size(TEXT_SMALL)
                        .text_color(muted)
                        .when(overridden, |d| d.line_through())
                        .child(row.header.value().to_owned()),
                )
                .child(
                    Button::new(SharedString::from(format!("inherited-header-{n}-source")))
                        .link()
                        .xsmall()
                        .px(SPACE_1)
                        .py(SPACE_0)
                        .label(row.source_name.clone())
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(OpenSource(source.clone()));
                        })),
                )
                .when(overridden, |r| {
                    r.child(
                        div()
                            .id(SharedString::from(format!(
                                "inherited-header-{n}-overridden"
                            )))
                            .test_support()
                            .text_size(TEXT_SMALL)
                            .text_color(muted)
                            .child("overridden"),
                    )
                })
        });
        v_flex()
            .id("inherited-headers")
            .test_support()
            // The gap above lives here so an empty group leaves none behind.
            .mt(SPACE_3)
            .gap(SPACE_1)
            .w_full()
            .child(
                div()
                    .id("inherited-headers-heading")
                    .test_support()
                    .text_size(TEXT_SMALL)
                    .text_color(muted)
                    .child("Inherited headers"),
            )
            .children(rows)
            .into_any_element()
    }
}
