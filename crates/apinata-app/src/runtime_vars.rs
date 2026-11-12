//! The runtime variables dialog: lists one collection's post-response captures, then the
//! environment and collection variables its scripts set, then the global ones under their
//! own heading. Every row has an editable value (saved on Enter or blur) and a Delete button,
//! an Add row creates a captured variable, and Clear all forgets them all. Values are shown as
//! the text `{{name}}` interpolates to; a masked row's field stays empty until revealed.

use crate::env_table::{ACTION_SLOT_WIDTH, SECRET_HIDDEN};
use crate::kv_grid;
use crate::style;
use crate::ui;
use apinata_core::{EnvOverlay, OverlayLayer, RuntimeVars};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, Entity, Focusable, Hsla, Rems, ScrollHandle, Subscription,
    TestSupportExt, Window, div, px, rems,
};
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

// What runtime variables are, shown above the rows whether or not any exist.
const ABOUT_NOTE: &str = "Values set by scripts and by responses while the app is open. They are \
used instead of environment values with the same name and are never saved to files.";

// The name column both row kinds share, so their value columns start at one x.
const NAME_WIDTH: Rems = rems(7.);

// Which list a row belongs to; a name can be captured and set by a script in several layers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Captured,
    Script,
    Global,
}

// A script or global row as shown, its value already masked unless revealed.
struct ShownRow {
    name: String,
    value: String,
    note: String,
}

// The overlay rows as shown, split by layer: environment and collection, then global.
struct ShownRows {
    script: Vec<ShownRow>,
    global: Vec<ShownRow>,
}

// Identifies one row's field: its kind, the overlay layer (0 for captured rows) and its name.
type FieldKey = (Kind, u8, String);

// One row as stored: where it lives, its plain value, its note and whether it is masked.
struct Row {
    kind: Kind,
    layer: Option<OverlayLayer>,
    name: String,
    plain: String,
    note: String,
    masked: bool,
}

impl Row {
    fn key(&self) -> FieldKey {
        (self.kind, layer_rank(self.layer), self.name.clone())
    }
}

fn layer_rank(layer: Option<OverlayLayer>) -> u8 {
    match layer {
        None | Some(OverlayLayer::Environment) => 0,
        Some(OverlayLayer::Collection) => 1,
        Some(_) => 2,
    }
}

// A row's value input and the subscription that saves it.
struct Field {
    input: Entity<InputState>,
    holder: &'static str,
    // The text this view last put in the field; a field that differs holds the person's edit.
    synced: String,
    _sub: Subscription,
}

/// The one rule for which runtime values are hidden, shared by the editable dialog and the
/// read-only overview: a row is masked by a secret's name, by a mark a person left on a
/// captured variable, or by holding a value a script read as sensitive.
#[derive(Clone)]
pub struct Masking {
    vars: RuntimeVars,
    secrets: Vec<String>,
    global_secrets: Vec<String>,
}

impl Masking {
    /// The rule over the `vars` store, the selected environment's `secrets` and the active
    /// global environment's `global_secrets`.
    pub fn new(vars: RuntimeVars, secrets: Vec<String>, global_secrets: Vec<String>) -> Self {
        Masking {
            vars,
            secrets,
            global_secrets,
        }
    }

    /// The values scripts read as sensitive, read once per pass over many rows.
    pub fn tainted(&self) -> Vec<String> {
        self.vars.tainted()
    }

    /// Whether the captured variable `name` holding `value` is hidden.
    pub fn captured(&self, name: &str, value: Option<&str>, tainted: &[String]) -> bool {
        self.vars.is_sensitive(name) || self.scripted(name, value, tainted, false)
    }

    /// Whether a row of another scope is hidden; a `global` one also by the global secrets.
    pub fn scripted(
        &self,
        name: &str,
        value: Option<&str>,
        tainted: &[String],
        global: bool,
    ) -> bool {
        self.secrets.iter().any(|s| s == name)
            || (global && self.global_secrets.iter().any(|s| s == name))
            || value.is_some_and(|text| {
                (tainted.iter()).any(|t| !t.is_empty() && text.contains(t.as_str()))
            })
    }

    /// The captured variable `name`'s stored text, whatever its masking.
    pub fn captured_text(&self, name: &str) -> Option<String> {
        (self.vars.entries().into_iter())
            .find(|v| v.name() == name)
            .and_then(|v| v.value().map(str::to_owned))
    }
}

