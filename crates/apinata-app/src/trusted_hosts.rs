//! The "Trusted hosts" settings of a collection: its trusted and denied hosts from the
//! per-user trust file, which this view edits in place.

use std::path::{Path, PathBuf};

use apinata_core::{TrustError, TrustStore, bare_host_refusal, normalize_bare_host};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Entity, Result, Subscription, TestSupportExt, Window, div,
};

use crate::app::AppState;
use crate::approval_prompt::untrusted_text;
use crate::{style, ui};

/// Shows one collection's lists from the trust file; typing a host and pressing Enter
/// trusts it, and each listed host has a remove button.
pub struct TrustedHostsView {
    trust_file: PathBuf,
    root: PathBuf,
    trusted: Vec<String>,
    denied: Vec<String>,
    error: Option<String>,
    input: Entity<InputState>,
    _subs: Vec<Subscription>,
}

impl TrustedHostsView {
    /// A view of the lists for the collection at `root` in `trust_file`, following the
    /// app's approvals when `AppState` is set so decisions made in the modal appear.
    pub fn new(
        trust_file: PathBuf,
        root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Host to trust"));
        let mut subs =
            vec![
                cx.subscribe_in(&input, window, |this, state, e: &InputEvent, window, cx| {
                    if matches!(e, InputEvent::PressEnter { .. }) {
                        let typed = state.read(cx).value().to_string();
                        let host = match bare_host(&typed) {
                            Ok(host) => host,
                            Err(message) => {
                                this.error = Some(message);
                                cx.notify();
                                return;
                            }
                        };
                        if this.write(|store, root| store.trust(root, &host), cx) {
                            state.update(cx, |s, cx| s.set_value("", window, cx));
                        }
                    }
                }),
            ];
        if let Some(state) = cx.try_global::<AppState>() {
            let approvals = state.approvals.clone();
            subs.push(cx.observe(&approvals, |this, _, cx| this.reload(cx)));
        }
        let mut view = TrustedHostsView {
            trust_file,
            root,
            trusted: Vec::new(),
            denied: Vec::new(),
            error: None,
            input,
            _subs: subs,
        };
        view.reload(cx);
        view
    }

    /// This collection's trusted hosts, in the store's order.
    pub fn trusted(&self) -> &[String] {
        &self.trusted
    }

    /// This collection's denied hosts, in the store's order.
    pub fn denied(&self) -> &[String] {
        &self.denied
    }

    /// The last load or write error, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Re-reads the trust file; a file that fails to load shows its error and empty lists.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        match TrustStore::load(&self.trust_file) {
            Ok(store) => {
                self.trusted = store.trusted(&self.root);
                self.denied = store.denied(&self.root);
                self.error = None;
            }
            Err(e) => {
                self.trusted.clear();
                self.denied.clear();
                self.error = Some(e.to_string());
            }
        }
        cx.notify();
    }

    // Reloads the store right before applying `edit` so entries written elsewhere survive;
    // a store that fails to load, or a host the store refuses, writes nothing.
    fn write(
        &mut self,
        edit: impl FnOnce(&mut TrustStore, &Path) -> Result<(), TrustError>,
        cx: &mut Context<Self>,
    ) -> bool {
        let written = TrustStore::load(&self.trust_file).and_then(|mut store| {
            edit(&mut store, &self.root)?;
            store.save()
        });
        match written {
            Ok(()) => {
                self.reload(cx);
                true
            }
            Err(e) => {
                self.error = Some(e.to_string());
                cx.notify();
                false
            }
        }
    }
}

/// The host to trust for `typed`, only when it is a bare host (see [`normalize_bare_host`]);
/// otherwise the error says why.
fn bare_host(typed: &str) -> Result<String, String> {
    normalize_bare_host(typed).ok_or_else(|| bare_host_refusal(typed))
}

// One list's rows: each host, shown through `untrusted_text` since the trust file can be
// hand-edited, beside a button that takes exactly that host off the lists.
fn host_rows(
    hosts: &[String],
    kind: &'static str,
    remove: &'static str,
    cx: &mut Context<TrustedHostsView>,
) -> Vec<AnyElement> {
    hosts
        .iter()
        .enumerate()
        .map(|(ix, host)| {
            let target = host.clone();
            h_flex()
                .gap_2()
                .child(
                    div()
                        .id((kind, ix))
                        .test_support()
                        .child(untrusted_text(host)),
                )
                .child(
                    Button::new((remove, ix))
                        .label("Remove")
                        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                            // The next host moves up under the pointer, so the second half
                            // of a double click must not remove it too.
                            if event.click_count() > 1 {
                                return;
                            }
                            let host = target.clone();
                            this.write(
                                |store, root| {
                                    store.remove(root, &host);
                                    Ok(())
                                },
                                cx,
                            );
                        })),
                )
                .into_any_element()
        })
        .collect()
}

/// A muted line standing in for a list with no hosts.
fn empty_row(id: &'static str, text: &'static str, cx: &App) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

impl Render for TrustedHostsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let trusted = host_rows(&self.trusted, "trusted-host", "trusted-remove", cx);
        let denied = host_rows(&self.denied, "denied-host", "denied-remove", cx);
        v_flex()
            .id("trusted-hosts")
            .gap_2()
            .child(
                h_flex().gap_2().child("Trust host").child(ui::padded_input(
                    Input::new(&self.input)
                        .id("hosts-add")
                        .w(style::WIDE_INPUT_WIDTH)
                        .bg(ui::field_fill(false, cx)),
                    &self.input,
                )),
            )
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .id("hosts-error")
                        .test_support()
                        .child(untrusted_text(&error)),
                )
            })
            .child("Trusted")
            .when(trusted.is_empty(), |el| {
                el.child(empty_row("trusted-empty", "No trusted hosts yet.", cx))
            })
            .children(trusted)
            .child("Denied")
            .when(denied.is_empty(), |el| {
                el.child(empty_row("denied-empty", "No denied hosts.", cx))
            })
            .children(denied)
    }
}
