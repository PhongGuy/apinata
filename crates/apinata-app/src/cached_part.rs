//! A piece of a view's render drawn as a view of its own, so a frame that changes none of the
//! piece's inputs reuses its last frame instead of rebuilding it.
//!
//! A cached view's box is sized by its parent's style alone, so each part records the size
//! its content took; the owner draws a part uncached (laid out by content, and measured)
//! whenever its inputs or that size moved, and cached at the measured size otherwise.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Empty, Entity, Pixels, SharedString, Size, StyleRefinement, TextStyle,
    WeakEntity, Window, canvas, div, size,
};

/// A view whose render is split into parts, each drawn by a [`Part`] that calls back in here.
pub trait PartOwner: Sized + 'static {
    /// Names one part of the owner's render.
    type Part: Copy + 'static;

    /// Draws `part` for its [`Part`] view.
    fn render_part(
        &mut self,
        part: Self::Part,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement;
}

/// One part's view: it renders its part from the owner and measures what it drew.
pub struct Part<O: PartOwner> {
    owner: WeakEntity<O>,
    part: O::Part,
    by_content: bool,
    size: Rc<Cell<Option<Size<Pixels>>>>,
    renders: usize,
}

impl<O: PartOwner> Part<O> {
    /// How many times this part rendered, for tests that pin what a key redraws.
    pub fn renders(&self) -> usize {
        self.renders
    }

    // The measured extent a cached part is styled with: the whole size for a part that sizes
    // to its content sideways, the height alone for one whose width the parent decides.
    fn extent(by_content: bool, took: Size<Pixels>) -> Size<Pixels> {
        if by_content {
            took
        } else {
            size(Pixels::ZERO, took.height)
        }
    }
}

impl<O: PartOwner> Render for Part<O> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let part = self.part;
        let content = self
            .owner
            .update(cx, |o, cx| o.render_part(part, window, cx))
            .unwrap_or_else(|_| Empty.into_any_element());
        // The marker reads the size the content took; a change after a cached draw asks for
        // another frame, which the owner then lays out by content again.
        let (cell, by_content) = (Rc::clone(&self.size), self.by_content);
        let measure = canvas(
            move |bounds, window, _| {
                let old = cell.replace(Some(bounds.size));
                let extent = |s| Self::extent(by_content, s);
                if old.is_some_and(|o| extent(o) != extent(bounds.size)) {
                    window.request_animation_frame();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let root = if by_content {
            h_flex()
        } else {
            v_flex().w_full()
        };
        root.relative().child(content).child(measure)
    }
}

// What every part's layout depends on beyond its own inputs: a change lays it out again.
#[derive(Clone, PartialEq)]
struct Layout {
    rem: Pixels,
    scale: f32,
    fonts: (Pixels, Pixels, SharedString, SharedString),
    text: TextStyle,
}

impl Layout {
    fn of(window: &Window, cx: &App) -> Self {
        let theme = cx.theme();
        Layout {
            rem: window.rem_size(),
            scale: window.scale_factor(),
            fonts: (
                theme.font_size,
                theme.mono_font_size,
                theme.font_family.clone(),
                theme.mono_font_family.clone(),
            ),
            text: window.text_style(),
        }
    }
}

/// A part's view and what it was last drawn from: its inputs `I`, the layout, and the
/// measured size its cached box was styled with.
pub(crate) struct Slot<O: PartOwner, I> {
    view: Entity<Part<O>>,
    seen: Option<(Layout, I)>,
    styled: Option<Size<Pixels>>,
}

impl<O: PartOwner, I: PartialEq> Slot<O, I> {
    /// A slot for `part` of `owner`; `by_content` when the part sizes to its content sideways.
    pub(crate) fn new(owner: WeakEntity<O>, part: O::Part, by_content: bool, cx: &mut App) -> Self {
        let view = cx.new(|_| Part {
            owner,
            part,
            by_content,
            size: Rc::default(),
            renders: 0,
        });
        Slot {
            view,
            seen: None,
            styled: None,
        }
    }

    /// The part's view.
    pub(crate) fn view(&self) -> &Entity<Part<O>> {
        &self.view
    }

    /// Draws the part afresh in the next frame, whatever its inputs.
    pub(crate) fn invalidate(&mut self) {
        self.seen = None;
    }

    /// The part for this frame. It is reused from its last frame only while `inputs`, the
    /// layout and its measured size are what its cached box was last styled with; otherwise
    /// it is drawn inside a box `outer` styles and laid out by its content, which measures it.
    pub(crate) fn element(
        &mut self,
        inputs: I,
        outer: StyleRefinement,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let seen = (Layout::of(window, cx), inputs);
        let part = self.view.read(cx);
        let took = part
            .size
            .get()
            .map(|s| Part::<O>::extent(part.by_content, s));
        let reuse = took.is_some()
            && self.seen.as_ref() == Some(&seen)
            && self.styled.is_none_or(|styled| Some(styled) == took);
        self.seen = Some(seen);
        let Some(extent) = took.filter(|_| reuse) else {
            self.styled = None;
            let mut wrapper = div();
            *wrapper.style() = outer;
            return wrapper.child(self.view.clone()).into_any_element();
        };
        self.styled = Some(extent);
        let mut style = outer.h(extent.height);
        if extent.width > Pixels::ZERO {
            style = style.w(extent.width);
        }
        self.view.clone().cached(style).into_any_element()
    }
}

/// The identity of a shared value, compared by allocation. It holds the allocation alive, so
/// a value freed and another allocated at the same address never compare equal.
pub struct Same<T>(std::sync::Weak<T>);

impl<T> Same<T> {
    /// The identity of `value`.
    pub fn of(value: &std::sync::Arc<T>) -> Self {
        Same(std::sync::Arc::downgrade(value))
    }
}

impl<T> Clone for Same<T> {
    fn clone(&self) -> Self {
        Same(std::sync::Weak::clone(&self.0))
    }
}

impl<T> PartialEq for Same<T> {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Weak::ptr_eq(&self.0, &other.0)
    }
}