/// The dialog body over the shared stores, re-read on every render so captures made while it
/// is open appear.
pub struct RuntimeVarsView {
    vars: RuntimeVars,
    overlay: EnvOverlay,
    masking: Masking,
    revealed: BTreeSet<(Kind, String)>,
    // Script rows that were masked when a value was typed into them; they stay masked.
    sticky: BTreeSet<(Kind, String)>,
    fields: BTreeMap<FieldKey, Field>,
    // Rows whose field must show their stored text again even while focused (a reveal).
    resync: BTreeSet<(Kind, String)>,
    add_name: Entity<InputState>,
    add_value: Entity<InputState>,
    _add_subs: Vec<Subscription>,
}

impl RuntimeVarsView {
    fn new(
        vars: RuntimeVars,
        overlay: EnvOverlay,
        secrets: Vec<String>,
        global_secrets: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let add_name = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        let add_value = cx.new(|cx| InputState::new(window, cx).placeholder("Value"));
        let subs = [&add_name, &add_value].map(|input| {
            cx.subscribe_in(
                input,
                window,
                |this: &mut RuntimeVarsView, _, event: &InputEvent, window, cx| match event {
                    // An add by Enter readies the row for the next variable.
                    InputEvent::PressEnter { .. } => {
                        if this.add(window, cx) {
                            this.add_name.focus_handle(cx).focus(window, cx);
                        }
                    }
                    // Tabbing from the name to the value must not add a half-typed variable,
                    // so a blur adds only once focus has settled outside both fields.
                    InputEvent::Blur => cx.defer_in(window, |this, window, cx| {
                        let inside = [&this.add_name, &this.add_value]
                            .iter()
                            .any(|i| i.focus_handle(cx).is_focused(window));
                        if !inside {
                            this.add(window, cx);
                        }
                    }),
                    _ => {}
                },
            )
        });
        let vars_for_masking = vars.clone();
        RuntimeVarsView {
            vars,
            overlay,
            masking: Masking::new(vars_for_masking, secrets, global_secrets),
            revealed: BTreeSet::new(),
            sticky: BTreeSet::new(),
            fields: BTreeMap::new(),
            resync: BTreeSet::new(),
            add_name,
            add_value,
            _add_subs: subs.into(),
        }
    }

    /// `(name, value or "undefined")` for every variable, by name; a masked row that is not
    /// revealed shows `SECRET_HIDDEN` as its value.
    pub fn rows(&self) -> Vec<(String, String)> {
        (self.captured_entries().into_iter())
            .map(|(name, value, _)| (name, value))
            .collect()
    }

    /// The value field of captured row `ix`, once drawn.
    pub fn captured_input(&self, ix: usize) -> Option<&Entity<InputState>> {
        let name = self.vars.entries().get(ix)?.name().to_owned();
        self.fields
            .get(&(Kind::Captured, 0, name))
            .map(|f| &f.input)
    }

    // Every stored row: captured first, then the overlay's, each with its masking.
    fn all_rows(&self) -> Vec<Row> {
        let tainted = self.masking.tainted();
        let captured = (self.vars.entries().into_iter()).map(|v| Row {
            kind: Kind::Captured,
            layer: None,
            masked: self.masking.captured(v.name(), v.value(), &tainted),
            name: v.name().to_owned(),
            plain: v.value().unwrap_or("undefined").to_owned(),
            note: String::new(),
        });
        let overlay = (self.overlay.entries().into_iter()).map(|v| {
            let plain = match v.value() {
                Some(text) => text,
                None if v.is_deleted() => "deleted",
                None => "undefined",
            };
            let global = v.layer() == OverlayLayer::Global;
            Row {
                kind: if global { Kind::Global } else { Kind::Script },
                layer: Some(v.layer()),
                masked: self.masking.scripted(v.name(), v.value(), &tainted, global)
                    || self.sticky.contains(&(
                        if global { Kind::Global } else { Kind::Script },
                        v.name().to_owned(),
                    )),
                name: v.name().to_owned(),
                plain: plain.to_owned(),
                note: overlay_note(v.layer()).to_owned(),
            }
        });
        captured.chain(overlay).collect()
    }

