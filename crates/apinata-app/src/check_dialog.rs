//! The Check collection dialog: the core's static check of the saved files, listed errors
//! first, then by path. Activating a finding opens the tab that shows its file, or reveals
//! the file's place in the sidebar when no tab shows it.

use std::path::{Path, PathBuf};

use apinata_core::{
    CheckOptions, CheckReport, HintShell, ItemPath, Severity, VarValue, check_collection,
};
use gpui_kit::base::actions::Confirm;
use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, ThemeStyled as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, App, Entity, FocusHandle, KeyDownEvent, Role, ScrollHandle, Task, TestSupportExt,
    Window, actions, div,
};

use crate::style;
use crate::tabs::TabKind;
use crate::ui::SolidRing as _;
use crate::workspace::Workspace;

actions!(
    apinata,
    [
        /// Checks the current collection for problems without sending anything.
        CheckCollection
    ]
);

/// Where activating a finding goes.
#[derive(Debug, Clone, PartialEq)]
pub enum FindingTarget {
    /// The tab that shows the file.
    Tab(TabKind),
    /// The sidebar row to reveal.
    Reveal(ItemPath),
}

/// Opens what a finding names in the collection at `root`; the Shell handles it.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = apinata, no_json)]
pub struct OpenCheckFinding {
    /// The collection the finding is in.
    pub root: PathBuf,
    /// Where it leads.
    pub target: FindingTarget,
}

/// Where the finding on `item`, in `environment` when it names one, leads. The check names
/// collection and folder scripts by their settings file and a valueless secret by its
/// environment; a file that is not a request reveals its folder.
pub fn finding_target(item: &str, environment: Option<&str>) -> FindingTarget {
    if item == "opencollection.yml" {
        return FindingTarget::Tab(TabKind::Collection);
    }
    if environment == Some(item) || item.starts_with("environments/") {
        return FindingTarget::Tab(TabKind::Environments);
    }
    if let Some(dir) = item.strip_suffix("/folder.yml")
        && let Some(path) = ItemPath::new(dir)
    {
        return FindingTarget::Tab(TabKind::Folder(path));
    }
    let Some(path) = ItemPath::new(item) else {
        return FindingTarget::Reveal(ItemPath::root());
    };
    if [".yml", ".yaml", ".bru"]
        .iter()
        .any(|ext| item.ends_with(ext))
    {
        FindingTarget::Tab(TabKind::Request(path))
    } else {
        FindingTarget::Reveal(path.parent().unwrap_or_else(ItemPath::root))
    }
}

/// The sidebar row to reveal when `target` has no tab to open: the folder holding the file.
pub fn fallback_reveal(target: &FindingTarget) -> ItemPath {
    match target {
        FindingTarget::Reveal(path) => path.clone(),
        FindingTarget::Tab(TabKind::Request(path) | TabKind::Folder(path)) => {
            path.parent().unwrap_or_else(ItemPath::root)
        }
        FindingTarget::Tab(_) => ItemPath::root(),
    }
}

/// One finding as the dialog lists it.
#[derive(Debug, Clone)]
struct Row {
    severity: Severity,
    item: String,
    environment: Option<String>,
    message: String,
}

impl Row {
    // The line `apinata check` prints for the finding, so the two read alike.
    fn spoken(&self) -> String {
        format!(
            "{}: {}{}: {}",
            self.severity.as_str(),
            self.item,
            self.environment_suffix(),
            self.message
        )
    }

    fn environment_suffix(&self) -> String {
        (self.environment.as_deref())
            .map(|e| format!(" [{e}]"))
            .unwrap_or_default()
    }
}

fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// What the dialog shows.
enum State {
    Running,
    Done { summary: String, rows: Vec<Row> },
    Failed(String),
}

// The report's findings, errors before warnings and otherwise in the report's path order.
fn done(report: &CheckReport) -> State {
    let mut rows: Vec<Row> = (report.findings().iter())
        .map(|f| Row {
            severity: f.severity(),
            item: f.item().to_owned(),
            environment: f.environment().map(str::to_owned),
            message: f.message().to_owned(),
        })
        .collect();
    rows.sort_by_key(|r| r.severity);
    let summary = format!(
        "{}, {}, {} checked",
        plural(report.errors(), "error"),
        plural(report.warnings(), "warning"),
        plural(report.items_checked(), "request")
    );
    State::Done { summary, rows }
}

