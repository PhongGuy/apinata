//! The GraphQL query builder: schema fields as a tree of rows ticked from the query text.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use apinata_core::{
    FieldPath, Lock, QUERY_VALIDATE_LIMIT, QueryEdit, QueryOutline, Schema, TypeKind,
    query_outline, toggle_field,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::EditorState;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, Entity, EventEmitter, Result, Subscription, TestSupportExt, Window, div,
    uniform_list,
};

use crate::graphql_docs::{StateIds, schema_state};
use crate::graphql_schema_store::{self, SchemaState, SchemaStore};
use crate::graphql_section::SchemaAction;
use crate::{style, ui};

/// The note while the query text does not parse.
pub const BROKEN_NOTE: &str = "Fix the query to edit it here";

/// The note while the query text is over the size the builder reads.
pub const TOO_LARGE_NOTE: &str = "Too large to edit here";

/// Why the builder cannot read `text`, given that it has no outline.
pub fn unavailable_note(text: &str) -> &'static str {
    if text.len() > QUERY_VALIDATE_LIMIT {
        TOO_LARGE_NOTE
    } else {
        BROKEN_NOTE
    }
}

/// What a row stands for, which decides its chevron and checkbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A root operation type: a chevron, no checkbox.
    Root,
    /// A scalar or enum field, or one whose type is missing.
    Leaf,
    /// An object or interface field with fields of its own.
    Composite,
    /// A union field.
    Union,
}

/// One line of the builder tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuilderRow {
    /// The root type name and the field names down to this row.
    pub path: FieldPath,
    /// 0 for a root type.
    pub depth: usize,
    /// `query: Query` for a root, else the field name.
    pub label: String,
    /// The field's type as written in SDL; empty for a root.
    pub ty: String,
    /// What the row stands for.
    pub kind: RowKind,
    /// Whether the field is deprecated.
    pub deprecated: bool,
    /// Whether the row's children are listed.
    pub expanded: bool,
    /// Whether the query selects the path; false for roots and broken text.
    pub checked: bool,
    /// Why the row cannot be toggled.
    pub lock: Option<String>,
}

/// Root types in query, mutation, subscription order; children only under expanded paths.
/// `outline` is the query's outline, or the note that locks every field when there is none.
pub fn rows(
    schema: &Schema,
    outline: Result<&QueryOutline, &str>,
    expanded: &HashSet<FieldPath>,
    show_deprecated: bool,
) -> Vec<BuilderRow> {
    let walk = Walk {
        schema,
        outline,
        expanded,
        show_deprecated,
    };
    let roots = [
        ("query", schema.query_type()),
        ("mutation", schema.mutation_type()),
        ("subscription", schema.subscription_type()),
    ];
    let mut out = Vec::new();
    for (op, name) in roots {
        let Some(name) = name else { continue };
        let path = FieldPath::new([name]);
        let open = expanded.contains(&path);
        // The panel shows a missing outline's note once above the rows, so roots skip it.
        out.push(BuilderRow {
            lock: walk
                .outline
                .ok()
                .and_then(|o| o.locked(&path))
                .map(|l| lock_text(&l)),
            path: path.clone(),
            depth: 0,
            label: format!("{op}: {name}"),
            ty: String::new(),
            kind: RowKind::Root,
            deprecated: false,
            expanded: open,
            checked: false,
        });
        if open {
            walk.children(&mut out, &path, name, 1);
        }
    }
    out
}

struct Walk<'a, 's> {
    schema: &'a Schema,
    outline: Result<&'a QueryOutline<'s>, &'a str>,
    expanded: &'a HashSet<FieldPath>,
    show_deprecated: bool,
}

impl Walk<'_, '_> {
    fn lock(&self, path: &FieldPath) -> Option<String> {
        match self.outline {
            Err(note) => Some(note.to_owned()),
            Ok(o) => o.locked(path).map(|l| lock_text(&l)),
        }
    }

    // Recursion follows only expanded paths, so a self-referencing type stays finite.
    fn children(&self, out: &mut Vec<BuilderRow>, parent: &FieldPath, ty: &str, depth: usize) {
        let Some(ty) = self.schema.type_named(ty) else {
            return;
        };
        for field in ty.fields() {
            let path = FieldPath::new(parent.names().iter().chain([&field.name().to_owned()]));
            let checked = self.outline.is_ok_and(|o| o.selects(&path));
            // A selected deprecated field always shows, so it can still be unticked.
            let deprecated = field.deprecation().is_some();
            if deprecated && !self.show_deprecated && !checked {
                continue;
            }
            let named = field.ty().named();
            let kind = match self.schema.type_named(named).map(|t| t.kind()) {
                None | Some(TypeKind::Scalar | TypeKind::Enum) => RowKind::Leaf,
                Some(TypeKind::Union) => RowKind::Union,
                Some(_) => RowKind::Composite,
            };
            let open = self.expanded.contains(&path);
            out.push(BuilderRow {
                lock: self.lock(&path),
                checked,
                path: path.clone(),
                depth,
                label: field.name().to_owned(),
                ty: field.ty().display().to_owned(),
                kind,
                deprecated,
                expanded: open,
            });
            if open && kind == RowKind::Composite {
                self.children(out, &path, named, depth + 1);
            }
        }
    }
}