    // The captured rows as shown, each with whether it is masked.
    fn captured_entries(&self) -> Vec<(String, String, bool)> {
        (self.all_rows().into_iter())
            .filter(|r| r.kind == Kind::Captured)
            .map(|r| {
                let shown = self.shown(r.kind, &r.name, r.plain, r.masked);
                (r.name, shown, r.masked)
            })
            .collect()
    }

    /// `(name, value | "undefined" | "deleted", note)` per environment and collection overlay
    /// entry, environment first; global entries are listed by `global_rows`. A masked row that
    /// is not revealed shows `SECRET_HIDDEN` as its value.
    pub fn script_rows(&self) -> Vec<(String, String, String)> {
        (self.shown_entries().script.into_iter())
            .map(|r| (r.name, r.value, r.note))
            .collect()
    }

    /// `(name, value | "undefined" | "deleted")` per global overlay entry, masked like the
    /// script rows.
    pub fn global_rows(&self) -> Vec<(String, String)> {
        (self.shown_entries().global.into_iter())
            .map(|r| (r.name, r.value))
            .collect()
    }

    // The script rows and then the global rows as shown, from one overlay read.
    fn shown_entries(&self) -> ShownRows {
        let (mut script, mut global) = (Vec::new(), Vec::new());
        for r in self.all_rows() {
            let list = match r.kind {
                Kind::Captured => continue,
                Kind::Script => &mut script,
                Kind::Global => &mut global,
            };
            let value = self.shown(r.kind, &r.name, r.plain, r.masked);
            list.push(ShownRow {
                name: r.name,
                value,
                note: r.note,
            });
        }
        ShownRows { script, global }
    }

    // `value`, or `SECRET_HIDDEN` while the row is masked and not revealed.
    fn shown(&self, kind: Kind, name: &str, value: String, masked: bool) -> String {
        if masked && !self.revealed.contains(&(kind, name.to_owned())) {
            SECRET_HIDDEN.to_owned()
        } else {
            value
        }
    }

    // Shows or masks again the `kind` row named `name`.
    fn set_revealed(&mut self, kind: Kind, name: String, on: bool, cx: &mut Context<Self>) {
        self.resync.insert((kind, name.clone()));
        if on {
            self.revealed.insert((kind, name));
        } else {
            self.revealed.remove(&(kind, name));
        }
        cx.notify();
    }

