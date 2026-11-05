//! Export to Postman, OpenAPI or HTML documentation: ask where to save, export the collection's
//! saved files off the UI thread, write the file atomically and toast what was not carried over.
//! One environment exports the same way, as a Postman environment.

use std::path::Path;

use apinata_core::{Collection, IgnorePolicy, OpenOptions};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, AppContext as _, Entity, Window};

use crate::ui;
use crate::workspace::Workspace;

/// The file format an export writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Postman v2.1 collection JSON.
    Postman,
    /// OpenAPI 3.1, JSON or YAML by the chosen file's extension.
    OpenApi,
    /// One HTML documentation page.
    Html,
}

/// The save dialog's suggested name: the sanitized collection name plus the Postman suffix.
pub fn default_file_name(collection_name: &str) -> String {
    format!(
        "{}.postman_collection.json",
        apinata_core::export::file_stem(collection_name)
    )
}

/// The OpenAPI save dialog's suggested name: the sanitized collection name plus `.openapi.json`.
pub fn openapi_file_name(collection_name: &str) -> String {
    format!(
        "{}.openapi.json",
        apinata_core::export::file_stem(collection_name)
    )
}

/// The HTML save dialog's suggested name: the sanitized collection name plus `.docs.html`.
pub fn html_file_name(collection_name: &str) -> String {
    format!(
        "{}.docs.html",
        apinata_core::export::file_stem(collection_name)
    )
}

/// The environment save dialog's suggested name: the sanitized environment name plus the Postman
/// environment suffix.
pub fn environment_file_name(name: &str) -> String {
    format!(
        "{}.postman_environment.json",
        apinata_core::export::file_stem(name)
    )
}

/// The toast after an environment export; `dirty` notes that unsaved edits were left out.
pub fn environment_exported_message(path: &Path, dirty: bool) -> String {
    let shown = path.display();
    if dirty {
        format!("Exported to {shown} (unsaved edits not included)")
    } else {
        format!("Exported to {shown}")
    }
}

/// The toast after a successful export, counting the notes when there are any.
pub fn exported_message(file: &Path, notes: usize) -> String {
    let shown = file.display();
    match notes {
        0 => format!("Exported to {shown}"),
        1 => format!("Exported to {shown} (1 item not carried over)"),
        n => format!("Exported to {shown} ({n} items not carried over)"),
    }
}

/// Writes the export atomically; the error names the file, as the CLI's does.
pub fn write_export(path: &Path, bytes: &[u8]) -> Result<(), String> {
    apinata_core::export::write_atomic(path, bytes)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Prompts for a target file, starting in the collection's parent directory, then exports the
/// collection at `root` there in `format` and toasts the outcome.
pub fn export_collection(
    workspace: &Entity<Workspace>,
    root: &Path,
    format: ExportFormat,
    window: &mut Window,
    cx: &mut App,
) {
    if crate::scratch::refuse(workspace, root, window, cx) {
        return;
    }
    let name = workspace.read(cx).collection_name(root, cx);
    let dir = root
        .parent()
        .map(Path::to_path_buf)
        .or_else(dirs::home_dir)
        .unwrap_or_default();
    let file_name = match format {
        ExportFormat::Postman => default_file_name(&name),
        ExportFormat::OpenApi => openapi_file_name(&name),
        ExportFormat::Html => html_file_name(&name),
    };
    let reply = cx.prompt_for_new_path(&dir, Some(file_name.as_str()));
    let root = root.to_path_buf();
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(path))) = reply.await else {
                return;
            };
            // Saved files only, opened fresh off the UI thread, as the actor's plan_saved does.
            let job = cx.background_spawn(async move {
                let options = OpenOptions::new().ignore(IgnorePolicy::BrunoApp);
                let collection =
                    Collection::open_with(&root, &options).map_err(|e| e.to_string())?;
                let (text, notes) = match format {
                    ExportFormat::Postman => {
                        let out = apinata_core::export::postman(&collection);
                        (out.json, out.notes.len())
                    }
                    ExportFormat::OpenApi => {
                        let out = apinata_core::export::openapi(&collection);
                        // The chosen file name decides the syntax, so renaming to .yaml is enough.
                        let text = if apinata_core::export::is_yaml_path(&path) {
                            out.yaml()?
                        } else {
                            out.json.clone()
                        };
                        (text, out.notes.len())
                    }
                    ExportFormat::Html => {
                        let out = apinata_core::export::html(&collection);
                        (out.html, out.notes.len())
                    }
                };
                write_export(&path, text.as_bytes())?;
                Ok::<_, String>((path, notes))
            });
            let note = match job.await {
                Ok((path, notes)) => ui::info_toast(exported_message(&path, notes)),
                Err(e) => Notification::error(format!("Could not export the collection: {e}")),
            };
            cx.update(|window, cx| window.push_notification(note, cx))
                .ok();
        })
        .detach();
}

/// Prompts for a target file, starting in the collection's parent directory, then writes the
/// saved environment `name` of the collection at `root` there as Postman environment JSON and
/// toasts the outcome; `dirty` adds a note that unsaved edits were not included.
pub fn export_environment(root: &Path, name: &str, dirty: bool, window: &mut Window, cx: &mut App) {
    let dir = root
        .parent()
        .map(Path::to_path_buf)
        .or_else(dirs::home_dir)
        .unwrap_or_default();
    let file_name = environment_file_name(name);
    let reply = cx.prompt_for_new_path(&dir, Some(file_name.as_str()));
    let root = root.to_path_buf();
    let name = name.to_string();
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(path))) = reply.await else {
                return;
            };
            // Core reads the saved files, so the export is what is on disk, secrets blanked.
            let job = cx.background_spawn(async move {
                let options = OpenOptions::new().ignore(IgnorePolicy::BrunoApp);
                let collection =
                    Collection::open_with(&root, &options).map_err(|e| e.to_string())?;
                let out = apinata_core::export::postman_environment(&collection, &name)
                    .ok_or_else(|| "it no longer exists".to_string())?;
                write_export(&path, out.json.as_bytes())?;
                Ok::<_, String>(path)
            });
            let note = match job.await {
                Ok(path) => ui::info_toast(environment_exported_message(&path, dirty)),
                Err(e) => Notification::error(format!("Could not export the environment: {e}")),
            };
            cx.update(|window, cx| window.push_notification(note, cx))
                .ok();
        })
        .detach();
}
