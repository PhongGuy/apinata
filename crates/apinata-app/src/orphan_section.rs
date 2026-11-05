//! The section both environment editors show under the variable table: stored secrets the
//! environment's file no longer marks secret, then those of environments with no file at all,
//! each with a Remove behind a confirmation. Only names appear here, never a value.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use apinata_core::{EnvVariable, OrphanReport, SecretError, SecretKey, SecretStore};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

use crate::scroll_follow::{FocusFollow, Followed};
use crate::ui;

/// The confirm step's detail, shared by both editors.
pub const REMOVE_DETAIL: &str =
    "The stored value is deleted. This cannot be undone, and another git branch may still use it.";

/// What the section has to say.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Orphans {
    /// Nothing listed yet for the selected environment.
    #[default]
    Unknown,
    /// The orphans as last listed.
    Names(Vec<String>),
    /// Why the list failed, which never reads as "no orphans".
    Failed(String),
}

impl Orphans {
    /// This state without any name the unsaved `variables` mark secret.
    pub fn without_drafts(&self, variables: &[EnvVariable]) -> Orphans {
        match self {
            Orphans::Names(names) => Orphans::Names(visible(names, variables)),
            other => other.clone(),
        }
    }
}

/// `names` without any the unsaved `variables` mark secret: a save may be about to declare them.
pub fn visible(names: &[String], variables: &[EnvVariable]) -> Vec<String> {
    let drafted = |name: &String| variables.iter().any(|v| v.secret && v.name == *name);
    names.iter().filter(|n| !drafted(n)).cloned().collect()
}

/// Asks to remove the stored value of orphan `name` of `env`; on "Delete", `remove` gets both.
pub fn ask_remove<E: 'static>(
    env: String,
    name: &str,
    window: &mut Window,
    cx: &mut Context<E>,
    remove: impl FnOnce(&mut E, String, String, &mut Context<E>) + 'static,
) {
    let title = format!("Remove stored secret \"{name}\"?");
    let ask = crate::confirm::delete_ask(title, Some(REMOVE_DETAIL));
    let answer = crate::confirm::ask(ask, window, cx);
    let name = name.to_owned();
    cx.spawn(async move |this, cx| {
        if answer.recv().await == Ok(1) {
            this.update(cx, |e, cx| remove(e, env, name, cx)).ok();
        }
    })
    .detach();
}

/// The section under the variable table, or nothing when there is nothing to say; each Remove
/// reports to `follow`, so Tab onto it scrolls it into view.
/// Ids: `env-orphans`, `("env-orphan-row", ix)`, `("env-orphan-remove", ix)`, `env-orphans-failed`.
pub fn section(
    state: &Orphans,
    follow: &FocusFollow,
    on_remove: impl Fn(&str, &mut Window, &mut App) + Clone + 'static,
    cx: &App,
) -> Option<AnyElement> {
    let muted = cx.theme().muted_foreground;
    match state {
        Orphans::Unknown => None,
        Orphans::Names(names) if names.is_empty() => None,
        Orphans::Failed(message) => {
            let message = ui::plain_quotes(message);
            let line =
                format!("Could not check for stored secrets no longer in this file: {message}.");
            let failed = div()
                .id("env-orphans-failed")
                .test_support()
                .aria_label(line.clone())
                .w_full()
                .min_w_0()
                .text_color(muted)
                .child(line);
            Some(failed.into_any_element())
        }
        Orphans::Names(names) => {
            let rows = names.iter().enumerate().map(|(ix, name)| {
                let on_remove = on_remove.clone();
                let clicked = name.clone();
                h_flex()
                    .id(("env-orphan-row", ix))
                    .test_support()
                    .w_full()
                    .gap_2()
                    // The button leads, so a pane wider than the window cannot clip it.
                    .child(Followed::new(
                        ("env-orphan-remove-follow", ix),
                        follow,
                        Button::new(("env-orphan-remove", ix))
                            .ghost()
                            .compact()
                            .label("Remove")
                            .accessibility_label(format!("Remove stored secret {name}"))
                            .on_click(move |_, window, cx| on_remove(&clicked, window, cx)),
                    ))
                    .child(div().flex_1().min_w_0().truncate().child(name.clone()))
            });
            let section = v_flex()
                .id("env-orphans")
                .test_support()
                .w_full()
                .gap_1()
                .child(
                    div()
                        .text_color(muted)
                        .child("These stored secrets are no longer in this environment's file."),
                )
                .children(rows);
            Some(section.into_any_element())
        }
    }
}

/// The confirm step's detail for a secret whose environment has no file. A file on a drive that
/// is not mounted reads exactly like a deleted one, so these words are the only guard.
pub const FILELESS_DETAIL: &str = "No file for this environment was found here. It may only be elsewhere, for example on a drive that is not mounted. Removing deletes the stored value for good.";

/// The stored secrets of one environment that has no file.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FilelessGroup {
    /// The environment name as stored.
    pub environment: String,
    /// Its stored variable names, sorted.
    pub variables: Vec<String>,
}

impl FilelessGroup {
    /// A group of `variables` under `environment`.
    pub fn new(environment: impl Into<String>, variables: Vec<String>) -> FilelessGroup {
        FilelessGroup {
            environment: environment.into(),
            variables,
        }
    }
}

/// What the fileless part of the section has to say.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Fileless {
    /// Nothing listed yet.
    #[default]
    Unknown,
    /// The groups as last listed, sorted by environment.
    Groups(Vec<FilelessGroup>),
    /// Why the list failed, which never reads as "none".
    Failed(String),
}

