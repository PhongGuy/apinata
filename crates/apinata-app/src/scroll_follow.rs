//! Keeps the focused control of a scroll area in view. GPUI scrolls nothing when Tab lands on a
//! control below the fold, and a focus handle knows nothing of its bounds, so each control is
//! wrapped in [`Followed`], which reports its bounds, and one [`FocusFollow::watch`] at the end
//! of the scroll area moves it just enough when focus lands in a wrapper.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, ElementId, FocusHandle, Pixels, ScrollHandle, Window, canvas, div,
};

/// How many frames a focus change stays armed, so a layout shift after it is still caught.
const ARMED_FRAMES: u8 = 3;

#[derive(Default)]
struct Inner {
    /// The wrappers painted this frame with their bounds, taken by the watch.
    marks: RefCell<Vec<(FocusHandle, Bounds<Pixels>)>>,
    /// The focus the watch last saw, so only a change arms it and wheeling away is left alone.
    last: RefCell<Option<FocusHandle>>,
    armed: Cell<u8>,
}

/// The meeting point of one scroll area's [`Followed`] wrappers and its watch; clones share it.
#[derive(Clone, Default)]
pub struct FocusFollow(Rc<Inner>);

impl FocusFollow {
    /// The marker that scrolls `scroll` to the focused wrapper. Draw it after every wrapper, as
    /// the last child of the scrolling box.
    pub fn watch(&self, scroll: &ScrollHandle) -> impl IntoElement + use<> {
        let (inner, scroll) = (self.0.clone(), scroll.clone());
        canvas(
            move |_, window, cx| inner.follow(&scroll, window, cx),
            |_, _, _, _| {},
        )
        .absolute()
        .size_0()
    }
}

impl Inner {
    fn follow(&self, scroll: &ScrollHandle, window: &mut Window, cx: &mut App) {
        let marks = std::mem::take(&mut *self.marks.borrow_mut());
        let focused = window.focused(cx);
        if *self.last.borrow() != focused {
            *self.last.borrow_mut() = focused;
            self.armed.set(ARMED_FRAMES);
        }
        if self.armed.get() == 0 {
            return;
        }
        self.armed.set(self.armed.get() - 1);
        let Some((_, at)) = marks.iter().find(|(h, _)| h.contains_focused(window, cx)) else {
            return;
        };
        let view = scroll.bounds();
        let mut offset = scroll.offset();
        if at.top() < view.top() {
            offset.y += view.top() - at.top();
        } else if at.bottom() > view.bottom() {
            // A wrapper taller than the area keeps its top in view.
            offset.y -= (at.bottom() - view.bottom()).min(at.top() - view.top());
        }
        // A wrapper already spanning the area is in view wherever it sits, so it stays put.
        let spans = at.left() <= view.left() && at.right() >= view.right();
        if !spans && at.left() < view.left() {
            offset.x += view.left() - at.left();
        } else if !spans && at.right() > view.right() {
            // A wrapper wider than the area keeps its left edge in view.
            offset.x -= (at.right() - view.right()).min(at.left() - view.left());
        }
        self.armed.set(0);
        if offset != scroll.offset() {
            scroll.set_offset(offset);
            window.request_animation_frame();
        }
    }
}

/// Wraps a control so focus anywhere inside it scrolls it into view through `follow`'s watch.
#[derive(IntoElement)]
pub struct Followed {
    id: ElementId,
    follow: FocusFollow,
    child: AnyElement,
}

impl Followed {
    /// `child` under the unique `id`, reporting to `follow`.
    pub fn new(
        id: impl Into<ElementId>,
        follow: &FocusFollow,
        child: impl IntoElement,
    ) -> Followed {
        Followed {
            id: id.into(),
            follow: follow.clone(),
            child: child.into_any_element(),
        }
    }
}

impl RenderOnce for Followed {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // The wrapper's own handle is no Tab stop; it only puts the wrapper on the focus path.
        let handle = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let marks = self.follow.0.clone();
        let reporter = {
            let handle = handle.clone();
            canvas(
                move |bounds, _, _| marks.marks.borrow_mut().push((handle, bounds)),
                |_, _, _, _| {},
            )
            // Pinned to the wrapper's corner: left at its static place, it would sit below
            // the child and report bounds one child height too low.
            .absolute()
            .top_0()
            .left_0()
            .size_full()
        };
        div()
            .id(self.id)
            .relative()
            .track_focus(&handle)
            .child(self.child)
            .child(reporter)
    }
}
