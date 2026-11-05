//! The bottom dock over the app's session log: a Console tab and a Network tab, each read
//! through its own feed and cursor, polled only while the dock is open.

use std::sync::Arc;
use std::time::Duration;

use apinata_core::export::HarExchange;
use apinata_core::{
    ConsoleLevel, ConsoleLine, LoggedBody, LoggedHop, NetworkEntry, SendSource, SessionLog,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tab::Tab;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::body_text;
use crate::har_export;
use crate::log_feed::{self, Feed, SOURCE_FILTERS};
use crate::response_pane::{self, CONSOLE_FILTERS};
use crate::tab_area::TabArea;
use crate::timing_bar::{self, Phases};
use crate::{style, ui};

/// The most network rows the list draws, newest kept.
pub const DOCK_NETWORK_ROWS: usize = 200;

/// The dock's tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockTab {
    /// Script console lines.
    Console,
    /// Network exchanges.
    Network,
}

impl DockTab {
    // The tab at strip position `ix`: Console first, then Network.
    fn at(ix: usize) -> Self {
        if ix == 0 {
            DockTab::Console
        } else {
            DockTab::Network
        }
    }
}

/// The dock: both feeds, the filters, the network selection and the poll loop.
pub struct ConsoleDock {
    renders: usize,
    log: SessionLog,
    console: Feed<ConsoleLine>,
    network: Feed<NetworkEntry>,
    tab: DockTab,
    level: Option<ConsoleLevel>,
    search: String,
    search_input: Entity<InputState>,
    source: Option<SendSource>,
    // Lets an export go through the open request tab that shows the same exchange.
    tab_area: Option<WeakEntity<TabArea>>,
    selected: Option<u64>,
    polling: Option<Task<()>>,
    focus: FocusHandle,
    // One tab stop per dock tab, Console then Network.
    tab_focus: [FocusHandle; 2],
    collapsed: bool,
    _subs: Vec<Subscription>,
}

/// A click on a collapsed dock's header, asking the shell to show the whole dock.
pub struct ExpandDock;

impl EventEmitter<ExpandDock> for ConsoleDock {}

impl ConsoleDock {
    /// How often an open dock reads the log.
    pub const POLL: Duration = Duration::from_millis(250);