/// The note shown for a row the builder will not toggle.
pub fn lock_text(lock: &Lock) -> String {
    match lock {
        Lock::OtherRoot => "One operation per request".into(),
        Lock::SubscriptionField => "A subscription selects one field".into(),
        Lock::Union => "Union fields need inline fragments".into(),
        Lock::LastChildOf(n) => format!("Last field of `{n}`; untick `{n}` to remove it"),
        Lock::LastOfOperation(n) => {
            format!("Last field of operation `{n}`; edit the query to remove it")
        }
        _ => "Not editable here".into(),
    }
}

/// Where a caret at `caret` lands after `range` is replaced by `inserted` bytes.
pub fn caret_after(caret: usize, range: Range<usize>, inserted: usize) -> usize {
    if caret <= range.start {
        caret
    } else if caret >= range.end {
        caret - range.len() + inserted
    } else {
        range.start
    }
}

const BUILDER_IDS: StateIds = StateIds {
    hint: "graphql-builder-hint",
    load: "graphql-builder-load",
    loading: "graphql-builder-loading",
    error: "graphql-builder-error",
    hint_text: "Load the schema to build a query.",
};

/// The builder panel for one request URL: the schema's fields as rows ticked from the query
/// editor's text. It emits `SchemaAction::Load` when the user asks for a schema it lacks.
pub struct BuilderPanel {
    query: Entity<EditorState>,
    url: String,
    expanded: HashSet<FieldPath>,
    show_deprecated: bool,
    rows: Vec<BuilderRow>,
    // Why the builder cannot read the text, shown once above the rows.
    note: Option<&'static str>,
    // The text and schema the rows were built from; `None` forces a rebuild.
    built: Option<(String, Arc<Schema>)>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<SchemaAction> for BuilderPanel {}

impl BuilderPanel {
    /// A collapsed panel over `query`, keyed on no URL yet.
    pub fn new(query: Entity<EditorState>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        // A programmatic load emits no change event, so the editor is observed instead.
        let subs = vec![
            cx.observe(&query, |_, _, cx| cx.notify()),
            cx.observe_global::<SchemaStore>(|_, cx| cx.notify()),
        ];
        BuilderPanel {
            query,
            url: String::new(),
            expanded: HashSet::new(),
            show_deprecated: false,
            rows: Vec::new(),
            note: None,
            built: None,
            _subs: subs,
        }
    }

    /// Key the panel on `url`, the request's raw URL.
    pub fn set_url(&mut self, url: String, cx: &mut Context<Self>) {
        if self.url != url {
            self.url = url;
            cx.notify();
        }
    }

