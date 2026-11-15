//! The About dialog: the app's name, running version and licence, with links to the project.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{App, TestSupportExt as _, Window, actions, div};

use crate::style::SPACE_1;
use crate::ui::{self, dialog_footer};

actions!(
    apinata,
    [
        /// Opens the About dialog.
        OpenAbout
    ]
);

/// The project's repository page.
pub const REPOSITORY_URL: &str = "https://github.com/PhongGuy/apinata";

/// Where issues are reported.
pub const ISSUES_URL: &str = "https://github.com/PhongGuy/apinata/issues";

/// The licence the app ships under.
pub const LICENCE: &str = "MIT OR Apache-2.0";

/// The version line, from the build's own package version.
pub fn version_text() -> String {
    format!("Version {}", env!("CARGO_PKG_VERSION"))
}

/// Opens the About dialog, id `about-dialog`, on `window`.
pub fn open(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |d, window, _| {
        let version = version_text();
        let licence = format!("Licence: {LICENCE}");
        // Enter acts on the focused link or button only, never as a confirm of the dialog.
        ui::with_close(crate::dialog_frame::place(d, window), "About Apinata")
            .on_ok(|_, _, _| false)
            .child(
                v_flex()
                    .id("about-dialog")
                    .test_support()
                    .key_context(ui::DIALOG_FOOTER_CONTEXT)
                    .gap(SPACE_1)
                    .child(line("about-version", version))
                    .child(line("about-licence", licence))
                    .child(link("about-repository", "Repository", REPOSITORY_URL))
                    .child(link("about-issues", "Report an issue", ISSUES_URL)),
            )
            .child(dialog_footer(
                None,
                [],
                Button::new("about-close")
                    .label("Close")
                    .primary()
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            ))
    });
}

fn line(id: &'static str, text: String) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .child(text)
}

// A link opens through the app's browser opener, the same way the update notice's page does.
fn link(id: &'static str, label: &'static str, url: &'static str) -> impl IntoElement {
    div().child(
        Button::new(id)
            .link()
            .label(label)
            .tooltip(url)
            .on_click(move |_, _, cx| cx.open_url(url)),
    )
}
