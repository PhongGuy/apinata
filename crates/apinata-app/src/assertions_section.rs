//! The Assertions section: a table of expression, operator and value rows.

use apinata_core::Assertion;
use gpui_kit::InteractiveElement as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    App, AppContext, Context, DragMoveEvent, Entity, EventEmitter, FocusHandle, Focusable as _,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, TestSupportExt as _, Window, div,
};
use std::cell::Cell;

use crate::command::Edit;
use crate::kv_grid::{
    self, ActionShow, KvColumn, Reloaded, action_slot, check_slot, kv_head, kv_row,
};
use crate::kv_table::set_text;
use crate::row_move::{
    self, DraggedRow, MoveRowDown, MoveRowUp, drop_line, drop_side, grip_slot, move_target,
    step_target,
};
use crate::scroll_follow::{FocusFollow, Followed};
use crate::section::SectionEdit;
use crate::style::{OPERATOR_MENU_WIDTH, OPERATOR_SELECT_WIDTH, ROW_HEIGHT, VALUE_GROW};
use crate::tabs::Side;
use crate::ui;

/// Bruno's operators in its picker order (`RequestPane/Assertions/AssertionOperator`).
pub const OPERATORS: [&str; 28] = [
    "eq",
    "neq",
    "gt",
    "gte",
    "lt",
    "lte",
    "in",
    "notIn",
    "contains",
    "notContains",
    "length",
    "matches",
    "notMatches",
    "startsWith",
    "endsWith",
    "between",
    "isEmpty",
    "isNotEmpty",
    "isNull",
    "isUndefined",
    "isDefined",
    "isTruthy",
    "isFalsy",
    "isJson",
    "isNumber",
    "isString",
    "isBoolean",
    "isArray",
];

/// The plain words the picker shows for each of `OPERATORS`, in the same order.
pub const OPERATOR_LABELS: [&str; 28] = [
    "equals",
    "not equals",
    "greater than",
    "greater than or equal",
    "less than",
    "less than or equal",
    "in",
    "not in",
    "contains",
    "does not contain",
    "length is",
    "matches",
    "does not match",
    "starts with",
    "ends with",
    "between",
    "is empty",
    "is not empty",
    "is null",
    "is undefined",
    "is defined",
    "is truthy",
    "is falsy",
    "is JSON",
    "is a number",
    "is a string",
    "is a boolean",
    "is an array",
];

/// The plain words for `operator`; an operator outside `OPERATORS` reads as itself.
pub fn operator_label(operator: &str) -> &str {
    OPERATORS
        .iter()
        .position(|op| *op == operator)
        .map_or(operator, |ix| OPERATOR_LABELS[ix])
}

/// One picker row: shows the plain words with the raw operator muted beside them, and is
/// found by either. Its value is the raw operator, which is what the file stores.
#[derive(Debug, Clone)]
pub struct OperatorItem {
    id: SharedString,
    op: SharedString,
    label: SharedString,
}

impl OperatorItem {
    fn new(op: &str) -> Self {
        OperatorItem {
            id: format!("as-op-option-{op}").into(),
            op: op.to_owned().into(),
            label: operator_label(op).to_owned().into(),
        }
    }
}

impl SelectItem for OperatorItem {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &SharedString {
        &self.op
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        [&self.label, &self.op]
            .iter()
            .any(|text| text.to_lowercase().contains(&query))
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .id(self.id.clone())
            .test_support()
            .aria_label(self.label.clone())
            .aria_description(self.op.clone())
            .w_full()
            .gap_2()
            .justify_between()
            .child(self.label.clone())
            .when(self.op != self.label, |row| {
                row.child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.op.clone()),
                )
            })
    }
}

type OperatorSelect = SelectState<SearchableVec<OperatorItem>>;

/// Whether `operator` takes no value: the last twelve of `OPERATORS`, as Bruno's
/// `isUnaryOperator` lists them.
pub fn is_unary(operator: &str) -> bool {
    OPERATORS[16..].contains(&operator)
}

/// The column headings, in column order.
pub const HEADINGS: [&str; 3] = ["Expression", "Operator", "Value"];

