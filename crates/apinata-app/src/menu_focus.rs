//! Focus repair for right-click menus built on the kit's `PopupMenu`.
//!
//! The kit's `PopupMenu::dismiss` loses its focus restore when the Escape key-up lands before
//! effects flush, leaving focus on the closed menu and the app's keys dead.

use std::rc::Rc;

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::prelude::*;
use gpui_kit::{App, DismissEvent, FocusHandle, Focusable, Subscription, Window};

use crate::palette::OpenCommandPalette;

/// What held focus when a right-press began, read in a capture-phase handler before the
/// press moves focus.
pub fn held(window: &Window, cx: &App) -> Option<FocusHandle> {
    window.focused(cx)
}

/// Settles focus once `menu` closes. `previous` yields what held focus before the menu opened
/// (None if it is gone by now) and `fallback` focuses a drawn element; both run after the
/// dismissal has settled. A dialog opened by a menu item keeps focus.
pub fn watch(
    menu: &mut Context<PopupMenu>,
    window: &mut Window,
    previous: impl FnOnce(&mut App) -> Option<FocusHandle> + 'static,
    fallback: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    let mut slots = Some((previous, fallback));
    menu.subscribe_in(
        &menu.entity(),
        window,
        move |menu, _, _: &DismissEvent, window, cx| {
            let Some((previous, fallback)) = slots.take() else {
                return;
            };
            let menu_focus = menu.focus_handle(cx);
            window.defer(cx, move |window, cx| {
                settle(&menu_focus, previous(cx).as_ref(), window, cx, fallback)
            });
        },
    )
    .detach();
}

/// `watch` for a menu that is still a plain `PopupMenu` value, as a tree's builder sees it: the
/// entity does not exist yet, so the menu is picked up by its focus handle once it is created.
pub fn watch_built(
    menu: &PopupMenu,
    cx: &mut App,
    previous: impl FnOnce(&mut App) -> Option<FocusHandle> + 'static,
    fallback: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    let mine = menu.focus_handle(cx);
    let slots = std::rc::Rc::new(std::cell::RefCell::new(Some((previous, fallback))));
    let watcher = std::rc::Rc::new(std::cell::RefCell::new(None::<Subscription>));
    let keep = Rc::clone(&watcher);
    *watcher.borrow_mut() = Some(cx.observe_new(
        move |created: &mut PopupMenu, window: Option<&mut Window>, cx: &mut Context<PopupMenu>| {
            if created.focus_handle(cx) != mine {
                return;
            }
            // One menu per watcher: stop observing creations now.
            keep.borrow_mut().take();
            let (Some(window), Some((previous, fallback))) = (window, slots.borrow_mut().take())
            else {
                return;
            };
            watch(cx, window, previous, fallback);
        },
    ));
}

fn settle(
    menu: &FocusHandle,
    previous: Option<&FocusHandle>,
    window: &mut Window,
    cx: &mut App,
    fallback: impl FnOnce(&mut Window, &mut App),
) {
    use gpui_kit::component::WindowExt as _;
    if window.has_active_dialog(cx) {
        return;
    }
    // Focus still on the closed menu is settled by identity: the action check below would
    // read the stale frame and call it live.
    if window.focused(cx).as_ref() == Some(menu) {
        match previous {
            Some(handle) => handle.focus(window, cx),
            None => fallback(window, cx),
        }
        return;
    }
    // What held focus is gone, and the kit's own restore may have put focus back on its stale
    // handle, which the action check below would also call live.
    if previous.is_none() || !window.is_action_available(&OpenCommandPalette, cx) {
        fallback(window, cx);
    }
}