    /// Forgets every runtime variable and the script-set variables of the collection, the
    /// global environment and the shown environment; other environments keep theirs.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.vars.clear();
        self.overlay.clear_layer(OverlayLayer::Environment);
        self.overlay.clear_layer(OverlayLayer::Collection);
        self.overlay.clear_layer(OverlayLayer::Global);
        cx.notify();
    }

    // Forgets the one variable a row shows; a script row's file value shows through again.
    fn delete(
        &mut self,
        kind: Kind,
        layer: Option<OverlayLayer>,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        match (kind, layer) {
            (Kind::Captured, _) | (_, None) => {
                self.vars.unset(name);
            }
            (_, Some(layer)) => {
                self.overlay.remove(layer, name);
            }
        }
        cx.notify();
    }

    // What a row's field holds while nobody is editing it: nothing for a masked row that is
    // not revealed, so the value never enters the field.
    fn field_text(&self, row: &Row) -> String {
        if row.masked && !self.revealed.contains(&(row.kind, row.name.clone())) {
            String::new()
        } else {
            row.plain.clone()
        }
    }

    // Saves the field of row `key`: a plain row when its text changed, a hidden masked row
    // when anything was typed (then the field empties again).
    fn commit(&mut self, key: &FieldKey, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.fields.get(key).map(|f| f.input.clone()) else {
            return;
        };
        let Some(row) = self.all_rows().into_iter().find(|r| &r.key() == key) else {
            return;
        };
        let typed = input.read(cx).value().to_string();
        let hidden = row.masked && !self.revealed.contains(&(row.kind, row.name.clone()));
        if typed == self.field_text(&row) {
            self.mark_synced(key, typed);
            return;
        }
        match row.layer {
            None => self.vars.set_text(&row.name, typed.clone()),
            Some(layer) => self.overlay.set_text(layer, &row.name, typed.clone()),
        }
        // Typing into a masked row keeps it masked: a captured variable is marked sensitive in
        // the shared store (so a reopened dialog masks it), a script row in this view.
        if row.masked {
            if row.kind == Kind::Captured {
                self.vars.mark_sensitive(&row.name);
            } else {
                self.sticky.insert((row.kind, row.name.clone()));
            }
        }
        self.mark_synced(key, if hidden { String::new() } else { typed });
        if hidden {
            input.update(cx, |i, cx| i.set_value("", window, cx));
        }
        cx.notify();
    }

    /// Saves every field's typing and adds the add row's variable, as Enter in each would.
    pub fn commit_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keys: Vec<FieldKey> = self.fields.keys().cloned().collect();
        for key in &keys {
            self.commit(key, window, cx);
        }
        self.add(window, cx);
    }

    // Records that `key`'s field now matches the store, so it is no longer an unsaved edit.
    fn mark_synced(&mut self, key: &FieldKey, text: String) {
        if let Some(field) = self.fields.get_mut(key) {
            field.synced = text;
        }
    }

    // Adds the typed name and value as a captured variable when the name is not empty, and
    // says whether it did.
    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let name = self.add_name.read(cx).value().trim().to_owned();
        if name.is_empty() {
            return false;
        }
        let value = self.add_value.read(cx).value().to_string();
        self.vars.set_text(name, value);
        for input in [&self.add_name, &self.add_value] {
            input.update(cx, |i, cx| i.set_value("", window, cx));
        }
        cx.notify();
        true
    }

    // Creates a field for each new row, drops the fields of vanished rows, and brings every
    // field not being edited back to its row's stored value.
    fn sync_fields(&mut self, rows: &[Row], window: &mut Window, cx: &mut Context<Self>) {
        let keys: BTreeSet<_> = rows.iter().map(Row::key).collect();
        let forced = std::mem::take(&mut self.resync);
        self.fields.retain(|k, _| keys.contains(k));
        for row in rows {
            let key = row.key();
            let text = self.field_text(row);
            let hidden = row.masked && text.is_empty();
            let holder = if hidden { SECRET_HIDDEN } else { "" };
            // A row seen for the first time gets its input and Enter/blur subscription here.
            let field = match self.fields.entry(key.clone()) {
                Entry::Occupied(slot) => slot.into_mut(),
                Entry::Vacant(slot) => {
                    let input = cx.new(|cx| {
                        InputState::new(window, cx)
                            .placeholder(holder)
                            .default_value(text.clone())
                    });
                    let at = key.clone();
                    let sub = cx.subscribe_in(
                        &input,
                        window,
                        move |this: &mut RuntimeVarsView, _, event: &InputEvent, window, cx| {
                            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                                this.commit(&at, window, cx);
                            }
                        },
                    );
                    slot.insert(Field {
                        input,
                        holder,
                        synced: text.clone(),
                        _sub: sub,
                    })
                }
            };
            // Writing an input notifies it, so only touch what differs or every frame is dirty.
            let input = field.input.clone();
            let fresh_holder = field.holder != holder;
            field.holder = holder;
            // An unsaved edit stays until it is saved; a reveal shows the stored text anyway.
            let force = forced.contains(&(row.kind, row.name.clone()));
            let edited = input.read(cx).value().as_ref() != field.synced;
            if fresh_holder || !edited || force {
                field.synced = text.clone();
                if fresh_holder {
                    input.update(cx, |i, cx| i.set_placeholder(holder, window, cx));
                }
                ui::sync_input_text(&input, &text, window, cx);
            }
        }
    }

    // The fixed action slot of a row: the eye button that shows a masked value or hides a
    // revealed one, else empty, so the columns line up with or without a button in it.
    fn slot(
        &self,
        kind: Kind,
        ix: usize,
        name: &str,
        masked: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (slot, eye, reveal, hide) = match kind {
            Kind::Captured => (
                "runtime-var-action-slot",
                "runtime-var-eye",
                "runtime-var-reveal",
                "runtime-var-hide",
            ),
            Kind::Script => (
                "script-var-action-slot",
                "script-var-eye",
                "script-var-reveal",
                "script-var-hide",
            ),
            Kind::Global => (
                "global-var-action-slot",
                "global-var-eye",
                "global-var-reveal",
                "global-var-hide",
            ),
        };
        let revealed = masked && self.revealed.contains(&(kind, name.to_owned()));
        let (id, icon, words) = if revealed {
            (hide, IconName::EyeOff, "Hide value")
        } else {
            (reveal, IconName::Eye, "Show value")
        };
        let n = name.to_owned();
        h_flex()
            .id((slot, ix))
            .flex_none()
            .w(ACTION_SLOT_WIDTH)
            .when(masked, |d| {
                d.child(
                    ui::swap_icon_button((eye, ix), (id, ix), icon, words)
                        .key_context(ui::DIALOG_FOOTER_CONTEXT)
                        .on_click(cx.listener(move |t, _, _, cx| {
                            t.set_revealed(kind, n.clone(), !revealed, cx)
                        })),
                )
            })
            .test_support()
    }

    // One row: name, editable value (with a script row's note under it), eye slot and Delete.
    fn render_row(
        &self,
        row: &Row,
        ix: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (row_id, name_id, value_id, edge, delete_id) = match row.kind {
            Kind::Captured => (
                "runtime-var",
                "runtime-var-name",
                "runtime-var-value",
                "runtime-var-edge",
                "runtime-var-delete",
            ),
            Kind::Script => (
                "script-var",
                "script-var-name",
                "script-var-value",
                "script-var-edge",
                "script-var-delete",
            ),
            Kind::Global => (
                "global-var",
                "global-var-name",
                "global-var-value",
                "global-var-edge",
                "global-var-delete",
            ),
        };
        let muted = cx.theme().muted_foreground;
        let input = &self.fields[&row.key()].input;
        let focused = input.focus_handle(cx).contains_focused(window, cx);
        let field = ui::table_input(input, false, window, cx).id((value_id, ix));
        let (kind, layer, name) = (row.kind, row.layer, row.name.clone());
        // A script row's note sits under its value, so every kind shares one value column
        // that takes the row's width; the name and slots stay at the row's top.
        h_flex()
            .id((row_id, ix))
            .test_support()
            .items_start()
            .gap_2()
            .child(cell((name_id, ix), row.name.clone(), true, None))
            .child(
                value_column()
                    .child(kv_grid::kv_cell_resting(
                        (edge, ix),
                        field,
                        input,
                        focused,
                        cx,
                    ))
                    .when(row.kind == Kind::Script, |d| {
                        d.child(div().text_size(style::TEXT_SMALL).child(cell(
                            ("script-var-note", ix),
                            row.note.clone(),
                            false,
                            Some(muted),
                        )))
                    }),
            )
            .child(self.slot(row.kind, ix, &row.name, row.masked, cx))
            .child(
                div().flex_none().w(ACTION_SLOT_WIDTH).child(
                    // Each row button takes Enter as its press; the row's field keeps its own.
                    ui::icon_button((delete_id, ix), IconName::Close, "Remove variable", None)
                        .key_context(ui::DIALOG_FOOTER_CONTEXT)
                        .on_click(cx.listener(move |t, _, _, cx| t.delete(kind, layer, &name, cx))),
                ),
            )
    }

    // The Add row: a name, a value and the button that adds them as a captured variable.
    fn render_add(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cell = |input: &Entity<InputState>, id: &'static str, edge: &'static str, cx: &App| {
            let focused = input.focus_handle(cx).contains_focused(window, cx);
            let field = ui::table_input(input, false, window, cx).id(id);
            kv_grid::kv_cell_resting(edge, field, input, focused, cx)
        };
        h_flex()
            .id("runtime-vars-add-row")
            .test_support()
            .items_start()
            .gap_2()
            .child(div().flex_none().w(NAME_WIDTH).child(cell(
                &self.add_name,
                "runtime-vars-add-name",
                "runtime-vars-add-name-edge",
                cx,
            )))
            .child(value_column().child(cell(
                &self.add_value,
                "runtime-vars-add-value",
                "runtime-vars-add-value-edge",
                cx,
            )))
            .child(div().flex_none().w(ACTION_SLOT_WIDTH))
            .child(
                div().flex_none().w(ACTION_SLOT_WIDTH).child(
                    ui::icon_button("runtime-vars-add", IconName::Plus, "Add variable", None)
                        .key_context(ui::DIALOG_FOOTER_CONTEXT)
                        .on_click(cx.listener(|t, _, window, cx| {
                            t.add(window, cx);
                        })),
                ),
            )
    }
}