/// One row as the table shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AssertionRow {
    /// The left side.
    pub expression: String,
    /// The operator.
    pub operator: String,
    /// The right side; ignored for a unary operator.
    pub value: String,
    /// Whether the row is on.
    pub enabled: bool,
    /// The index of the core entry this row came from; `None` for a new row.
    pub source: Option<usize>,
}

/// Rows for `assertions`; a missing value shows as empty text.
pub fn assertion_rows(assertions: &[Assertion]) -> Vec<AssertionRow> {
    assertions
        .iter()
        .enumerate()
        .map(|(ix, a)| AssertionRow {
            expression: a.expression().to_owned(),
            operator: a.operator().to_owned(),
            value: a.value().unwrap_or_default().to_owned(),
            enabled: a.enabled(),
            source: Some(ix),
        })
        .collect()
}

/// Assertions from `rows`. A row with a source edits a clone of that entry and touches only
/// what changed, so its description and unknown keys survive, as Bruno's `updateAssertion`.
pub fn assertions_from(base: &[Assertion], rows: &[AssertionRow]) -> Vec<Assertion> {
    rows.iter()
        .map(|row| {
            let value = (!is_unary(&row.operator)).then(|| row.value.clone());
            let mut a = match row.source.and_then(|ix| base.get(ix)).cloned() {
                Some(mut a) => {
                    if a.expression() != row.expression {
                        a.set_expression(row.expression.clone());
                    }
                    if a.operator() != row.operator
                        || value
                            .as_deref()
                            .is_some_and(|v| a.value().unwrap_or_default() != v)
                    {
                        a.set_operator(row.operator.clone(), value);
                    }
                    a
                }
                None => Assertion::new(row.expression.clone(), row.operator.clone(), value),
            };
            a.set_enabled(row.enabled);
            a
        })
        .collect()
}

/// A row control that keeps focus through a reload that keeps its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Check,
    Operator,
    Remove,
}

struct Row {
    expression: Entity<InputState>,
    value: Entity<InputState>,
    operator: Entity<OperatorSelect>,
    op: String,
    enabled: bool,
    source: Option<usize>,
    // Not a tab stop; it only sees focus inside the row's checkbox.
    check: FocusHandle,
    // Not a tab stop; it only sees focus inside the row's Remove.
    remove: FocusHandle,
    // Tracks the whole row, so focus on any control in it (cell, operator, checkbox,
    // delete) counts as focus in the row.
    within: FocusHandle,
    _subs: [Subscription; 3],
}

impl Row {
    fn inputs(&self) -> [&Entity<InputState>; 2] {
        [&self.expression, &self.value]
    }
}

/// Edits a request's assertions, one row per entry, each with an operator picker, always
/// ending with an empty row that turns into a real one when typed in.
pub struct AssertionsSection {
    rows: Vec<Row>,
    blank: Row,
    base: Vec<Assertion>,
    /// Where each row reports its bounds to the scroll area holding the section.
    follow: FocusFollow,
    /// The `(row, column)` of the cell the last edit was typed in; cleared by any other edit.
    typed: Option<(usize, u8)>,
    /// Where a dragged row would land: the row under the pointer and the side of it.
    drop: Option<(usize, Side)>,
    /// The row whose checkbox or Remove takes focus on the next draw, after a reload moved
    /// its row.
    control_focus: Cell<Option<(usize, Control)>>,
    /// The control that held focus when the pending control focus was left; the pending
    /// focus is the user's only while it still does.
    pending_origin: Option<FocusHandle>,
}

impl EventEmitter<SectionEdit> for AssertionsSection {}