    /// A closed dock on the Console tab, reading `log`.
    pub fn new(log: SessionLog, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter console lines"));
        let mut subs = vec![
            cx.subscribe(&search_input, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = input.read(cx).value().to_string();
                    this.set_search(&query, cx);
                }
            }),
        ];
        subs.extend(ui::redraw_on_settings(cx));
        ConsoleDock {
            renders: 0,
            log,
            console: Feed::default(),
            network: Feed::default(),
            tab: DockTab::Console,
            level: None,
            search: String::new(),
            search_input,
            source: None,
            tab_area: None,
            selected: None,
            polling: None,
            focus: cx.focus_handle(),
            tab_focus: [
                cx.focus_handle().tab_stop(true),
                cx.focus_handle().tab_stop(true),
            ],
            collapsed: false,
            _subs: subs,
        }
    }

    /// Shows only the header, whose click then emits [`ExpandDock`], or the whole dock.
    pub fn set_collapsed(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.collapsed != on {
            self.collapsed = on;
            cx.notify();
        }
    }

    /// Whether only the header shows.
    pub fn collapsed(&self) -> bool {
        self.collapsed
    }

    /// Names the tab area whose request tabs the HAR export prefers over the session log.
    pub fn set_tab_area(&mut self, area: WeakEntity<TabArea>) {
        self.tab_area = Some(area);
    }

    /// Reads both kinds from their own cursors; notifies and returns true when either moved.
    /// Clears the selection when its entry is gone.
    pub fn poll(&mut self, cx: &mut Context<Self>) -> bool {
        // Each slice is taken and the lock released before anything is absorbed or drawn.
        let lines = self.log.console_since(self.console.next());
        let entries = self.log.network_since(self.network.next());
        let console = self.console.absorb(lines.entries, lines.next, lines.first);
        let network = self
            .network
            .absorb(entries.entries, entries.next, entries.first);
        let moved = console || network;
        if moved {
            self.drop_stale_selection();
            cx.notify();
        }
        moved
    }

    /// How many times this view has rendered; tests read it to check what a change redraws.
    #[doc(hidden)]
    pub fn renders(&self) -> usize {
        self.renders
    }

    /// On: polls at once, then every `POLL`. Off: stops polling.
    pub fn set_polling(&mut self, on: bool, cx: &mut Context<Self>) {
        if !on {
            self.polling = None;
            return;
        }
        if self.polling.is_some() {
            return;
        }
        self.poll(cx);
        // The loop holds only a weak handle, and dropping the stored task ends it.
        self.polling = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Self::POLL).await;
                if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    /// Whether the poll loop is running.
    pub fn polling(&self) -> bool {
        self.polling.is_some()
    }

    /// The active tab.
    pub fn tab(&self) -> DockTab {
        self.tab
    }

    /// Switches to `tab`.
    pub fn set_tab(&mut self, tab: DockTab, cx: &mut Context<Self>) {
        self.tab = tab;
        cx.notify();
    }

    // The dock tab holding focus, if any.
    fn focused_tab(&self, window: &Window) -> Option<usize> {
        self.tab_focus.iter().position(|f| f.is_focused(window))
    }

    // With two tabs, either arrow moves from the focused tab to the other one, choosing it.
    fn step_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.focused_tab(window) else {
            return;
        };
        let to = 1 - ix;
        window.focus(&self.tab_focus[to], cx);
        self.set_tab(DockTab::at(to), cx);
    }

    /// The console level filter; `None` keeps every level.
    pub fn level_filter(&self) -> Option<ConsoleLevel> {
        self.level
    }

    /// Sets the console level filter.
    pub fn set_level_filter(&mut self, f: Option<ConsoleLevel>, cx: &mut Context<Self>) {
        self.level = f;
        cx.notify();
    }

    /// Sets the console search text.
    pub fn set_search(&mut self, query: &str, cx: &mut Context<Self>) {
        self.search = query.to_string();
        cx.notify();
    }

    /// The network source filter; `None` keeps every source.
    pub fn source_filter(&self) -> Option<SendSource> {
        self.source
    }

    /// Sets the network source filter, dropping a selection it hides.
    pub fn set_source_filter(&mut self, f: Option<SendSource>, cx: &mut Context<Self>) {
        self.source = f;
        self.drop_stale_selection();
        cx.notify();
    }

    /// Lines passing the level filter and search, oldest first.
    pub fn console_shown(&self) -> Vec<Arc<ConsoleLine>> {
        self.console
            .entries()
            .iter()
            .filter(|l| {
                log_feed::console_keeps(l.level, &l.request, &l.text, self.level, &self.search)
            })
            .cloned()
            .collect()
    }

    /// Entries passing the source filter, oldest first.
    pub fn network_shown(&self) -> Vec<Arc<NetworkEntry>> {
        self.network
            .entries()
            .iter()
            .filter(|e| log_feed::network_keeps(e.source, self.source))
            .cloned()
            .collect()
    }

    /// Selects the network entry with `seq`, or clears the selection.
    pub fn select(&mut self, seq: Option<u64>, cx: &mut Context<Self>) {
        self.selected = seq;
        cx.notify();
    }

    /// The selected network entry, if it is still held.
    pub fn selected(&self) -> Option<Arc<NetworkEntry>> {
        let seq = self.selected?;
        self.network
            .entries()
            .iter()
            .find(|e| e.seq == seq)
            .cloned()
    }

    /// Clears the log's console lines, then polls.
    pub fn clear_console(&mut self, cx: &mut Context<Self>) {
        self.log.clear_console();
        self.poll(cx);
        cx.notify();
    }

    /// Clears the log's network entries and the selection, then polls.
    pub fn clear_network(&mut self, cx: &mut Context<Self>) {
        self.log.clear_network();
        self.selected = None;
        self.poll(cx);
        cx.notify();
    }

    // The level filter and search box over the shown lines. The pane's budget is spent
    // newest first, so a noisy log keeps its latest rows; they are drawn back in time order.
    fn console_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = CONSOLE_FILTERS
            .iter()
            .position(|f| *f == self.level)
            .unwrap_or(0);
        let dock = cx.entity().downgrade();
        let pick = ui::segmented(
            "dock-console-level",
            ["All", "Log", "Debug", "Info", "Warn", "Error"],
            selected,
            move |ix, _, cx| {
                let f = CONSOLE_FILTERS.get(*ix).copied().flatten();
                dock.update(cx, |d, cx| d.set_level_filter(f, cx)).ok();
            },
        );
        let search = div()
            .id("dock-console-search")
            .test_support()
            .flex_1()
            .child(ui::padded_input(
                Input::new(&self.search_input).bg(ui::field_fill(false, cx)),
                &self.search_input,
            ));
        let lines = self.console_shown();
        let (mut bodies, left) =
            response_pane::console_shown(lines.iter().rev().map(|l| l.text.as_str()));
        bodies.reverse();
        let lines = &lines[left..];
        let muted = cx.theme().muted_foreground;
        let none = lines
            .is_empty()
            .then(|| ui::empty_state("dock-console-none", "Nothing was logged.", &[]));
        let omitted = (left > 0).then(|| {
            div()
                .id("dock-console-omitted")
                .test_support()
                .text_color(muted)
                .child(log_feed::omitted_label(left, "line"))
        });
        let rows = lines.iter().zip(bodies).enumerate().map(|(ix, (l, body))| {
            let clock = log_feed::clock_label(l.at);
            let level = l.level.as_str();
            let tag = l.phase.map(|p| format!("[{}]", p.as_str()));
            let aria = match &tag {
                Some(tag) => format!("{clock} {} {tag} {level} {}", l.request, l.text),
                None => format!("{clock} {} {level} {}", l.request, l.text),
            };
            h_flex()
                .id(("dock-console", ix))
                .test_support()
                .aria_label(aria)
                .items_start()
                .gap(style::SPACE_2)
                .child(div().flex_none().text_color(muted).child(clock))
                .child(div().flex_none().child(l.request.clone()))
                .children(tag.map(|tag| div().flex_none().text_color(muted).child(tag)))
                .child(
                    div()
                        .flex_none()
                        .text_color(response_pane::level_colour(l.level, cx))
                        .child(level),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .whitespace_normal()
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(response_pane::console_body(
                            "dock-console-more",
                            ix,
                            body,
                            cx,
                        )),
                )
        });
        v_flex()
            .flex_1()
            .min_h_0()
            .px_2()
            .gap(style::SPACE_2)
            .child(h_flex().gap_2().items_center().child(pick).child(search))
            .child(response_pane::scroll_frame(
                "dock-console-list",
                v_flex().children(omitted).children(none).children(rows),
            ))
            .into_any_element()
    }

    // The source filter above the newest `DOCK_NETWORK_ROWS` shown entries, one clickable
    // row each; the selected row takes the theme's list selection colour.
    fn network_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = SOURCE_FILTERS
            .iter()
            .position(|f| *f == self.source)
            .unwrap_or(0);
        let dock = cx.entity().downgrade();
        let pick = ui::segmented(
            "dock-network-source",
            SOURCE_FILTERS.map(log_feed::source_label),
            current,
            move |ix, _, cx| {
                let f = SOURCE_FILTERS.get(*ix).copied().flatten();
                dock.update(cx, |d, cx| d.set_source_filter(f, cx)).ok();
            },
        );
        let entries = self.network_shown();
        let left = entries.len().saturating_sub(DOCK_NETWORK_ROWS);
        let muted = cx.theme().muted_foreground;
        let active = cx.theme().list_active;
        let none = entries
            .is_empty()
            .then(|| ui::empty_state("dock-network-none", "No requests yet.", &[]));
        let omitted = (left > 0).then(|| {
            div()
                .id("dock-network-omitted")
                .test_support()
                .text_color(muted)
                .child(log_feed::omitted_label(left, "request"))
        });
        let rows = entries[left..].iter().enumerate().map(|(ix, e)| {
            let (source, status) = (e.source.as_str(), log_feed::status_label(e.status));
            let time = body_text::format_time(e.duration);
            let seq = e.seq;
            let on = self.selected == Some(seq);
            h_flex()
                .when(on, |row| row.bg(active))
                .id(("dock-network", ix))
                .test_support()
                .role(Role::ListBoxOption)
                .aria_label(format!("{source} {} {} {status} {time}", e.method, e.url))
                .aria_selected(on)
                .gap(style::SPACE_2)
                .cursor_pointer()
                .on_click(cx.listener(move |d, _, _, cx| d.select(Some(seq), cx)))
                .child(div().flex_none().text_color(muted).child(source))
                .child(div().flex_none().child(e.method.clone()))
                .child(div().flex_1().min_w_0().truncate().child(e.url.clone()))
                .child(div().flex_none().child(status))
                .child(div().flex_none().text_color(muted).child(time))
        });
        // The list and the detail share the row; items_stretch keeps the scroll frames sized.
        let list = v_flex()
            .id("dock-network-column")
            .test_support()
            .role(Role::ListBox)
            .flex_1()
            .min_w_0()
            .h_full()
            .child(response_pane::scroll_frame(
                "dock-network-list",
                // Rows stop short of the overlay scrollbar; the empty note stays centred.
                v_flex()
                    .when(none.is_none(), |c| c.pr(style::SCROLLBAR_GUTTER))
                    .children(omitted)
                    .children(none)
                    .children(rows),
            ));
        let detail = self
            .selected()
            .map(|e| detail_pane(&e, self.tab_area.clone(), cx));
        v_flex()
            .flex_1()
            .min_h_0()
            .px_2()
            .gap(style::SPACE_2)
            .child(h_flex().items_center().child(pick))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .gap(style::SPACE_2)
                    .child(list)
                    .children(detail),
            )
            .into_any_element()
    }

    // A selection whose entry left the ring, no longer passes the source filter or fell
    // behind the drawn rows is dropped, so no detail outlives what the list shows.
    fn drop_stale_selection(&mut self) {
        let Some(seq) = self.selected else { return };
        let shown = self.network_shown();
        let drawn = &shown[shown.len().saturating_sub(DOCK_NETWORK_ROWS)..];
        if !drawn.iter().any(|e| e.seq == seq) {
            self.selected = None;
        }
    }
}