impl Render for RuntimeVarsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = v_flex().id("runtime-vars").gap_2().test_support();
        let rows = self.all_rows();
        self.sync_fields(&rows, window, cx);
        let muted = cx.theme().muted_foreground;
        let empty = rows.is_empty();
        let mut lists: [Vec<AnyElement>; 3] = Default::default();
        let mut counts = [0usize; 3];
        for row in &rows {
            let list = row.kind as usize;
            let ix = counts[list];
            counts[list] += 1;
            lists[list].push(self.render_row(row, ix, window, cx).into_any_element());
        }
        let [captured, scripted, global] = lists;
        // Global values follow under their own heading; the layer is the heading, so no note.
        let heading = (!global.is_empty()).then(|| {
            div()
                .id("runtime-vars-global-heading")
                .test_support()
                .pt_2()
                .text_color(muted)
                .child("Global environment")
        });
        // The note opens the body, so a full list explains itself as an empty one does.
        let about = div()
            .id("runtime-vars-about")
            .text_size(style::TEXT_SMALL)
            .text_color(muted)
            .aria_label(ABOUT_NOTE)
            .child(ABOUT_NOTE)
            .test_support();
        root.child(about)
            .when(empty, |d| {
                d.child(
                    div()
                        .id("runtime-vars-empty")
                        .test_support()
                        .child("No runtime variables yet."),
                )
            })
            .children(captured)
            .children(scripted)
            .children(heading)
            .children(global)
            .child(self.render_add(window, cx))
            .when(!empty, |d| {
                d.child(
                    Button::new("runtime-vars-clear")
                        .key_context(ui::DIALOG_FOOTER_CONTEXT)
                        .label("Clear all")
                        .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
                )
            })
    }
}

