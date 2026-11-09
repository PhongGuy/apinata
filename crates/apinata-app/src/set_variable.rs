//! The "Set as variable..." dialog of the response body's node menu: a name, a target and the
//! node's value shown read-only. Environment and collection targets are written through the
//! collection's own edit and save path; the runtime target only fills the in-memory store.

use crate::ui::LockButton as _;
use std::path::{Path, PathBuf};

use apinata_core::{EnvVariable, VarValue, Variable};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::{ActiveTheme as _, IndexPath, WindowExt, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Entity, Result, SharedString, Subscription, TestSupportExt, Window, div, rems,
};

use crate::command::{Command, Edit, Target};
use crate::style::{SHORT_FORM_LABEL_WIDTH, SPACE_2};
use crate::ui;
use crate::workspace::Workspace;

/// Where the variable goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarTarget {
    /// The named environment's file.
    Environment(String),
    /// The collection's own variables in `opencollection.yml`.
    Collection,
    /// The runtime store only; nothing is written to disk.
    Runtime,
}

impl VarTarget {
    /// The picker's label.
    pub fn label(&self) -> SharedString {
        match self {
            VarTarget::Environment(name) => format!("Environment: {name}").into(),
            VarTarget::Collection => "Collection variables".into(),
            VarTarget::Runtime => "Runtime only".into(),
        }
    }
}

/// The picker's choices: the active environment when there is one, the collection, then
/// runtime only. A secret is never a target; a secret variable of the environment is refused
/// by name in [`apply`].
pub fn targets(environment: Option<&str>) -> Vec<VarTarget> {
    (environment.map(|name| VarTarget::Environment(name.to_owned())))
        .into_iter()
        .chain([VarTarget::Collection, VarTarget::Runtime])
        .collect()
}

/// What the user chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The variable name, trimmed and never blank.
    pub name: String,
    /// The picked target.
    pub target: VarTarget,
}

/// The dialog's body: a name input, a target picker and the value.
pub struct SetVariableForm {
    targets: Vec<VarTarget>,
    name: Entity<InputState>,
    target: Entity<ui::IdSelect>,
    value: SharedString,
    _name_changes: Subscription,
}

impl SetVariableForm {
    /// A form for `value`, with `name` prefilled and the first of `targets` picked.
    pub fn new(
        targets: Vec<VarTarget>,
        name: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let labels = ui::id_items("set-variable-target", targets.iter().map(VarTarget::label));
        let name = cx.new(|cx| InputState::new(window, cx).default_value(name.to_owned()));
        let target = cx.new(|cx| SelectState::new(labels, Some(IndexPath::new(0)), window, cx));
        // The Set button follows the name, so each keystroke re-renders.
        let changes = cx.subscribe(&name, |_, _, _: &InputEvent, cx| cx.notify());
        SetVariableForm {
            targets,
            name,
            target,
            value: value.to_owned().into(),
            _name_changes: changes,
        }
    }

    /// The name input.
    pub fn name_input(&self) -> &Entity<InputState> {
        &self.name
    }

    /// The picked target.
    pub fn target(&self, cx: &App) -> VarTarget {
        (self.target.read(cx).selected_index(cx))
            .and_then(|ix| self.targets.get(ix.row).cloned())
            .unwrap_or(VarTarget::Runtime)
    }

    /// The trimmed name and picked target; `None` while the name is blank.
    pub fn answer(&self, cx: &App) -> Option<Answer> {
        let name = self.name.read(cx).value().trim().to_owned();
        (!name.is_empty()).then(|| Answer {
            name,
            target: self.target(cx),
        })
    }
}

impl Render for SetVariableForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let value = div()
            .id("set-variable-value")
            .test_support()
            .aria_label(self.value.clone())
            .w_full()
            .max_h(rems(8.))
            .overflow_y_scroll()
            .p(SPACE_2)
            .rounded(cx.theme().radius)
            .bg(ui::field_fill(true, cx))
            .text_color(cx.theme().muted_foreground)
            .whitespace_normal()
            .child(self.value.clone());
        v_flex()
            .id("set-variable-form")
            .gap(SPACE_2)
            .child(
                ui::form_row(
                    "set-variable-name-row",
                    "Name",
                    ui::padded_input(
                        Input::new(&self.name)
                            .id("set-variable-name")
                            .w_full()
                            .bg(ui::field_fill(false, cx)),
                        &self.name,
                    ),
                )
                .label_width(SHORT_FORM_LABEL_WIDTH),
            )
            .child(
                ui::form_row(
                    "set-variable-target-row",
                    "Target",
                    ui::dialog_select(Select::new(&self.target).id("set-variable-target").w_full())
                        .w_full(),
                )
                .label_width(SHORT_FORM_LABEL_WIDTH),
            )
            .child(
                ui::form_row("set-variable-value-row", "Value", value)
                    .label_width(SHORT_FORM_LABEL_WIDTH),
            )
    }
}