/// The detail pane as plain text: "{method} {url} {status or error}", the error line if any,
/// "Request headers" then one "name: value" line each, the request body, "Response headers"
/// (last hop) likewise, the response body, then one line per hop, each PAC-routed hop followed
/// by its "PAC: ..." line.
pub fn detail_text(e: &NetworkEntry) -> String {
    let mut out = vec![format!(
        "{} {} {}",
        e.method,
        e.url,
        log_feed::status_label(e.status)
    )];
    out.extend(attempt_label(e));
    out.extend(e.error.clone());
    let pairs = |out: &mut Vec<String>, pairs: &[(String, String)]| {
        out.extend(pairs.iter().map(|(n, v)| format!("{n}: {v}")));
    };
    out.push("Request headers".into());
    pairs(&mut out, &e.request_headers);
    if e.request_body.size > 0 {
        out.push(e.request_body.text.clone());
    }
    out.push("Response headers".into());
    pairs(&mut out, last_response_headers(e));
    if e.response_body.size > 0 {
        out.push(e.response_body.text.clone());
    }
    for h in &e.hops {
        out.push(hop_label(h));
        out.extend(hop_override_label(h));
        out.extend(hop_pac_label(h));
    }
    if e.hops_omitted > 0 {
        out.push(hops_omitted_label(e.hops_omitted));
    }
    out.join("\n")
}