struct CheckView {
    workspace: Entity<Workspace>,
    root: PathBuf,
    state: State,
    selected: usize,
    scroll: ScrollHandle,
    list_focus: FocusHandle,
    // Where focus goes when the dialog closes on a finding.
    target: FocusHandle,
    _check: Option<Task<()>>,
}

impl CheckView {
    // Checks the saved files on a worker. Secrets are looked up in the app's store and the
    // active global environment counts as defined, as a send from the app would see them.
    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ws = self.workspace.read(cx);
        let mut opts = CheckOptions::default()
            .hint_shell(HintShell::Mcp)
            .secrets(std::sync::Arc::clone(ws.store()));
        let globals = (ws.effective_global())
            .and_then(|(g, name)| g.environment(&name).map(|e| e.variables()))
            .unwrap_or_default();
        for v in globals.into_iter().filter(|v| v.enabled) {
            let value = match v.value {
                VarValue::Text(text) => text,
                VarValue::Typed { data, .. } => data,
                _ => String::new(),
            };
            opts = opts.global_env_var(v.name, value);
        }
        self.state = State::Running;
        self.selected = 0;
        let root = self.root.clone();
        let work = cx.background_spawn(async move {
            check_collection(&root, &opts).map_err(|e| e.to_string())
        });
        self._check = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            this.update_in(cx, |view, window, cx| {
                view.state = match result {
                    Ok(report) => done(&report),
                    Err(text) => State::Failed(text),
                };
                view.scroll.set_offset(Default::default());
                view.list_focus.focus(window, cx);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn rows(&self) -> &[Row] {
        match &self.state {
            State::Done { rows, .. } => rows,
            _ => &[],
        }
    }

    fn step(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() || !self.list_focus.is_focused(window) {
            return;
        }
        let last = self.rows().len().saturating_sub(1);
        self.selected = match event.keystroke.key.as_str() {
            "up" => self.selected.saturating_sub(1),
            "down" => (self.selected + 1).min(last),
            "home" => 0,
            "end" => last,
            _ => return,
        };
        cx.stop_propagation();
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    // Closes the dialog, then dispatches from the focus it came from, as the search dialog does.
    fn open_finding(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows().get(ix) else {
            return;
        };
        let action: Box<dyn Action> = Box::new(OpenCheckFinding {
            root: self.root.clone(),
            target: finding_target(&row.item, row.environment.as_deref()),
        });
        window.close_dialog(cx);
        self.target.focus(window, cx);
        window.defer(cx, move |window, cx| window.dispatch_action(action, cx));
    }

    fn row(&self, ix: usize, row: &Row, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (active, hover, muted) = (theme.list_active, theme.list_hover, theme.muted_foreground);
        let colour = match row.severity {
            Severity::Error => theme.danger,
            _ => theme.warning,
        };
        let on = ix == self.selected;
        let place = format!("{}{}", row.item, row.environment_suffix());
        h_flex()
            .id(("check-finding", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_label(row.spoken())
            .aria_selected(on)
            .items_start()
            .gap(style::SPACE_2)
            .px(style::SPACE_3)
            .py(style::SPACE_1)
            .cursor_pointer()
            .when(on, |r| r.bg(active))
            .when(!on, |r| r.hover(move |s| s.bg(hover)))
            .on_click(cx.listener(move |view, _, window, cx| view.open_finding(ix, window, cx)))
            .child(
                div()
                    .flex_none()
                    .w(style::SEARCH_FIELD_WIDTH)
                    .text_color(colour)
                    .child(row.severity.as_str()),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().whitespace_normal().child(row.message.clone()))
                    .child(
                        div()
                            .text_size(style::TEXT_SMALL)
                            .text_color(muted)
                            .truncate()
                            .child(place),
                    ),
            )
    }
}

impl Render for CheckView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (muted, danger) = (cx.theme().muted_foreground, cx.theme().danger);
        let rerun = div()
            .id("check-rerun")
            .test_support()
            .aria_label("Re-run")
            .child(
                Button::new("check-rerun-button")
                    .solid_ring(cx)
                    .key_context(crate::ui::DIALOG_FOOTER_CONTEXT)
                    .compact()
                    .label("Re-run")
                    .on_click(cx.listener(|view, _, window, cx| view.run(window, cx))),
            );
        let status = match &self.state {
            State::Running => div()
                .id("check-running")
                .test_support()
                .aria_label("Checking…")
                .text_color(muted)
                .child("Checking…"),
            State::Done { summary, .. } => div()
                .id("check-summary")
                .test_support()
                .aria_label(summary.clone())
                .text_color(muted)
                .child(summary.clone()),
            State::Failed(text) => {
                let text = format!("The collection could not be checked: {text}");
                div()
                    .id("check-error")
                    .test_support()
                    .aria_label(text.clone())
                    .text_color(danger)
                    .whitespace_normal()
                    .child(text)
            }
        };
        let rows: Vec<_> = (self.rows().to_vec().iter().enumerate())
            .map(|(ix, row)| self.row(ix, row, cx).into_any_element())
            .collect();
        let empty = matches!(&self.state, State::Done { rows, .. } if rows.is_empty());
        let focused = self.list_focus.is_focused(window);
        v_flex()
            .gap(style::SPACE_2)
            .child(
                h_flex()
                    .gap(style::SPACE_2)
                    .px(style::SPACE_3)
                    .child(div().flex_1().min_w_0().child(status))
                    .child(rerun),
            )
            .when(empty, |v| {
                v.child(
                    div()
                        .id("check-empty")
                        .test_support()
                        .aria_label("No problems found")
                        .px(style::SPACE_3)
                        .py(style::SPACE_2)
                        .child("No problems found"),
                )
            })
            .when(!rows.is_empty(), |v| {
                v.child(
                    div()
                        .relative()
                        .pt(style::FOCUS_RING_ROOM)
                        .child(
                            v_flex()
                                .id("check-findings")
                                .test_support()
                                .role(Role::ListBox)
                                .aria_label("Findings")
                                .track_focus(&self.list_focus)
                                .on_key_down(cx.listener(Self::step))
                                // Enter opens the selected finding instead of closing the dialog.
                                .on_action(cx.listener(|view, _: &Confirm, window, cx| {
                                    if view.list_focus.is_focused(window) {
                                        cx.stop_propagation();
                                        view.open_finding(view.selected, window, cx);
                                    } else {
                                        cx.propagate();
                                    }
                                }))
                                .max_h(crate::palette::list_max_height(window))
                                .overflow_y_scroll()
                                .track_scroll(&self.scroll)
                                .children(rows),
                        )
                        .vertical_scrollbar(&self.scroll)
                        .when(focused, |list| {
                            list.child(
                                div()
                                    .absolute()
                                    .top(style::FOCUS_RING_ROOM)
                                    .left_0()
                                    .right_0()
                                    .bottom_0()
                                    .focus_ring_style(window, cx),
                            )
                        }),
                )
            })
    }
}

/// Opens the Check collection dialog over `window` and checks the collection at `root`.
/// Activating a finding dispatches [`OpenCheckFinding`] from the focus the dialog came from.
pub fn open(
    workspace: &Entity<Workspace>,
    root: &Path,
    fallback: FocusHandle,
    window: &mut Window,
    cx: &mut App,
) {
    if crate::scratch::refuse(workspace, root, window, cx) {
        return;
    }
    let target = window.focused(cx).unwrap_or(fallback);
    let view = cx.new(|cx| CheckView {
        workspace: workspace.clone(),
        root: root.to_path_buf(),
        state: State::Running,
        selected: 0,
        scroll: ScrollHandle::new(),
        list_focus: cx.focus_handle().tab_stop(true),
        target,
        _check: None,
    });
    window.open_dialog(cx, {
        let view = view.clone();
        move |dialog, window, _| {
            crate::ui::with_close(
                crate::dialog_frame::place(dialog, window),
                "Check collection",
            )
            .w(style::resolve(style::PALETTE_WIDTH, window))
            .on_ok(|_, _, _| false)
            .child(div().id("check-dialog").test_support().child(view.clone()))
        }
    });
    view.update(cx, |v, cx| v.run(window, cx));
}
