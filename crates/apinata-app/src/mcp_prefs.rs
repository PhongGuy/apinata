//! The "AI agents (MCP)" section of Preferences: how to connect an agent, what it may do,
//! and a link to each open collection's AI access tab. The server is a separate process, so
//! the section never claims a connection state.

use crate::ui::SolidRing as _;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, SharedString, TestSupportExt, WeakEntity, div};
use std::path::PathBuf;

use crate::app::AppState;
use crate::preferences::PreferencesView;
use crate::prefs_section::PrefsSection;
use crate::shell::Shell;
use crate::style::TEXT_SMALL;
use crate::ui;

/// The command an agent runs to reach a collection, with a placeholder for its folder.
pub const CONNECT_COMMAND: &str = "apinata mcp --collection <collection folder>";

/// What a connected agent may do without asking, and what waits for approval.
pub const APPROVAL_NOTE: &str = "Add the command to your AI assistant's MCP settings; it serves one collection over stdin and stdout. The assistant may read and edit the collection's files, which git tracks. A request it sends to a host other than localhost waits for your approval in this app, unless the collection's AI access lists the host as trusted.";

// Search words for rows whose labels are collection names.
const TERMS: [&str; 5] = ["ai", "agent", "mcp", "trust", "trusted hosts"];

/// The section, listing the collections open in `shell`'s workspace.
pub fn section(
    shell: Option<&WeakEntity<Shell>>,
    cx: &mut Context<PreferencesView>,
) -> PrefsSection {
    let command = div()
        .id("mcp-command")
        .test_support()
        .font_family(cx.theme().mono_font_family.clone())
        .child(CONNECT_COMMAND)
        .test_text(CONNECT_COMMAND);
    let mut section = PrefsSection::new("prefs-mcp", "AI agents (MCP)")
        .row_with_terms("pref-mcp-command", "Connect an agent", TERMS, command)
        .note(ui::form_note("prefs-mcp-note", APPROVAL_NOTE));
    let collections = open_collections(shell, cx);
    if collections.is_empty() {
        let none = div()
            .id("mcp-no-collections")
            .test_support()
            .text_size(TEXT_SMALL)
            .text_color(cx.theme().muted_foreground)
            .child("No collections are open.");
        return section.row_with_terms("pref-mcp-none", "Collections", TERMS, none);
    }
    for (ix, (root, name, pending)) in collections.into_iter().enumerate() {
        let shell = shell.cloned();
        let link = Button::new(("prefs-mcp-access", ix))
            .solid_ring(cx)
            .link()
            .label("AI access")
            .on_click(move |_, window, cx| {
                // The tab opens under the dialog, so the dialog goes first.
                window.close_dialog(cx);
                if let Some(shell) = shell.as_ref().and_then(WeakEntity::upgrade) {
                    shell.update(cx, |s, cx| s.open_ai_access(&root, window, cx));
                }
            });
        let waiting = (pending > 0).then(|| {
            let text = match pending {
                1 => "1 request waiting for approval".to_owned(),
                n => format!("{n} requests waiting for approval"),
            };
            div()
                .id(("prefs-mcp-pending", ix))
                .test_support()
                .text_size(TEXT_SMALL)
                .text_color(cx.theme().muted_foreground)
                .child(text.clone())
                .test_text(text)
        });
        let control = v_flex().child(h_flex().child(link)).children(waiting);
        let id = SharedString::from(format!("pref-mcp-collection-{ix}"));
        section = section.row_with_terms(id, name, TERMS, control);
    }
    section
}

// Each open collection's root, name and count of hosts waiting for approval, in open order.
fn open_collections(
    shell: Option<&WeakEntity<Shell>>,
    cx: &App,
) -> Vec<(PathBuf, SharedString, usize)> {
    let Some(workspace) = shell
        .and_then(WeakEntity::upgrade)
        .map(|s| s.read(cx).workspace().clone())
    else {
        return Vec::new();
    };
    let queue = cx
        .try_global::<AppState>()
        .map(|s| s.approvals.read(cx).queue().to_vec())
        .unwrap_or_default();
    let ws = workspace.read(cx);
    (ws.user_collections().into_iter())
        .map(|c| {
            let root = c.root().to_path_buf();
            // Prompts carry the canonical root.
            let canonical = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
            let pending = queue.iter().filter(|p| p.collection == canonical).count();
            (root.clone(), ws.collection_name(&root, cx), pending)
        })
        .collect()
}

/// The window's shell, found from its root view, for the section's links.
pub fn shell_of(window: &gpui_kit::Window, cx: &App) -> Option<WeakEntity<Shell>> {
    let root = window.root::<gpui_kit::base::Root>().flatten()?;
    let shell: Entity<Shell> = root.read(cx).view().clone().downcast::<Shell>().ok()?;
    Some(shell.downgrade())
}