// "Attempt: 2 of 3" for a send that was one of several tries, as the log numbered it.
fn attempt_label(e: &NetworkEntry) -> Option<String> {
    e.attempt
        .map(|(attempt, of)| format!("Attempt: {attempt} of {of}"))
}

fn last_response_headers(e: &NetworkEntry) -> &[(String, String)] {
    e.hops.last().map_or(&[], |h| &h.response_headers)
}

// "{method} {url} {status} · {version} · {time}", as the response timeline words a hop.
fn hop_label(h: &LoggedHop) -> String {
    let version = h.version().map(|v| format!(" · {v}")).unwrap_or_default();
    let time = body_text::format_time(h.elapsed);
    format!("{} {} {}{version} · {time}", h.method, h.url, h.status)
}

// The hop's measured phases on one line, or None when it was not timed.
fn hop_timing_label(h: &LoggedHop) -> Option<String> {
    let line = timing_bar::hop_line(&Phases::from(h.timing.as_ref()?));
    (!line.is_empty()).then_some(line)
}

// The hop's PAC route or fallback cause as the response timeline words it.
fn hop_pac_label(h: &LoggedHop) -> Option<String> {
    h.pac.as_ref().map(|route| format!("PAC: {route}"))
}

// The hop's host override as the response timeline words it.
fn hop_override_label(h: &LoggedHop) -> Option<String> {
    h.override_from
        .as_ref()
        .map(|mapping| format!("Host override: {mapping}"))
}

