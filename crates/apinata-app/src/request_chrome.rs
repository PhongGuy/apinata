//! The request editor's chrome (breadcrumb, URL row parts, section bar) as views of their own,
//! so a key typed into a section body reuses their last frame instead of rebuilding them.
//!
//! Each part is a [`crate::cached_part::Part`] the editor draws cached while its inputs hold.

use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Window};

use crate::cached_part::{Part, PartOwner};
use crate::request_editor::RequestEditor;

/// One part of the request editor's chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromePart {
    /// The collection, folder and request name line.
    Breadcrumb,
    /// The method picker, or the custom method field in its place.
    Method,
    /// The URL input.
    Url,
    /// Save, Send (or Cancel, Stop) and Generate code.
    Buttons,
    /// The section tab strip.
    Sections,
}

impl ChromePart {
    /// Every part, in the order the editor holds them.
    pub const ALL: [ChromePart; 5] = [
        ChromePart::Breadcrumb,
        ChromePart::Method,
        ChromePart::Url,
        ChromePart::Buttons,
        ChromePart::Sections,
    ];

    // Only the button group sizes to its content sideways; every other part fills the width
    // its parent's flex gives it.
    pub(crate) fn sized_by_content(self) -> bool {
        self == ChromePart::Buttons
    }
}

/// A chrome part's view: it renders its part from the editor that owns it.
pub type Chrome = Part<RequestEditor>;

impl PartOwner for RequestEditor {
    type Part = ChromePart;

    fn render_part(
        &mut self,
        part: ChromePart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_chrome(part, window, cx)
    }
}
