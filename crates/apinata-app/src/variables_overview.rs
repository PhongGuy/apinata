//! The dock's Variables tab: the variables the active request sees, grouped by the scope
//! that defines them, after the names a send would refuse as undefined. A row a higher scope
//! overrides is dimmed, struck through and labelled with the scope that wins; a secret shows a
//! mask, never its value. Runtime and environment values, and undefined names, are edited in
//! place; a row of any other scope opens that scope's own editor.

use crate::env_table::SECRET_HIDDEN;
use crate::keymap::OpenEnvironments;
use crate::kv_grid;
use crate::runtime_vars::Masking;
use crate::set_variable::{self, VarTarget};
use crate::shell::OpenGlobalEnvironments;
use crate::style;
use crate::tab_area::TabArea;
use crate::tabs::{TabId, TabKind};
use crate::ui::{self, ToastKind};
use crate::workspace::Workspace;
use apinata_core::{ItemPath, ScopedVariable, VarScope, VarValue, VariableSet};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Entity, Focusable as _, Hsla, Rems, ScrollHandle, SharedString, Subscription,
    TestSupportExt, WeakEntity, Window, div, rems,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

// Which in-place field an input belongs to: a defined row by scope and name, or an
// undefined name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum FieldKey {
    Defined(u8, String),
    Undefined(String),
}

fn scope_rank(scope: VarScope) -> u8 {
    ORDER
        .iter()
        .position(|s| *s == scope)
        .unwrap_or(ORDER.len()) as u8
}

// One in-place input, the text it was last filled with, the last value a write refused (so
// its toast shows once), and its Enter/blur subscription.
struct Field {
    input: Entity<InputState>,
    synced: String,
    refused: Option<String>,
    _sub: Subscription,
}

// What the shown rows were built from, so an unchanged editor costs no rebuild.
#[derive(PartialEq)]
struct Built {
    tab: TabId,
    set: Option<usize>,
    revision: u64,
    undefined: Vec<SharedString>,
    environment: Option<String>,
}

// The request the rows belong to.
struct Bound {
    tab: TabId,
    root: PathBuf,
    item: ItemPath,
    _editor: Subscription,
}

/// The Variables tab's list over the active request tab, rebuilt whenever that tab or its
/// variable set changes.
pub struct VariablesOverview {
    workspace: Entity<Workspace>,
    tab_area: WeakEntity<TabArea>,
    bound: Option<Bound>,
    built: Option<Built>,
    sections: Option<Vec<Section>>,
    undefined: Vec<String>,
    // Undefined names declared secret somewhere, each with the scope whose editor holds it.
    unstored: BTreeMap<String, VarScope>,
    masking: Option<Masking>,
    // Rows (by scope and name, so a changed list keeps them) whose masked value is shown.
    revealed: BTreeSet<(String, String)>,
    // The scope whose definition wins, per name, so an overridden row can name it.
    winners: BTreeMap<String, VarScope>,
    // The selected environment's own plain text values, the ones edited in place.
    env_texts: BTreeMap<String, String>,
    environment: Option<String>,
    fields: BTreeMap<FieldKey, Field>,
    focused: Option<String>,
    scroll: ScrollHandle,
    _subs: Vec<Subscription>,
}

impl VariablesOverview {
    /// A list over the active request tab of `tab_area`, following tab switches and
    /// collection changes in `workspace`.
    pub fn new(
        workspace: Entity<Workspace>,
        tab_area: &Entity<TabArea>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subs = vec![
            cx.observe_in(&workspace, window, |this, _, window, cx| {
                this.refresh(window, cx);
            }),
            cx.observe_in(tab_area, window, |this, _, window, cx| {
                this.refresh(window, cx);
            }),
        ];
        VariablesOverview {
            workspace,
            tab_area: tab_area.downgrade(),
            bound: None,
            built: None,
            sections: None,
            undefined: Vec::new(),
            unstored: BTreeMap::new(),
            masking: None,
            revealed: BTreeSet::new(),
            winners: BTreeMap::new(),
            env_texts: BTreeMap::new(),
            environment: None,
            fields: BTreeMap::new(),
            focused: None,
            scroll: ScrollHandle::new(),
            _subs: subs,
        }
    }

    /// The name the last [`Self::focus_name`] picked, if any.
    pub fn focused(&self) -> Option<&str> {
        self.focused.as_deref()
    }