// The column holding a row's value (and a script row's note), taking the row's free width.
fn value_column() -> Div {
    v_flex().flex_1().min_w_0()
}

/// One labelled cell of a row, `NAME_WIDTH` wide for a name or else as wide as its column,
/// in `color` when given; its text wraps at spaces.
fn cell(
    id: (&'static str, usize),
    text: String,
    name: bool,
    color: Option<Hsla>,
) -> impl IntoElement {
    // A long value must wrap rather than push the row's button out of the dialog.
    let base = div().id(id).min_w_0().whitespace_normal();
    let base = if name {
        base.flex_none().w(NAME_WIDTH)
    } else {
        base.w_full()
    };
    let base = match color {
        Some(color) => base.text_color(color),
        None => base,
    };
    base.test_support().aria_label(text.clone()).child(text)
}

/// The note a script-set row shows for the layer the script wrote.
pub fn overlay_note(layer: OverlayLayer) -> &'static str {
    match layer {
        OverlayLayer::Environment => "Environment, set by a script",
        OverlayLayer::Global => "Global, set by a script",
        _ => "Collection, set by a script",
    }
}

/// Opens the "Runtime variables" dialog over `window` for `vars` and the script-set
/// `overlay`, masking script rows named in `secrets` (the selected environment's secret
/// names), global rows also named in `global_secrets` (the active global environment's), and
/// rows holding a tainted value, and returns its body.
pub fn open_runtime_vars(
    vars: RuntimeVars,
    overlay: EnvOverlay,
    secrets: Vec<String>,
    global_secrets: Vec<String>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<RuntimeVarsView> {
    let view =
        cx.new(|cx| RuntimeVarsView::new(vars, overlay, secrets, global_secrets, window, cx));
    let body = view.clone();
    let scroll = ScrollHandle::new();
    window.open_dialog(cx, move |d, window, _| {
        // The capped box scrolls itself above the status bar.
        let room = crate::dialog_frame::titled_body_room(window);
        let view = body.clone();
        ui::with_close(crate::dialog_frame::place(d, window), "Runtime variables")
            // Enter saves the field it is pressed in; it must not close the dialog.
            .on_ok(|_, _, _| false)
            // Closing is the last blur: typing not yet saved is saved, as Preferences does.
            .on_close(move |_, window, cx| {
                view.update(cx, |v, cx| v.commit_all(window, cx));
            })
            .child(
                div()
                    .relative()
                    .child(
                        div()
                            .id("runtime-vars-scroll")
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