impl AssertionsSection {
    /// An empty section.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        AssertionsSection {
            rows: Vec::new(),
            blank: Self::new_row(&fresh(), window, cx),
            base: Vec::new(),
            follow: FocusFollow::default(),
            typed: None,
            drop: None,
            control_focus: Cell::new(None),
            pending_origin: None,
        }
    }

    /// The cell the latest emitted edit was typed in, or `None` when another kind of edit
    /// (add, delete, toggle, operator) made it.
    pub fn last_typed(&self) -> Option<(usize, u8)> {
        self.typed
    }

    /// Reports each row's bounds to `follow`, the watch of the scroll area holding the section.
    pub fn set_follow(&mut self, follow: FocusFollow) {
        self.follow = follow;
    }

    // A pending focus waits for a draw the section may not get while hidden; once focus has
    // left the control it came from, the user moved on and the pending focus is dropped.
    fn drop_moved_on_focus(&mut self, window: &Window) {
        if !(self.pending_origin.as_ref()).is_some_and(|h| h.is_focused(window)) {
            self.control_focus.set(None);
            self.pending_origin = None;
        }
    }

    /// Shows `assertions` from a snapshot without emitting an edit. Rows are reused by index
    /// and only differing text is set, so an echo of the user's typing keeps the caret.
    pub fn load(&mut self, assertions: &[Assertion], window: &mut Window, cx: &mut Context<Self>) {
        self.base = assertions.to_vec();
        let rows = assertion_rows(assertions);
        // A control focus an earlier load left for a draw that has not come yet is where focus
        // belongs; the window's focus still names the old row's control.
        self.drop_moved_on_focus(window);
        let pending = self.control_focus.take();
        // An undo or redo of a move shows the rows reordered; focus goes with the row it was on.
        let followed = self.focused_cell(window, cx).and_then(|(ix, col)| {
            let to = row_move::follow(&self.rows(cx), &rows, ix, |_| false, same_content)?;
            Some((to, col))
        });
        // Focus on a row's control (its checkbox, operator or Remove) stays with that row if
        // it survives the load, and moves as a delete moves it if not.
        let held = (pending.map(|(ix, _)| ix)).or_else(|| self.focused_row(window, cx));
        let in_cell = pending.is_none() && self.focused_cell(window, cx).is_some();
        let kind = (pending.map(|(_, kind)| kind))
            .or_else(|| held.map(|ix| self.focused_control(ix, window, cx)));
        let control = held
            .filter(|_| !in_cell)
            .map(|ix| kv_grid::after_reload(&self.rows(cx), &rows, ix, |_| false, same_content));
        self.rows.truncate(rows.len());
        for (ix, row) in rows.into_iter().enumerate() {
            if ix == self.rows.len() {
                let new = Self::new_row(&row, window, cx);
                self.rows.push(new);
                continue;
            }
            let existing = &mut self.rows[ix];
            set_text(&existing.expression, &row.expression, window, cx);
            set_text(&existing.value, &row.value, window, cx);
            if existing.op != row.operator {
                show_operator(&existing.operator, &row.operator, window, cx);
            }
            existing.op = row.operator;
            existing.enabled = row.enabled;
            existing.source = row.source;
        }
        match (followed, control) {
            (Some((to, col)), _) => {
                if let Some(row) = self.rows.get(to) {
                    row.inputs()[col].focus_handle(cx).focus(window, cx);
                }
            }
            (None, Some(Reloaded::Kept(to))) if Some(to) != held || pending.is_some() => match kind
            {
                Some(Control::Operator) | None => {
                    self.rows[to].operator.focus_handle(cx).focus(window, cx)
                }
                Some(kind) => {
                    self.control_focus.set(Some((to, kind)));
                    self.pending_origin = window.focused(cx);
                }
            },
            (None, Some(Reloaded::Dropped(ix))) => {
                let target = kv_grid::after_delete(&self.rows, ix, &self.blank);
                target.expression.clone().focus_handle(cx).focus(window, cx);
            }
            _ => {}
        }
        cx.notify();
    }

    /// The rows as they stand, typed text included.
    pub fn rows(&self, cx: &App) -> Vec<AssertionRow> {
        self.rows
            .iter()
            .map(|row| AssertionRow {
                expression: row.expression.read(cx).value().to_string(),
                operator: row.op.clone(),
                value: row.value.read(cx).value().to_string(),
                enabled: row.enabled,
                source: row.source,
            })
            .collect()
    }

    /// Whether `assertions`, an edit this section sent, was built from entries a load has
    /// since replaced: the section would now build something else from its rows, shown or
    /// in the YAML an entry was read from.
    pub fn lags(&self, assertions: &[Assertion], cx: &App) -> bool {
        let now = assertions_from(&self.base, &self.rows(cx));
        now.len() != assertions.len() || !now.iter().zip(assertions).all(|(a, b)| a.same_source(b))
    }

    /// Sets row `ix`'s operator and sends the edit. At `rows.len()` it makes the empty row a
    /// real one with that operator; an index past that does nothing.
    pub fn set_operator(
        &mut self,
        ix: usize,
        operator: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ix == self.rows.len() {
            self.materialise(window, cx);
            let row = &mut self.rows[ix];
            show_operator(&row.operator, operator, window, cx);
            row.op = operator.to_owned();
            self.typed = None;
            self.changed(cx);
            return;
        }
        let Some(row) = self.rows.get_mut(ix) else {
            return;
        };
        if row.op != operator {
            show_operator(&row.operator, operator, window, cx);
            row.op = operator.to_owned();
            self.typed = None;
            self.changed(cx);
        }
    }

    /// Appends `assertion` as an enabled row, focuses its expression and sends the edit. A row
    /// that already states the same expression, operator and value is focused instead, and
    /// nothing is added.
    pub fn add_assertion(
        &mut self,
        assertion: &Assertion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = AssertionRow {
            expression: assertion.expression().to_owned(),
            operator: assertion.operator().to_owned(),
            value: assertion.value().unwrap_or_default().to_owned(),
            enabled: true,
            source: None,
        };
        let same = |have: &AssertionRow| {
            have.expression == row.expression
                && have.operator == row.operator
                && (is_unary(&row.operator) || have.value == row.value)
        };
        let ix = match self.rows(cx).iter().position(same) {
            Some(ix) => ix,
            None => {
                self.rows.push(Self::new_row(&row, window, cx));
                self.typed = None;
                self.changed(cx);
                self.rows.len() - 1
            }
        };
        self.rows[ix].expression.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// The expression input of row `ix`, or of the empty row at `rows.len()`.
    pub fn expression_input(&self, ix: usize) -> Option<&Entity<InputState>> {
        self.rows
            .get(ix)
            .or((ix == self.rows.len()).then_some(&self.blank))
            .map(|row| &row.expression)
    }

    // Each handler finds its row by entity rather than by index, which a delete could stale.
    // Input from the empty row first makes it a real row, keeping its entities and focus.
    fn new_row(row: &AssertionRow, window: &mut Window, cx: &mut Context<Self>) -> Row {
        let mut input = |text: &str, hint: &'static str, cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(hint)
                    .default_value(text.to_owned())
            })
        };
        let expression = input(&row.expression, "res.status", cx);
        let value = input(&row.value, "Value", cx);
        let operator = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
        });
        show_operator(&operator, &row.operator, window, cx);
        let [expression_sub, value_sub] = [&expression, &value].map(|input| {
            cx.subscribe_in(
                input,
                window,
                |this: &mut Self, input, event, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        if this.blank.inputs().contains(&input) {
                            this.materialise(window, cx);
                        }
                        this.typed = this.cell_of(input);
                        this.changed(cx);
                    }
                },
            )
        });
        let subs = [
            expression_sub,
            value_sub,
            cx.subscribe_in(
                &operator,
                window,
                |this, select, event: &SelectEvent<SearchableVec<OperatorItem>>, window, cx| {
                    if let SelectEvent::Confirm(Some(op)) = event {
                        let ix = this
                            .rows
                            .iter()
                            .chain([&this.blank])
                            .position(|r| &r.operator == select);
                        if let Some(ix) = ix {
                            this.set_operator(ix, op, window, cx);
                        }
                    }
                },
            ),
        ];
        Row {
            expression,
            value,
            operator,
            op: row.operator.clone(),
            enabled: row.enabled,
            source: row.source,
            check: cx.focus_handle(),
            remove: cx.focus_handle(),
            within: cx.focus_handle(),
            _subs: subs,
        }
    }

    /// The row and column of `input` among the real rows.
    fn cell_of(&self, input: &Entity<InputState>) -> Option<(usize, u8)> {
        self.rows.iter().enumerate().find_map(|(ix, row)| {
            let col = row.inputs().iter().position(|i| *i == input)?;
            Some((ix, col as u8))
        })
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        let assertions = assertions_from(&self.base, &self.rows(cx));
        cx.emit(SectionEdit {
            edit: Edit::Assertions(assertions),
            typed: self.typed,
            lagging: false,
            loads: 0,
        });
        cx.notify();
    }

    /// Make the empty row the last real row and start a fresh empty one below it.
    fn materialise(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fresh = Self::new_row(&fresh(), window, cx);
        let row = std::mem::replace(&mut self.blank, fresh);
        self.rows.push(row);
    }

    /// Remove row `ix` and move focus to the next row's expression, else the previous row's,
    /// else the empty row's, so the keys of the editor around the table keep working.
    fn delete(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix < self.rows.len() {
            self.rows.remove(ix);
            let target = kv_grid::after_delete(&self.rows, ix, &self.blank)
                .expression
                .clone();
            target.focus_handle(cx).focus(window, cx);
            self.typed = None;
            self.changed(cx);
        }
    }

    /// The row and input column holding focus, when it is in a real row's inputs.
    fn focused_cell(&self, window: &Window, cx: &App) -> Option<(usize, usize)> {
        self.rows.iter().enumerate().find_map(|(ix, row)| {
            let col = row
                .inputs()
                .iter()
                .position(|i| i.focus_handle(cx).contains_focused(window, cx))?;
            Some((ix, col))
        })
    }

    /// The real row holding focus anywhere in it: a cell, the operator, its checkbox or delete.
    fn focused_row(&self, window: &Window, cx: &App) -> Option<usize> {
        (self.rows.iter()).position(|row| row.within.contains_focused(window, cx))
    }

    /// Which control of row `ix` holds focus, when it is not a cell.
    fn focused_control(&self, ix: usize, window: &Window, cx: &App) -> Control {
        let row = &self.rows[ix];
        if row.check.contains_focused(window, cx) {
            Control::Check
        } else if row.remove.contains_focused(window, cx) {
            Control::Remove
        } else {
            Control::Operator
        }
    }

    /// Move row `from` to index `dest` as one edit, keeping its inputs, operator and flags.
    /// Focus goes to the moved row's expression unless it already sits in that row.
    pub fn move_row(
        &mut self,
        from: usize,
        dest: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let len = self.rows.len();
        if from >= len || dest >= len || from == dest {
            return;
        }
        // Focus on the row element itself (a grip press) is not focus on a control in it.
        let inside = self.focused_row(window, cx) == Some(from)
            && !self.rows[from].within.is_focused(window);
        row_move::reorder(&mut self.rows, from, dest);
        if !inside {
            self.rows[dest]
                .expression
                .focus_handle(cx)
                .focus(window, cx);
        }
        self.drop = None;
        self.typed = None;
        self.changed(cx);
    }

    /// Mark where dragging row `from` over row `to` would land, or clear the mark with `None`.
    /// A side where the drop would leave the row in place shows nothing.
    pub fn hover_drop(
        &mut self,
        from: usize,
        to: usize,
        side: Option<Side>,
        cx: &mut Context<Self>,
    ) {
        let next = side
            .filter(|&s| move_target(self.rows.len(), from, to, s).is_some())
            .map(|s| (to, s));
        let next = match (next, self.drop) {
            (None, Some((at, _))) if at != to => self.drop,
            _ => next,
        };
        if next != self.drop {
            self.drop = next;
            cx.notify();
        }
    }

    /// Alt+Up or Alt+Down: swap the row holding focus with its neighbour.
    fn step(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.focused_row(window, cx) else {
            return;
        };
        if let Some(dest) = step_target(self.rows.len(), ix, up) {
            self.move_row(ix, dest, window, cx);
        }
    }

    /// A row dropped on the empty row moves nowhere; focus stays with the dragged row.
    fn drop_on_empty(&mut self, from: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.drop = None;
        if let Some(row) = self.rows.get(from) {
            row.expression.focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    /// A row dropped on the `side` of row `to` lands there when that moves it.
    fn drop_row(&mut self, from: usize, to: usize, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.drop.filter(|&(at, _)| at == to).map(|(_, s)| s);
        self.drop = None;
        cx.notify();
        if let Some(dest) = side.and_then(|s| move_target(self.rows.len(), from, to, s)) {
            self.move_row(from, dest, window, cx);
        }
    }

    fn toggle(&mut self, ix: usize, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        // A click lands focus in the row unless the checkbox already holds it, so Alt+Up and
        // Alt+Down move the row.
        if let Some(row) = self.rows.get(ix)
            && !row.check.contains_focused(window, cx)
        {
            row.check.focus(window, cx);
        }
        if let Some(row) = self.rows.get_mut(ix) {
            row.enabled = enabled;
            self.typed = None;
            self.changed(cx);
        }
    }
}

// Whether two rows show the same content, wherever they came from.
fn same_content(a: &AssertionRow, b: &AssertionRow) -> bool {
    (&a.expression, &a.operator, &a.value, a.enabled)
        == (&b.expression, &b.operator, &b.value, b.enabled)
}

// A new row is `eq` with an empty value, as Bruno's `addAssertion`.
fn fresh() -> AssertionRow {
    AssertionRow {
        operator: "eq".into(),
        enabled: true,
        ..AssertionRow::default()
    }
}

// Offers Bruno's operators plus `operator` itself when it is outside them, then selects it.
fn show_operator(
    select: &Entity<OperatorSelect>,
    operator: &str,
    window: &mut Window,
    cx: &mut App,
) {
    let mut items: Vec<OperatorItem> = OPERATORS.iter().map(|op| OperatorItem::new(op)).collect();
    if !OPERATORS.contains(&operator) {
        items.push(OperatorItem::new(operator));
    }
    let selected: SharedString = operator.to_owned().into();
    select.update(cx, |s, cx| {
        s.set_items(SearchableVec::new(items), window, cx);
        s.set_selected_value(&selected, window, cx);
    });
}

impl AssertionsSection {
    /// Whether row `ix`'s `control` takes the focus a reload left for it, clearing that once
    /// it does.
    fn take_control_focus(&self, ix: usize, control: Control) -> bool {
        let take = self.control_focus.get() == Some((ix, control));
        if take {
            self.control_focus.set(None);
        }
        take
    }

    /// Draw row `ix`. The empty row has nothing to toggle or delete yet, so its checkbox and
    /// delete are laid out hidden, under their own ids, to keep its columns in line.
    fn render_row(
        &self,
        ix: usize,
        row: &Row,
        blank: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        // `columns()` is a fixed three-column layout.
        #[allow(clippy::expect_used)]
        let [expr_col, op_col, value_col] = columns()
            .try_into()
            .expect("the assertion table has three columns");
        let focused = |input: &Entity<InputState>, cx: &App| {
            input.focus_handle(cx).contains_focused(window, cx)
        };
        // Each cell's edge lights while the pointer is over it and turns to the ring on focus.
        let cell = |input: &Entity<InputState>, id: &'static str, edge: &'static str, cx: &App| {
            let field = ui::table_input(input, false, window, cx).id((id, ix));
            kv_grid::kv_cell((edge, ix), field, input, focused(input, cx), cx)
        };
        let remove_id = (if blank { "as-slot-delete" } else { "as-delete" }, ix);
        // The remove control shows on the hovered row and stays while the row is typed in.
        let show = if blank {
            ActionShow::Never
        } else if row.within.contains_focused(window, cx) {
            ActionShow::Always
        } else {
            ActionShow::OnHover
        };
        let handle = (!blank).then(|| {
            DraggedRow::new(
                cx.entity_id(),
                ix,
                row_move::preview_label(&row.expression.read(cx).value()).into(),
            )
        });
        let landing = self
            .drop
            .filter(|&(at, _)| at == ix && !blank)
            .map(|(_, s)| s);
        kv_row(("as-row", ix))
            .track_focus(&row.within)
            .h(ROW_HEIGHT)
            .relative()
            .when(!blank, |r| {
                r.on_drag_move(cx.listener(move |t, e: &DragMoveEvent<DraggedRow>, _, cx| {
                    let d = e.drag(cx).clone();
                    if d.owner != cx.entity_id() {
                        return;
                    }
                    let side = e
                        .bounds
                        .contains(&e.event.position)
                        .then(|| drop_side(e.event.position.y, e.bounds));
                    t.hover_drop(d.from, ix, side, cx);
                }))
                .on_drop(cx.listener(move |t, d: &DraggedRow, window, cx| {
                    if d.owner == cx.entity_id() {
                        t.drop_row(d.from, ix, window, cx);
                    }
                }))
            })
            .when(blank, |r| {
                r.on_drop(cx.listener(move |t, d: &DraggedRow, window, cx| {
                    if d.owner == cx.entity_id() {
                        t.drop_on_empty(d.from, window, cx);
                    }
                }))
            })
            .when_some(landing, |r, side| {
                r.child(drop_line("as-drop-line", side, cx))
            })
            .child(grip_slot(("as-grip", ix), ("as-slot-grip", ix), handle, cx))
            .child(check_slot(
                div()
                    .track_focus(&row.check)
                    .when(blank, |d| d.invisible())
                    .child(
                        ui::checkbox(
                            (
                                if blank {
                                    "as-slot-enabled"
                                } else {
                                    "as-enabled"
                                },
                                ix,
                            ),
                            row.enabled,
                        )
                        .tab_stop(!blank)
                        .take_focus(!blank && self.take_control_focus(ix, Control::Check))
                        .on_click(cx.listener(
                            move |t, on: &bool, window, cx| t.toggle(ix, *on, window, cx),
                        )),
                    ),
            ))
            .child(expr_col.size(cell(&row.expression, "as-expr", "as-expr-edge", cx)))
            .child(
                op_col.size(
                    div().child(
                        Select::new(&row.operator)
                            .id(("as-op", ix))
                            .menu_width(OPERATOR_MENU_WIDTH)
                            .search_placeholder("Search operators")
                            .small(),
                    ),
                ),
            )
            .map(|r| {
                if is_unary(&row.op) {
                    r.child(value_col.size(div()))
                } else {
                    r.child(value_col.size(cell(&row.value, "as-value", "as-value-edge", cx)))
                }
            })
            .child(action_slot(
                div()
                    .track_focus(&row.remove)
                    .child(kv_grid::button_focusing(
                        remove_id,
                        ui::icon_button(remove_id, IconName::Close, "Remove assertion", None)
                            .tab_stop(!blank)
                            .on_click(
                                cx.listener(move |t, _, window, cx| t.delete(ix, window, cx)),
                            ),
                        !blank && self.take_control_focus(ix, Control::Remove),
                    )),
                show,
            ))
    }
}

/// The assertion table's columns: Expression (`as-head-expr`, `as-expr`, flex), Operator
/// (`as-head-op`, `as-op`, `OPERATOR_SELECT_WIDTH`), Value (`as-head-value`, `as-value`,
/// `VALUE_GROW` parts of the free width to the expression's one); headings are `HEADINGS`.
pub fn columns() -> Vec<KvColumn> {
    let [expr, op, value] = HEADINGS;
    vec![
        KvColumn::flex("as-head-expr", "as-expr", expr),
        KvColumn::fixed("as-head-op", "as-op", op, OPERATOR_SELECT_WIDTH),
        KvColumn::grow("as-head-value", "as-value", value, VALUE_GROW),
    ]
}

impl Render for AssertionsSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.drop_moved_on_focus(window);
        let mut rows: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .map(|(ix, row)| {
                let row = self.render_row(ix, row, false, window, cx);
                Followed::new(("as-row-wrapper", ix), &self.follow, row).into_any_element()
            })
            .collect();
        let blank = self.render_row(self.rows.len(), &self.blank, true, window, cx);
        rows.push(
            Followed::new(("as-row-wrapper", self.rows.len()), &self.follow, blank)
                .into_any_element(),
        );
        // The heading leads with an empty check slot and ends with an empty action slot, so
        // each heading starts where its column does; the Value heading stays for unary rows.
        v_flex()
            .gap_1()
            .w_full()
            .key_context(kv_grid::STRUCTURED_ROWS_CONTEXT)
            .on_action(cx.listener(|t, _: &MoveRowUp, window, cx| t.step(true, window, cx)))
            .on_action(cx.listener(|t, _: &MoveRowDown, window, cx| t.step(false, window, cx)))
            .child(
                kv_head("as-head", columns())
                    .grip_column()
                    .check_column()
                    .shared_band()
                    .action(None),
            )
            .children(rows)
    }
}