fn hops_omitted_label(n: usize) -> String {
    let plural = if n == 1 { "" } else { "s" };
    format!("{n} more hop{plural} not shown")
}

// The selected entry beside the list: error, both header tables and bodies, then the hops,
// laid out as the response pane's Timeline with muted section headings.
fn detail_pane(e: &NetworkEntry, area: Option<WeakEntity<TabArea>>, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let mono = cx.theme().mono_font_family.clone();
    let heading = |text: &'static str| {
        div()
            .pt(style::SPACE_1)
            .text_size(style::TEXT_SMALL)
            .text_color(muted)
            .child(text)
    };
    // Each body is bounded as a console entry is, with its own "more lines" note. The bound
    // can hide the log's truncation marker (the text's last line), so a cut body repeats it.
    let body = |[id, more_id, cut_id]: [&'static str; 3], b: &LoggedBody| {
        let cut = b.truncated.then(|| {
            let marker = b.text.lines().last().unwrap_or_default().to_string();
            div()
                .id((cut_id, 0usize))
                .test_support()
                .aria_label(marker.clone())
                .text_color(muted)
                .child(marker)
        });
        (b.size > 0).then(|| {
            v_flex()
                .w_full()
                .child(
                    div()
                        .id(id)
                        .test_support()
                        .aria_label(b.text.clone())
                        .font_family(mono.clone())
                        .child(response_pane::console_body(
                            more_id,
                            0,
                            response_pane::console_rows(&b.text),
                            cx,
                        )),
                )
                .children(cut)
        })
    };
    let attempt = attempt_label(e).map(|text| {
        div()
            .id("dock-attempt")
            .test_support()
            .aria_label(text.clone())
            .text_color(muted)
            .child(text)
    });
    let error = e.error.as_ref().map(|text| {
        div()
            .id("dock-error")
            .test_support()
            .aria_label(text.clone())
            .text_color(cx.theme().danger)
            .child(text.clone())
    });
    // A tab still showing this exchange exports it the way the tab does; otherwise the log's
    // entry, already redacted, exports as it stands.
    let logged = e.clone();
    let export = HarExchange::from_network_entry(e).map(|exchange| {
        Button::new("dock-export-har")
            .ghost()
            .compact()
            .label("Export as HAR")
            .on_click(move |_, window, cx| {
                let area = area.as_ref().and_then(|a| a.upgrade());
                let via_tab = area
                    .is_some_and(|a| a.update(cx, |a, cx| a.export_logged(&logged, window, cx)));
                if !via_tab {
                    har_export::save(exchange.clone(), window, cx);
                }
            })
    });
    let hops = e.hops.iter().enumerate().map(|(ix, h)| {
        let label = hop_label(h);
        let timing = hop_timing_label(h).map(|text| {
            div()
                .id(("dock-hop-timing", ix))
                .test_support()
                .aria_label(text.clone())
                .text_color(muted)
                .child(text)
        });
        let pac = hop_pac_label(h).map(|text| {
            div()
                .id(("dock-hop-pac", ix))
                .test_support()
                .aria_label(text.clone())
                .text_color(muted)
                .child(text)
        });
        let mapped = hop_override_label(h).map(|text| {
            div()
                .id(("dock-hop-override", ix))
                .test_support()
                .aria_label(text.clone())
                .text_color(muted)
                .child(text)
        });
        v_flex()
            .child(
                div()
                    .id(("dock-hop", ix))
                    .test_support()
                    .aria_label(label.clone())
                    .child(label),
            )
            .children(timing)
            .children(mapped)
            .children(pac)
    });
    let omitted = (e.hops_omitted > 0).then(|| {
        let label = hops_omitted_label(e.hops_omitted);
        div()
            .id("dock-hops-omitted")
            .test_support()
            .aria_label(label.clone())
            .text_color(muted)
            .child(label)
    });
    div()
        .id("dock-detail")
        .test_support()
        .aria_label(detail_text(e))
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(response_pane::scroll_frame_for(
            "dock-detail-scroll",
            e.seq,
            v_flex()
                .gap_1()
                .children(export)
                .child(format!(
                    "{} {} {}",
                    e.method,
                    e.url,
                    log_feed::status_label(e.status)
                ))
                .children(attempt)
                .children(error)
                .child(heading("Request headers"))
                .child(response_pane::header_table(
                    [
                        "dock-request-header",
                        "dock-request-header-name",
                        "dock-request-header-value",
                    ],
                    &e.request_headers,
                ))
                .children(body(
                    [
                        "dock-request-body",
                        "dock-request-body-more",
                        "dock-request-body-truncated",
                    ],
                    &e.request_body,
                ))
                .child(heading("Response headers"))
                .child(response_pane::header_table(
                    [
                        "dock-response-header",
                        "dock-response-header-name",
                        "dock-response-header-value",
                    ],
                    last_response_headers(e),
                ))
                .children(body(
                    [
                        "dock-response-body",
                        "dock-response-body-more",
                        "dock-response-body-truncated",
                    ],
                    &e.response_body,
                ))
                .when(!e.hops.is_empty(), |col| col.child(heading("Hops")))
                .children(hops)
                .children(omitted),
        ))
        .into_any_element()
}