    /// Lists or hides the children of `path`.
    pub fn toggle_expanded(&mut self, path: &FieldPath, cx: &mut Context<Self>) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.clone());
        }
        self.built = None;
        cx.notify();
    }

    /// Whether the children of `path` are listed.
    pub fn is_expanded(&self, path: &FieldPath) -> bool {
        self.expanded.contains(path)
    }

    /// The rows as of the last render.
    pub fn rows(&self) -> &[BuilderRow] {
        &self.rows
    }

    /// Applies `edit` to the query editor as one undoable replacement, only when the editor
    /// still holds `snapshot`; restores the caret per `caret_after` and keeps the scroll.
    /// False when dropped.
    pub fn apply(
        &mut self,
        snapshot: &str,
        edit: QueryEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.query.update(cx, |e, cx| {
            if *e.value() != *snapshot {
                return false;
            }
            // Selecting the edit range scrolls to it, so the view the user left is put back.
            let scroll = e.scroll_offset();
            let caret = caret_after(e.cursor(), edit.range.clone(), edit.text.len());
            e.set_selected_range(edit.range, cx);
            e.replace(edit.text, window, cx);
            e.set_selected_range(caret..caret, cx);
            e.set_scroll_offset(scroll, cx);
            true
        })
    }

    // A click decides tick or untick from the current text, never the rendered row: an
    // unselected composite only expands, anything else is toggled in the query.
    fn click(
        &mut self,
        path: &FieldPath,
        kind: RowKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(SchemaState::Loaded(schema)) = graphql_schema_store::state(cx, &self.url) else {
            return;
        };
        let snapshot = self.query.read(cx).value().to_string();
        let Some(outline) = query_outline(&snapshot, &schema) else {
            return;
        };
        let selected = outline.selects(path);
        if kind == RowKind::Composite && !selected {
            self.expanded.insert(path.clone());
            self.built = None;
            cx.notify();
            return;
        }
        if let Some(edit) = toggle_field(&snapshot, &schema, path, !selected) {
            self.apply(&snapshot, edit, window, cx);
        }
    }

    // Rebuild only when the text or schema moved; expansion and the toggle clear `built`.
    fn refresh(&mut self, schema: Arc<Schema>, cx: &App) {
        let text = self.query.read(cx).value().to_string();
        if let Some((t, s)) = &self.built
            && *t == text
            && Arc::ptr_eq(s, &schema)
        {
            return;
        }
        let outline = query_outline(&text, &schema);
        let note = unavailable_note(&text);
        self.note = outline.is_none().then_some(note);
        self.rows = rows(
            &schema,
            outline.as_ref().ok_or(note),
            &self.expanded,
            self.show_deprecated,
        );
        self.built = Some((text, schema));
    }

    fn loaded(&mut self, schema: Arc<Schema>, cx: &mut Context<Self>) -> Div {
        self.refresh(schema, cx);
        let note = self.note.map(|note| {
            div()
                .id("graphql-builder-note")
                .text_size(style::TEXT_SMALL)
                .text_color(cx.theme().muted_foreground)
                .aria_label(note)
                .child(note)
                .test_support()
        });
        let toggle = ui::checkbox("graphql-builder-deprecated", self.show_deprecated)
            .label("Show deprecated")
            .on_click(cx.listener(|this, on: &bool, _, cx| {
                this.show_deprecated = *on;
                this.built = None;
                cx.notify();
            }));
        let list = uniform_list(
            "graphql-builder-rows",
            self.rows.len(),
            cx.processor(|this: &mut Self, range: Range<usize>, _, cx| {
                range.map(|ix| this.row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .size_full()
        .test_support();
        v_flex()
            .size_full()
            .gap_2()
            .children(note)
            .child(toggle)
            .child(div().flex_1().min_h_0().child(list))
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let row = &self.rows[ix];
        let muted = cx.theme().muted_foreground;
        let chevron = matches!(row.kind, RowKind::Root | RowKind::Composite).then(|| {
            let path = row.path.clone();
            Button::new(("graphql-builder-chevron", ix))
                .ghost()
                .xsmall()
                .icon(if row.expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_expanded(&path, cx)))
        });
        let check = (row.kind != RowKind::Root).then(|| {
            let (path, kind) = (row.path.clone(), row.kind);
            let check = ui::checkbox(("graphql-builder-check", ix), row.checked)
                .disabled(row.lock.is_some())
                .on_click(cx.listener(move |this, _: &bool, window, cx| {
                    this.click(&path, kind, window, cx)
                }))
                .accessible_name(row.lock.clone().unwrap_or_else(|| row.label.clone()));
            match &row.lock {
                Some(lock) => check.tooltip(lock.clone()),
                None => check,
            }
        });
        let ty = (!row.ty.is_empty()).then(|| {
            div()
                .id(("graphql-builder-type", ix))
                .test_support()
                .aria_label(row.ty.clone())
                .text_color(muted)
                .child(row.ty.clone())
        });
        let deprecated = row
            .deprecated
            .then(|| div().text_color(muted).child("(deprecated)"));
        // Only a root shows its lock as text; a field's lock is on its disabled box.
        let root_lock = (row.kind == RowKind::Root)
            .then(|| row.lock.clone())
            .flatten()
            .map(|lock| div().text_color(muted).child(lock));
        h_flex()
            .id(("graphql-builder-row", ix))
            .gap_1()
            .items_center()
            .pl(style::ui(16. * row.depth as f32))
            .whitespace_nowrap()
            .text_size(style::TEXT_SMALL)
            .children(chevron)
            .children(check)
            .child(row.label.clone())
            .children(ty)
            .children(deprecated)
            .children(root_lock)
            .test_support()
            .into_any_element()
    }
}

impl Render for BuilderPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match schema_state(&self.url, &BUILDER_IDS, cx) {
            Ok(schema) => self.loaded(schema, cx).into_any_element(),
            Err(state) => state,
        };
        v_flex()
            .id("graphql-builder")
            .test_support()
            .size_full()
            .gap_2()
            .child(body)
    }
}