    /// Marks the row of `name` and moves focus to its input when it is edited in place.
    pub fn focus_name(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh(window, cx);
        self.focused = Some(name.to_owned());
        let key = (self.fields.keys())
            .find(|k| match k {
                FieldKey::Defined(_, n) | FieldKey::Undefined(n) => n == name,
            })
            .cloned();
        if let Some(field) = key.and_then(|k| self.fields.get(&k)) {
            field.input.update(cx, |i, cx| i.focus(window, cx));
        }
        if let Some(ix) = self.child_index(name) {
            self.scroll.scroll_to_item(ix);
        }
        cx.notify();
    }

    // Where `name`'s first row sits among the list's children, headings included.
    fn child_index(&self, name: &str) -> Option<usize> {
        let mut ix = 0;
        if !self.undefined.is_empty() {
            ix += 1;
            if let Some(at) = self.undefined.iter().position(|n| n == name) {
                return Some(ix + at);
            }
            ix += self.undefined.len();
        }
        for section in self.sections.iter().flatten() {
            ix += 1;
            if let Some(at) = section.rows.iter().position(|r| r.name() == name) {
                return Some(ix + at);
            }
            ix += section.rows.len();
        }
        None
    }

    // Follows the active request tab and rebuilds the rows when its set, the set's runtime
    // revision, its undefined names or the selected environment moved.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = self
            .workspace
            .read(cx)
            .tabs()
            .active()
            .and_then(|e| match &e.kind {
                TabKind::Request(item) => Some((e.id, e.root.clone(), item.clone())),
                _ => None,
            });
        let editor = active.as_ref().and_then(|(id, _, _)| {
            let area = self.tab_area.upgrade()?;
            area.read(cx).editor(*id).cloned()
        });
        let leaving = self
            .bound
            .as_ref()
            .is_some_and(|b| Some(b.tab) != active.as_ref().map(|a| a.0));
        // A field belongs to one request: whatever its leave-save could not write is dropped
        // with it, so same-named fields of the next request start from their own state.
        if leaving {
            self.flush(window, cx);
            self.fields.clear();
        }
        let (Some((tab, root, item)), Some(editor)) = (active, editor) else {
            let had = self.bound.take().is_some() | self.built.take().is_some();
            if had {
                self.sections = None;
                self.undefined.clear();
                self.fields.clear();
                cx.notify();
            }
            return;
        };
        if self.bound.as_ref().is_none_or(|b| b.tab != tab) {
            let sub = cx.observe_in(&editor, window, |this, _, window, cx| {
                this.refresh(window, cx);
            });
            self.bound = Some(Bound {
                tab,
                root: root.clone(),
                item,
                _editor: sub,
            });
            self.focused = None;
        }
        let (set, undefined) = {
            let e = editor.read(cx);
            (e.variables().cloned(), e.undefined_shown())
        };
        let environment = self.workspace.read(cx).selected_environment(&root, cx);
        let built = Built {
            tab,
            set: set.as_ref().map(|s| Arc::as_ptr(s) as usize),
            revision: set.as_ref().map(|s| s.revision()).unwrap_or_default(),
            undefined,
            environment: environment.clone(),
        };
        if self.built.as_ref() == Some(&built) {
            return;
        }
        self.rebuild(&root, set.as_deref(), &built, window, cx);
        self.built = Some(built);
        cx.notify();
    }

    fn rebuild(
        &mut self,
        root: &Path,
        set: Option<&VariableSet>,
        built: &Built,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rows = set.map(VariableSet::overview);
        let (secrets, global_secrets) = secret_names(&self.workspace, Some(root), cx);
        // An undefined name declared secret in any environment or global environment, active
        // or not, gets the editor that holds it instead of a plain field.
        let homes: BTreeMap<String, VarScope> = (built.undefined.iter())
            .filter_map(|n| {
                let (scope, _) = set_variable::secret_home(&self.workspace, root, n, cx)?;
                Some((n.to_string(), scope))
            })
            .collect();
        let global_active = global_secrets.clone();
        let ws = self.workspace.read(cx);
        self.masking = ws
            .runtime_vars(root, cx)
            .map(|vars| Masking::new(vars, secrets, global_secrets));
        self.environment = built.environment.clone();
        self.env_texts = (ws.snapshot(root, cx).zip(built.environment.as_ref()))
            .and_then(|(snap, env)| {
                let view = snap.content.environments.iter().find(|e| e.name == *env)?;
                Some(
                    (view.variables.iter())
                        .filter(|v| v.enabled && !v.secret)
                        .filter_map(|v| match &v.value {
                            VarValue::Text(t) => Some((v.name.clone(), t.clone())),
                            _ => None,
                        })
                        .collect(),
                )
            })
            .unwrap_or_default();
        self.winners = (rows.iter().flatten())
            .filter(|r| !r.is_shadowed())
            .map(|r| (r.name().to_owned(), r.scope()))
            .collect();
        self.sections = rows.as_deref().map(sections);
        // Undefined names that also list as rows are the editor's stale view; the rows win.
        self.undefined = (built.undefined.iter())
            .map(|n| n.to_string())
            .filter(|n| !self.winners.contains_key(n))
            .collect();
        self.unstored = (self.undefined.iter())
            .filter_map(|n| {
                let scope = if global_active.contains(n) {
                    VarScope::GlobalEnvironment
                } else if let Some(scope) = homes.get(n) {
                    *scope
                } else if set.is_some_and(|s| s.is_unstored_secret(n)) {
                    VarScope::Environment
                } else {
                    return None;
                };
                Some((n.clone(), scope))
            })
            .collect();
        self.sync_fields(window, cx);
    }

    // The text a defined row is edited as, or `None` when it is not edited in place: only an
    // unmasked winning row of the runtime scope or of the selected environment's own file.
    fn editable_text(&self, row: &ScopedVariable, tainted: &[String]) -> Option<String> {
        if row.is_shadowed() || row.is_secret() {
            return None;
        }
        let shown = self.shown(row, tainted);
        if shown.eye {
            return None;
        }
        match row.scope() {
            VarScope::Runtime => self.masking.as_ref()?.captured_text(row.name()),
            VarScope::Environment => self.env_texts.get(row.name()).cloned(),
            _ => None,
        }
    }

    // Gives every in-place row an input, refills inputs nobody is typing in, and drops the
    // inputs of rows that are gone.
    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tainted = (self.masking.as_ref())
            .map(Masking::tainted)
            .unwrap_or_default();
        let mut wanted: Vec<(FieldKey, String)> = (self.sections.iter().flatten())
            .flat_map(|s| s.rows.iter())
            .filter_map(|row| {
                let text = self.editable_text(row, &tainted)?;
                Some((
                    FieldKey::Defined(scope_rank(row.scope()), row.name().to_owned()),
                    text,
                ))
            })
            .collect();
        wanted.extend(
            (self.undefined.iter())
                .filter(|n| !self.unstored.contains_key(*n))
                .map(|n| (FieldKey::Undefined(n.clone()), String::new())),
        );
        let keys: BTreeSet<FieldKey> = wanted.iter().map(|(k, _)| k.clone()).collect();
        self.fields.retain(|k, _| keys.contains(k));
        for (key, text) in wanted {
            if let Some(field) = self.fields.get_mut(&key) {
                let typing = field.input.focus_handle(cx).contains_focused(window, cx);
                if !typing && field.synced != text {
                    field.synced = text.clone();
                    field
                        .input
                        .update(cx, |i, cx| i.set_value(text, window, cx));
                }
                continue;
            }
            let holder = match &key {
                FieldKey::Undefined(_) => match &self.environment {
                    Some(env) => format!("Value for {env}"),
                    None => "Runtime value".to_owned(),
                },
                FieldKey::Defined(..) => String::new(),
            };
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(holder)
                    .default_value(text.clone())
            });
            let at = key.clone();
            let sub = cx.subscribe_in(
                &input,
                window,
                move |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        this.commit(&at, window, cx);
                    }
                },
            );
            self.fields.insert(
                key,
                Field {
                    input,
                    synced: text,
                    refused: None,
                    _sub: sub,
                },
            );
        }
    }

    // Writes a changed field to its scope: a runtime row to the runtime store, an environment
    // row to the selected environment's file, an undefined name to the selected environment,
    // else to the runtime store.
    fn commit(&mut self, key: &FieldKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.write(key, window, cx) {
            self.refresh(window, cx);
        }
    }

    // Writes every field whose text differs from what it was filled with, before the rows
    // they belong to go away, so leaving the request keeps an edit as blur would.
    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirty: Vec<FieldKey> = (self.fields.iter())
            .filter(|(_, f)| f.input.read(cx).value().as_ref() != f.synced)
            .map(|(k, _)| k.clone())
            .collect();
        for key in dirty {
            self.write(&key, window, cx);
        }
    }

    // Writes one field to its scope; returns whether a write happened. A field is marked
    // written only once the write is accepted, so a refused one stays pending for a retry.
    fn write(&mut self, key: &FieldKey, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let (Some(field), Some(bound)) = (self.fields.get(key), &self.bound) else {
            return false;
        };
        let value = field.input.read(cx).value().to_string();
        if value == field.synced || (matches!(key, FieldKey::Undefined(_)) && value.is_empty()) {
            return false;
        }
        let env_target = || self.environment.clone().map(VarTarget::Environment);
        let (name, target) = match key {
            FieldKey::Defined(rank, name) if *rank == scope_rank(VarScope::Runtime) => {
                (name, Some(VarTarget::Runtime))
            }
            FieldKey::Defined(_, name) => (name, env_target()),
            // A secret never goes into a plain field, whichever scope declares it.
            FieldKey::Undefined(name) if self.unstored.contains_key(name) => (name, None),
            FieldKey::Undefined(name) => (name, env_target().or(Some(VarTarget::Runtime))),
        };
        let Some(target) = target else {
            return false;
        };
        let (root, tab) = (bound.root.clone(), bound.tab);
        let name = name.clone();
        let workspace = self.workspace.clone();
        match set_variable::apply(&workspace, &root, &name, &value, &target, cx) {
            Ok(()) => {
                if let Some(field) = self.fields.get_mut(key) {
                    field.synced = value;
                }
                // A runtime value is read live, so the editor's badge and this list follow now;
                // a file write comes back through the collection's next snapshot.
                if let Some(area) = self.tab_area.upgrade()
                    && let Some(editor) = area.read(cx).editor(tab).cloned()
                {
                    editor.update(cx, |_, cx| cx.notify());
                }
                true
            }
            Err(message) => {
                let field = self.fields.get_mut(key);
                let again = field
                    .as_ref()
                    .is_some_and(|f| f.refused.as_ref() == Some(&value));
                if let Some(field) = field {
                    field.refused = Some(value);
                }
                if !again {
                    window.push_notification(ui::kind_toast(ToastKind::Error, message), cx);
                }
                false
            }
        }
    }

    /// Whether the field of `name` holds text not yet written.
    #[doc(hidden)]
    pub fn pending(&self, name: &str, cx: &App) -> bool {
        (self.fields.iter()).any(|(k, f)| {
            let (FieldKey::Defined(_, n) | FieldKey::Undefined(n)) = k;
            n == name && f.input.read(cx).value().as_ref() != f.synced
        })
    }

    // Opens the editor of the scope that defines `name` in `scope`.
    fn open_editor(
        &mut self,
        scope: VarScope,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bound) = &self.bound else {
            return;
        };
        let (root, item) = (bound.root.clone(), bound.item.clone());
        let kind = match scope {
            VarScope::Environment => {
                window.dispatch_action(Box::new(OpenEnvironments), cx);
                return;
            }
            VarScope::GlobalEnvironment => {
                window.dispatch_action(Box::new(OpenGlobalEnvironments), cx);
                return;
            }
            VarScope::Collection => TabKind::Collection,
            VarScope::Folder => {
                TabKind::Folder(defining_folder(&self.workspace, &root, &item, name, cx))
            }
            _ => TabKind::Request(item),
        };
        let opened = self.workspace.update(cx, |w, cx| match kind {
            TabKind::Request(item) => w.open_item(&root, item, false, cx),
            kind => w.open_settings(&root, kind, cx),
        });
        if let (Some(id), Some(area)) = (opened, self.tab_area.upgrade()) {
            area.update(cx, |t, cx| t.focus_tab(id, window, cx));
        }
    }
}

