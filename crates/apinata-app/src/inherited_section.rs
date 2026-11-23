//! The environment editor's read-only block of what the `extends` chain gives: a collapsible
//! header, one dimmed row per inherited variable, and Override for a row nothing overrides yet.
//! A secret row shows only that it is secret; its value is never read for this block.

use std::rc::Rc;

use apinata_core::{InheritedState, InheritedVariable};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, TestSupportExt, Window, div};

use crate::scroll_follow::{FocusFollow, Followed};
use crate::style::{HEADER_NAME_WIDTH, SPACE_2, TEXT_SMALL};
use crate::ui::SolidRing as _;

/// The block's header line.
pub fn header_text(parent: &str, count: usize) -> String {
    format!("Inherited from {parent} ({count})")
}

/// What one row says: name, value or that it is secret, its source when that is not the direct
/// `parent`, and how the environment's own rows stand against it.
pub fn row_text(var: &InheritedVariable, parent: &str) -> String {
    let mut text = match &var.value {
        Some(value) if !var.secret => format!("{} = {value}", var.name),
        _ => format!("{} (secret)", var.name),
    };
    if var.from != parent {
        text.push_str(&format!(", from {}", var.from));
    }
    match var.state {
        InheritedState::OverriddenByOwn => text.push_str(", overridden here"),
        InheritedState::WinsOverOwn => {
            text.push_str(", this secret still wins over the row of the same name here")
        }
        _ => {}
    }
    text
}

/// A click on the header.
pub type OnToggle = Box<dyn Fn(&mut Window, &mut App)>;

/// A click on a row's Override, with the variable's name.
pub type OnOverride = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// The callbacks the block's controls report to.
pub struct Actions {
    /// Collapses or expands the rows.
    pub toggle: OnToggle,
    /// Copies the named inherited variable into the environment.
    pub override_row: OnOverride,
}

/// What the block shows.
pub struct Block<'a> {
    /// The environment's direct parent; `None` shows no block.
    pub parent: Option<&'a str>,
    /// What the chain gives.
    pub vars: &'a [InheritedVariable],
    /// Why the chain stops early, if it does.
    pub warning: Option<&'a str>,
    /// Whether the rows are expanded.
    pub open: bool,
}

/// The block, or nothing when the environment extends none; a broken chain shows its warning.
/// Ids: `env-inherited`, `env-inherited-toggle`, `("env-inherited-row", ix)`,
/// `("env-inherited-override", ix)`, `env-inherited-warning`.
pub fn section(
    block: Block<'_>,
    follow: &FocusFollow,
    actions: Actions,
    cx: &App,
) -> Option<AnyElement> {
    let Block {
        parent,
        vars,
        warning,
        open,
    } = block;
    let parent = parent?;
    let muted = cx.theme().muted_foreground;
    let header = header_text(parent, vars.len());
    let toggle = actions.toggle;
    let header = Button::new("env-inherited-toggle")
        .solid_ring(cx)
        .ghost()
        .compact()
        .label(header.clone())
        .accessibility_label(format!(
            "{header}, {}",
            if open { "expanded" } else { "collapsed" }
        ))
        .on_click(move |_, window, cx| toggle(window, cx));
    let warning = warning.map(|text| {
        div()
            .id("env-inherited-warning")
            .test_support()
            .aria_label(text.to_owned())
            .text_color(cx.theme().warning)
            .child(text.to_owned())
    });
    let rows = vars.iter().enumerate().filter(|_| open).map(|(ix, var)| {
        let text = row_text(var, parent);
        // Only a row nothing here overrides offers Override; the others already have a row.
        let action = (var.state == InheritedState::Inherited).then(|| {
            let on_click = Rc::clone(&actions.override_row);
            let name = var.name.clone();
            Followed::new(
                ("env-inherited-override-follow", ix),
                follow,
                Button::new(("env-inherited-override", ix))
                    .solid_ring(cx)
                    .ghost()
                    .compact()
                    .label("Override")
                    .accessibility_label(format!("Override {}", var.name))
                    .on_click(move |_, window, cx| on_click(&name, window, cx)),
            )
        });
        // Laid out like the inherited headers: name, dimmed value, then where it comes from.
        let overridden = var.state == InheritedState::OverriddenByOwn;
        let shown = match &var.value {
            Some(value) if !var.secret => value.clone(),
            _ => "secret".to_owned(),
        };
        let mut note = Vec::new();
        if var.from != parent {
            note.push(format!("from {}", var.from));
        }
        match var.state {
            InheritedState::OverriddenByOwn => note.push("overridden".to_owned()),
            InheritedState::WinsOverOwn => note.push("wins over own row".to_owned()),
            _ => {}
        }
        h_flex()
            .id(("env-inherited-row", ix))
            .test_support()
            .aria_label(text)
            .w_full()
            .gap(SPACE_2)
            .items_center()
            .text_size(TEXT_SMALL)
            .text_color(muted)
            .child(
                div()
                    .w(HEADER_NAME_WIDTH)
                    .flex_none()
                    .truncate()
                    .when(overridden, |d| d.line_through())
                    .child(var.name.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(overridden, |d| d.line_through())
                    .when(var.secret, |d| d.italic())
                    .child(shown),
            )
            .when(!note.is_empty(), |r| {
                r.child(div().flex_none().child(note.join(", ")))
            })
            .children(action)
    });
    let block = v_flex()
        .id("env-inherited")
        .test_support()
        .w_full()
        .gap_1()
        .child(Followed::new("env-inherited-toggle-follow", follow, header))
        .children(warning)
        .children(rows);
    Some(block.into_any_element())
}
