//! The section both environment editors show under the variable table: stored secrets the
//! environment's file no longer marks secret, each with a Remove behind a confirmation. Only
//! names appear here, never a value.

use apinata_core::EnvVariable;
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