// The innermost folder above `item` whose own variables define `name`, else its parent.
fn defining_folder(
    workspace: &Entity<Workspace>,
    root: &Path,
    item: &ItemPath,
    name: &str,
    cx: &App,
) -> ItemPath {
    let snapshot = workspace.read(cx).snapshot(root, cx);
    let mut at = item.parent();
    while let Some(folder) = at {
        let defines = (snapshot.as_ref())
            .and_then(|s| s.content.folders.get(&folder).cloned())
            .is_some_and(|f| f.vars.iter().any(|v| v.enabled && v.name == name));
        if defines {
            return folder;
        }
        at = folder.parent();
    }
    item.parent().unwrap_or_else(ItemPath::root)
}

/// The selected environment's secret names, then the active global environment's.
pub fn secret_names(
    workspace: &Entity<Workspace>,
    root: Option<&Path>,
    cx: &App,
) -> (Vec<String>, Vec<String>) {
    let ws = workspace.read(cx);
    let secrets = root
        .and_then(|r| ws.snapshot(r, cx))
        .map(|s| {
            let selected = s.selected_environment.as_deref();
            (s.content.environments.iter())
                .filter(|e| Some(e.name.as_str()) == selected)
                .flat_map(|e| e.variables.iter().filter(|v| v.secret))
                .map(|v| v.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let global_secrets = (ws.effective_global())
        .and_then(|(g, name)| g.environment(&name).map(|e| e.variables()))
        .map(|vars| {
            (vars.into_iter())
                .filter(|v| v.secret)
                .map(|v| v.name)
                .collect()
        })
        .unwrap_or_default();
    (secrets, global_secrets)
}

impl Render for VariablesOverview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh(window, cx);
        let list = self.list(window, cx);
        div()
            .relative()
            .size_full()
            .child(
                div()
                    .id("dock-variables-scroll")
                    .test_support()
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px_2()
                    .pb_2()
                    .child(list),
            )
            .vertical_scrollbar(&self.scroll)
    }
}

/// The dock body for the Variables tab: `view`, or nothing before the shell gives one.
pub fn dock_body(view: Option<&Entity<VariablesOverview>>) -> AnyElement {
    match view {
        Some(view) => div()
            .flex_1()
            .min_h_0()
            .child(view.clone())
            .into_any_element(),
        None => div().into_any_element(),
    }
}

impl VariablesOverview {
    fn list(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tainted = self
            .masking
            .as_ref()
            .map(Masking::tainted)
            .unwrap_or_default();
        let root = v_flex().id("variables-overview").gap_1().test_support();
        let muted = cx.theme().muted_foreground;
        if self.bound.is_none() {
            return root.child(
                div()
                    .id("var-overview-empty")
                    .test_support()
                    .child("Open a request to see its variables."),
            );
        }
        let Some(sections) = &self.sections else {
            return root.child(
                div()
                    .id("var-overview-failed")
                    .test_support()
                    .child("The variables could not be read. Check the environment and try again."),
            );
        };
        let mut children = Vec::new();
        if !self.undefined.is_empty() {
            children.push(
                div()
                    .id("var-undefined-heading")
                    .test_support()
                    .pt_2()
                    .text_color(cx.theme().warning)
                    .aria_label("Undefined")
                    .child("Undefined")
                    .into_any_element(),
            );
            for (i, name) in self.undefined.iter().enumerate() {
                children.push(self.undefined_row(i, name, window, cx));
            }
        }
        if sections.is_empty() && self.undefined.is_empty() {
            return root.child(
                div()
                    .id("var-overview-empty")
                    .test_support()
                    .child("No variables in scope."),
            );
        }
        let mut ix = 0;
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
                    self.row_element(ix, row, shown, window, cx)
                        .into_any_element(),
                );
                ix += 1;
            }
        }
        // Rows stop short of the overlay scrollbar so it never covers the "overridden by" label.
        root.pr(style::SCROLLBAR_GUTTER).children(children)
    }

    fn marked(&self, name: &str, cx: &App) -> Option<Hsla> {
        (self.focused.as_deref() == Some(name)).then(|| cx.theme().accent)
    }

    // An undefined name with an input that defines it; a secret with no stored value gets a
    // button to the environment editor instead, where secret values are kept.
    fn undefined_row(
        &self,
        i: usize,
        name: &str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let secret = self.unstored.get(name).copied();
        let input = (self.fields.get(&FieldKey::Undefined(name.to_owned()))).map(|f| &f.input);
        h_flex()
            .id(("var-undefined-row", i))
            .test_support()
            .items_center()
            .gap_2()
            .when_some(self.marked(name, cx), |d, bg| d.bg(bg))
            .child(
                div()
                    .id(("var-undefined-name", i))
                    .test_support()
                    .flex_none()
                    .w(NAME_WIDTH)
                    .min_w_0()
                    .whitespace_normal()
                    .aria_label(name.to_owned())
                    .child(name.to_owned()),
            )
            .children(input.map(|input| {
                let focused = input.focus_handle(cx).contains_focused(window, cx);
                let field =
                    ui::table_input(input, false, window, cx).id(("var-undefined-input", i));
                div().flex_1().min_w_0().child(kv_grid::kv_cell(
                    ("var-undefined-edge", i),
                    field,
                    input,
                    focused,
                    cx,
                ))
            }))
            .when_some(secret, |d, scope| {
                d.child(div().flex_1()).child(
                    ui::icon_button(
                        ("var-undefined-open", i),
                        IconName::ExternalLink,
                        "Set this secret in its editor",
                        None,
                        cx,
                    )
                    .on_click(
                        cx.listener(move |t, _, window, cx| t.open_editor(scope, "", window, cx)),
                    ),
                )
            })
            .into_any_element()
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

    // One row: name, value (masked when sensitive, an input when edited in place, else a
    // button to its scope's editor) and, when a higher scope wins, a label naming it; an
    // overridden value keeps to one line with the whole text in a tooltip.
    fn row_element(
        &self,
        ix: usize,
        row: &ScopedVariable,
        shown: Shown,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let look = crate::theme::shadowed_look(cx.theme());
        let key = FieldKey::Defined(scope_rank(row.scope()), row.name().to_owned());
        let input = (!row.is_shadowed())
            .then(|| self.fields.get(&key).map(|f| f.input.clone()))
            .flatten();
        let opens = !row.is_shadowed()
            && input.is_none()
            && !shown.eye
            && !matches!(row.scope(), VarScope::IterationData | VarScope::Dynamic);
        let shadowed = row.is_shadowed();
        let value = shown.text.clone();
        let (id, icon, words) = if shown.revealed {
            ("var-overview-hide", IconName::EyeOff, "Hide value")
        } else {
            ("var-overview-reveal", IconName::Eye, "Show value")
        };
        let now = shown.revealed;
        let reveal = reveal_key(row);
        let (scope, name) = (row.scope(), row.name().to_owned());
        let marked = self.marked(row.name(), cx);
        h_flex()
            .id(("var-overview-row", ix))
            .test_support()
            .items_start()
            .gap_2()
            .when(shadowed, |d| {
                d.opacity(look.0)
                    .when_some(look.1, |d, color| d.text_color(color))
            })
            .when_some(marked, |d, bg| d.bg(bg))
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
                    .when(!shadowed, |d| d.whitespace_normal())
                    .when(shadowed, |d| {
                        d.line_through().truncate().tooltip(ui::text_tooltip(
                            "var-overview-value-tooltip",
                            value.clone(),
                        ))
                    })
                    .aria_label(value.clone())
                    .map(|d| match &input {
                        Some(input) => {
                            let focused = input.focus_handle(cx).contains_focused(window, cx);
                            let field = ui::table_input(input, false, window, cx)
                                .id(("var-overview-input", ix));
                            d.child(kv_grid::kv_cell(
                                ("var-overview-edge", ix),
                                field,
                                input,
                                focused,
                                cx,
                            ))
                        }
                        None => d.child(value),
                    }),
            )
            .when(opens, |d| {
                d.child(
                    ui::icon_button(
                        ("var-overview-open", ix),
                        IconName::ExternalLink,
                        "Open its editor",
                        None,
                        cx,
                    )
                    .on_click(
                        cx.listener(move |t, _, window, cx| {
                            t.open_editor(scope, &name, window, cx)
                        }),
                    ),
                )
            })
            .when(shown.eye, |d| {
                d.child(
                    ui::swap_icon_button(("var-overview-eye", ix), (id, ix), icon, words, cx)
                        .on_click(cx.listener(move |t, _, _, cx| {
                            if now {
                                t.revealed.remove(&reveal);
                            } else {
                                t.revealed.insert(reveal.clone());
                            }
                            cx.notify();
                        })),
                )
            })
            .when(shadowed, |d| {
                let note = match self.winners.get(row.name()) {
                    Some(scope) => format!("overridden by {}", heading(*scope).to_lowercase()),
                    None => "overridden".to_owned(),
                };
                d.child(
                    div()
                        .id(("var-overview-shadowed", ix))
                        .test_support()
                        .flex_none()
                        .text_size(style::TEXT_SMALL)
                        .text_color(muted)
                        .aria_label(note.clone())
                        .child(note),
                )
            })
    }
}