/// The fileless entries of `report`, one group per environment in the report's order.
pub fn fileless_groups(report: &OrphanReport) -> Vec<FilelessGroup> {
    let mut groups: Vec<FilelessGroup> = Vec::new();
    for orphan in report.orphans.iter().filter(|o| !o.has_file) {
        match groups.last_mut() {
            Some(g) if g.environment == orphan.environment => {
                g.variables.push(orphan.variable.clone())
            }
            _ => groups.push(FilelessGroup::new(
                orphan.environment.clone(),
                vec![orphan.variable.clone()],
            )),
        }
    }
    groups
}

/// Asks to remove the stored value of `name` in fileless `env`, carrying the warning that the
/// file may only be elsewhere; on "Delete", `remove` gets both.
pub fn ask_remove_fileless<E: 'static>(
    env: &str,
    name: &str,
    window: &mut Window,
    cx: &mut Context<E>,
    remove: impl FnOnce(&mut E, String, String, &mut Context<E>) + 'static,
) {
    let title = format!("Remove stored secret \"{name}\" of environment \"{env}\"?");
    let ask = crate::confirm::delete_ask(title, Some(FILELESS_DETAIL));
    let answer = crate::confirm::ask(ask, window, cx);
    let (env, name) = (env.to_owned(), name.to_owned());
    cx.spawn(async move |this, cx| {
        if answer.recv().await == Ok(1) {
            this.update(cx, |e, cx| remove(e, env, name, cx)).ok();
        }
    })
    .detach();
}

/// The fileless part, or nothing when there is nothing to say; `on_remove` gets the environment
/// and variable. Ids: `env-fileless`, `("env-fileless-env", gx)` labelled with the environment,
/// `("env-fileless-row", ix)` and `("env-fileless-remove", ix)` numbered across groups,
/// `env-fileless-failed`.
pub fn fileless_section(
    state: &Fileless,
    follow: &FocusFollow,
    on_remove: impl Fn(&str, &str, &mut Window, &mut App) + Clone + 'static,
    cx: &App,
) -> Option<AnyElement> {
    let muted = cx.theme().muted_foreground;
    let groups = match state {
        Fileless::Unknown => return None,
        Fileless::Groups(groups) if groups.is_empty() => return None,
        Fileless::Groups(groups) => groups,
        Fileless::Failed(message) => {
            let message = ui::plain_quotes(message);
            let line = format!(
                "Could not check for stored secrets of environments with no file: {message}."
            );
            let failed = div()
                .id("env-fileless-failed")
                .test_support()
                .aria_label(line.clone())
                .w_full()
                .min_w_0()
                .text_color(muted)
                .child(line);
            return Some(failed.into_any_element());
        }
    };
    let mut ix = 0usize;
    let groups = groups.iter().enumerate().map(|(gx, group)| {
        let rows = group.variables.iter().map(|name| {
            let on_remove = on_remove.clone();
            let (env, var) = (group.environment.clone(), name.clone());
            ix += 1;
            h_flex()
                .id(("env-fileless-row", ix - 1))
                .test_support()
                .w_full()
                .gap_2()
                .child(Followed::new(
                    ("env-fileless-remove-follow", ix - 1),
                    follow,
                    Button::new(("env-fileless-remove", ix - 1))
                        .ghost()
                        .compact()
                        .label("Remove")
                        .accessibility_label(format!(
                            "Remove stored secret {name} of {}",
                            group.environment
                        ))
                        .on_click(move |_, window, cx| on_remove(&env, &var, window, cx)),
                ))
                .child(div().flex_1().min_w_0().truncate().child(name.clone()))
        });
        let rows: Vec<_> = rows.collect();
        v_flex()
            .w_full()
            .child(
                div()
                    .id(("env-fileless-env", gx))
                    .test_support()
                    .aria_label(group.environment.clone())
                    .w_full()
                    .min_w_0()
                    .truncate()
                    .child(group.environment.clone()),
            )
            .children(rows)
    });
    let groups: Vec<_> = groups.collect();
    let section = v_flex()
        .id("env-fileless")
        .test_support()
        .w_full()
        .gap_1()
        .child(
            div()
                .text_color(muted)
                .child("These stored secrets belong to environments with no file here."),
        )
        .children(groups);
    Some(section.into_any_element())
}

/// A store that answers every `list` of one root from the first answer, so one action asks the
/// keychain for its keys once however many questions it puts to them.
pub(crate) struct ListOnce<'a> {
    inner: &'a dyn SecretStore,
    listed: Mutex<Option<ListAnswer>>,
}

type ListAnswer = (PathBuf, Result<Vec<SecretKey>, SecretError>);

impl<'a> ListOnce<'a> {
    pub(crate) fn new(inner: &'a dyn SecretStore) -> ListOnce<'a> {
        ListOnce {
            inner,
            listed: Mutex::new(None),
        }
    }
}

impl SecretStore for ListOnce<'_> {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        self.inner.get(key)
    }

    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.inner.set(key, value)
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.inner.delete(key)
    }

    fn list(&self, collection: &Path) -> Result<Vec<SecretKey>, SecretError> {
        let mut listed = self.listed.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((root, answer)) = listed.as_ref()
            && root == collection
        {
            return answer.clone();
        }
        let answer = self.inner.list(collection);
        *listed = Some((collection.to_owned(), answer.clone()));
        answer
    }
}