impl Focusable for ConsoleDock {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ConsoleDock {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let tab = self.tab;
        let dock = cx.entity().downgrade();
        let ring = cx.theme().ring;
        let keyboard = crate::focus_mode::is_keyboard(cx);
        // Each tab is its own tab stop; the ring is an overlay on a padded box, since the
        // kit tab clips anything drawn outside its content, and marks keyboard focus only.
        let labels = ["Console", "Network"]
            .into_iter()
            .enumerate()
            .map(|(ix, label)| {
                let focus = &self.tab_focus[ix];
                // The box is as tall as a counted section tab, so both rings leave the same room.
                Tab::new().child(
                    h_flex()
                        .id(("dock-tab", ix))
                        .test_support()
                        .relative()
                        .track_focus(focus)
                        .line_height(relative(style::TAB_LINE_HEIGHT))
                        .px(style::SPACE_0)
                        .items_center()
                        .child(div().id(("dock-tab-label", ix)).test_support().child(label))
                        .child(ui::badge_strut())
                        .when(focus.is_focused(window) && keyboard, |d| {
                            d.child(
                                div()
                                    .id(("dock-tab-ring", ix))
                                    .test_support()
                                    .absolute()
                                    .inset_0()
                                    .border_1()
                                    .border_color(ring),
                            )
                        }),
                )
            });
        // The tabs answer the segmented keys: Enter and Space choose the
        // focused tab, the arrows choose and focus the other, wrapping.
        let tabs = div()
            .id("dock-tabs")
            .test_support()
            .key_context(ui::SEGMENTED_CONTEXT)
            .on_action(cx.listener(|d, _: &ui::ActivateSegment, window, cx| {
                if let Some(ix) = d.focused_tab(window) {
                    d.set_tab(DockTab::at(ix), cx);
                }
            }))
            .on_action(cx.listener(|d, _: &ui::NextSegment, window, cx| d.step_tab(window, cx)))
            .on_action(cx.listener(|d, _: &ui::PrevSegment, window, cx| d.step_tab(window, cx)))
            .child(
                ui::tab_strip("dock-tab-strip", cx)
                    .selected_index(match tab {
                        DockTab::Console => 0,
                        DockTab::Network => 1,
                    })
                    .children(labels)
                    .on_click(move |ix, _, cx| {
                        dock.update(cx, |d, cx| d.set_tab(DockTab::at(*ix), cx))
                            .ok();
                    }),
            );
        let clear = match tab {
            DockTab::Console => Button::new("dock-console-clear")
                .label("Clear")
                .on_click(cx.listener(|d, _, _, cx| d.clear_console(cx))),
            DockTab::Network => Button::new("dock-network-clear")
                .label("Clear")
                .on_click(cx.listener(|d, _, _, cx| d.clear_network(cx))),
        };
        let collapsed = self.collapsed;
        let body = (!collapsed).then(|| match tab {
            DockTab::Console => self.console_tab(cx),
            DockTab::Network => self.network_tab(cx),
        });
        v_flex()
            .id("console-dock")
            .test_support()
            .track_focus(&self.focus)
            .size_full()
            .child(
                h_flex()
                    .id("dock-header")
                    .test_support()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .when(collapsed, |h| {
                        h.on_click(cx.listener(|_, _, _, cx| cx.emit(ExpandDock)))
                    })
                    .child(tabs)
                    .child(div().flex_1())
                    .child(clear),
            )
            .children(body)
    }
}