/// Sets variable `name` to `value` in `target` of the collection at `root`. A write replaces
/// a variable of the same name, edits the draft and saves it unless the file already had
/// unsaved edits (those stay a draft). Errs, changing nothing, on a bad name, a collection
/// that is not open or switching, and a name that is a secret of the environment.
pub fn apply(
    workspace: &Entity<Workspace>,
    root: &Path,
    name: &str,
    value: &str,
    target: &VarTarget,
    cx: &mut App,
) -> Result<(), String> {
    if apinata_core::variable_name_problem(name).is_some() {
        return Err("Use letters, digits, _, - and . in a variable name".into());
    }
    let snapshot = workspace
        .read(cx)
        .snapshot(root, cx)
        .ok_or("The collection is not open")?;
    let (doc, edit, dirty) = match target {
        VarTarget::Runtime => {
            let vars = workspace.read(cx).runtime_vars(root, cx);
            return vars
                .map(|v| v.set_text(name, value))
                .ok_or_else(|| "The collection is not open".into());
        }
        VarTarget::Environment(env) => {
            let view = (snapshot.content.environments.iter())
                .find(|e| e.name == *env)
                .ok_or("That environment is gone")?;
            let mut vars: Vec<EnvVariable> = view.variables.clone();
            match vars.iter_mut().find(|v| v.name == name) {
                Some(v) if v.secret => {
                    return Err(format!("{name} is a secret in {env}; pick another name"));
                }
                Some(v) => {
                    v.value = VarValue::Text(value.to_owned());
                    v.enabled = true;
                }
                None => vars.push(EnvVariable::new(name, VarValue::Text(value.to_owned()))),
            }
            (
                Target::Environment(env.clone()),
                Edit::Variables(vars),
                view.dirty,
            )
        }
        VarTarget::Collection => {
            let view = &snapshot.content.collection;
            let mut vars: Vec<Variable> = view.vars.clone();
            match vars.iter_mut().find(|v| v.name == name) {
                Some(v) => {
                    v.value = VarValue::Text(value.to_owned());
                    v.enabled = true;
                }
                None => vars.push(Variable::new(name, VarValue::Text(value.to_owned()))),
            }
            (Target::Collection, Edit::Vars(vars), view.dirty)
        }
    };
    let root: PathBuf = root.to_path_buf();
    workspace.update(cx, |w, cx| {
        let edit = Command::Edit {
            target: doc.clone(),
            edit,
        };
        w.send(&root, edit, cx).ok_or("The collection is busy")?;
        if !dirty {
            w.send(&root, Command::Save(doc), cx)
                .ok_or("The collection is busy")?;
        }
        Ok(())
    })
}

/// Opens the dialog over `window` for the node `value`, with `name` prefilled, and returns its
/// body. Enter and Set apply it; Escape and Cancel close it without a write.
pub fn open(
    workspace: &Entity<Workspace>,
    root: &Path,
    name: &str,
    value: &str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SetVariableForm> {
    open_value(workspace, root, name, value, false, window, cx)
}

/// [`open`] for a `value` that may be `sensitive`. A sensitive value is never shown in the
/// dialog, goes to the runtime store only (never a file) and marks its variable sensitive.
pub fn open_value(
    workspace: &Entity<Workspace>,
    root: &Path,
    name: &str,
    value: &str,
    sensitive: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SetVariableForm> {
    let environment = workspace.read(cx).selected_environment(root, cx);
    let choices = if sensitive {
        vec![VarTarget::Runtime]
    } else {
        targets(environment.as_deref())
    };
    let shown = if sensitive { "[redacted]" } else { value };
    let form = cx.new(|cx| SetVariableForm::new(choices, name, shown, window, cx));
    let (body, ws, root, value) = (
        form.clone(),
        workspace.clone(),
        root.to_path_buf(),
        value.to_owned(),
    );
    let input = form.read(cx).name.clone();
    window.defer(cx, move |window, cx| {
        let builder_form = body.clone();
        window.open_dialog(cx, move |dialog, window, cx| {
            let dialog = crate::dialog_frame::place(dialog, window);
            let ready = builder_form.read(cx).answer(cx).is_some();
            // Enter and the Set button run the same write; a refusal keeps the dialog open.
            let confirm = {
                let (form, ws, root, value) = (
                    builder_form.clone(),
                    ws.clone(),
                    root.clone(),
                    value.clone(),
                );
                move |window: &mut Window, cx: &mut App| {
                    let Some(answer) = form.read(cx).answer(cx) else {
                        return false;
                    };
                    match apply(&ws, &root, &answer.name, &value, &answer.target, cx) {
                        Ok(()) => {
                            if sensitive && let Some(vars) = ws.read(cx).runtime_vars(&root, cx) {
                                vars.mark_sensitive(&answer.name);
                            }
                            let message = format!("Set {}", answer.name);
                            window.push_notification(ui::info_toast(message), cx);
                            true
                        }
                        Err(why) => {
                            window.push_notification(ui::info_toast(why), cx);
                            false
                        }
                    }
                }
            };
            let on_ok = confirm.clone();
            let buttons = ui::dialog_footer(
                Some(
                    Button::new("set-variable-cancel")
                        .label("Cancel")
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
                [],
                Button::new("set-variable-set")
                    .label("Set")
                    .primary()
                    .lock_filled(!ready, cx)
                    .on_click(move |_, window, cx| {
                        if confirm(window, cx) {
                            window.close_dialog(cx);
                        }
                    }),
            );
            dialog
                .title("Set as variable")
                .close_button(false)
                .on_ok(move |_, window, cx| on_ok(window, cx))
                .child(
                    v_flex()
                        .id("set-variable-dialog")
                        .test_support()
                        .child(builder_form.clone())
                        .child(buttons),
                )
        });
        input.update(cx, |i, cx| {
            i.focus(window, cx);
            i.select_all(window, cx);
        });
    });
    form
}
