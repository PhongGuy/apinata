//! Export to Postman, OpenAPI or HTML documentation: ask where to save, export the collection's
//! saved files off the UI thread, write the file atomically and toast what was not carried over.
//! One environment exports the same way, as a Postman environment; every environment of a
//! collection or of the global set goes to a chosen folder, one file each.

use std::path::{Path, PathBuf};

use apinata_core::export::{ExportNote, ExportNoteKind};
use apinata_core::{Collection, GlobalEnvironments, IgnorePolicy, OpenOptions};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Sizable as _, WindowExt as _};
use gpui_kit::{
    App, AppContext as _, Entity, IntoElement as _, ParentElement as _, PathPromptOptions,
    Styled as _, Window,
};

use crate::ui;
use crate::workspace::Workspace;

/// The file format an export writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Postman v2.1 collection JSON.
    Postman {
        /// Write scripts as they are instead of translating them to Postman's script API.
        keep_scripts: bool,
    },
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

/// The toast after exporting `count` environments to the folder `dir`; `dirty` notes that
/// unsaved edits were left out.
pub fn environments_exported_message(dir: &Path, count: usize, dirty: bool) -> String {
    let noun = if count == 1 {
        "environment"
    } else {
        "environments"
    };
    let done = format!("Exported {count} {noun} to {}", dir.display());
    if dirty {
        format!("{done} (unsaved edits not included)")
    } else {
        done
    }
}

/// The toast after a successful export, naming the file: how many items were not carried
/// over, or, when nothing was lost, how many were changed to fit.
pub fn exported_message(file: &Path, notes: &[ExportNote]) -> String {
    let shown = file_name(file);
    match (
        count(notes, ExportNoteKind::Lost),
        count(notes, ExportNoteKind::Changed),
    ) {
        (0, 0) => format!("Exported to {shown}"),
        (0, 1) => format!("Exported to {shown} (1 item changed to fit)"),
        (0, n) => format!("Exported to {shown} ({n} items changed to fit)"),
        (1, _) => format!("Exported to {shown} (1 item not carried over)"),
        (n, _) => format!("Exported to {shown} ({n} items not carried over)"),
    }
}

/// The export report's summary: how many items `file` could not carry, or that everything was
/// carried with some changed to fit.
pub fn report_summary(file: &Path, notes: &[ExportNote]) -> String {
    let shown = file_name(file);
    match (
        count(notes, ExportNoteKind::Lost),
        count(notes, ExportNoteKind::Changed),
    ) {
        (0, 1) => format!("Everything was carried over to {shown}; 1 item was changed to fit."),
        (0, n) => format!("Everything was carried over to {shown}; {n} items were changed to fit."),
        (1, _) => format!("1 item was not carried over to {shown}."),
        (n, _) => format!("{n} items were not carried over to {shown}."),
    }
}

/// The report's sections, "Left out" then "Changed", each only when it has notes, without
/// backticks. Each kind of note is one line: its words capitalised, then every place it
/// applies to on a line of its own, in note order. Words after a detail's first ": " belong
/// to its one place, so notes that differ only there share a kind.
pub fn report_groups(notes: &[ExportNote]) -> Vec<(Option<&'static str>, Vec<String>)> {
    [
        ("Left out", ExportNoteKind::Lost),
        ("Changed", ExportNoteKind::Changed),
    ]
    .into_iter()
    .filter_map(|(heading, kind)| {
        let mut groups: Vec<(&str, Vec<String>)> = Vec::new();
        for note in notes.iter().filter(|n| n.kind == kind) {
            let (words, place) = match note.detail.split_once(": ") {
                Some((words, own)) => (words, format!("{}: {own}", note.source)),
                None => (note.detail.as_str(), note.source.clone()),
            };
            match groups.iter_mut().find(|(w, _)| *w == words) {
                Some((_, places)) => places.push(place),
                None => groups.push((words, vec![place])),
            }
        }
        let lines: Vec<String> = groups
            .into_iter()
            .map(|(words, places)| {
                let mut line = crate::import_report::failure_text(words);
                for place in places {
                    line.push('\n');
                    line.push_str(&place);
                }
                line.replace('`', "")
            })
            .collect();
        (!lines.is_empty()).then_some((Some(heading), lines))
    })
    .collect()
}

// The file's own name; the folder is one click away in the toast.
fn file_name(file: &Path) -> String {
    file.file_name().map_or_else(
        || file.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn count(notes: &[ExportNote], kind: ExportNoteKind) -> usize {
    notes.iter().filter(|n| n.kind == kind).count()
}

// The toast after a collection export, with a button that shows the file in its folder. When
// the report has anything, "Show details" opens it, so the reader can judge the file before
// sharing it, and the toast stays until dismissed.
fn exported_toast(path: &Path, notes: Vec<ExportNote>) -> Notification {
    let toast = ui::info_toast(exported_message(path, &notes)).autohide(notes.is_empty());
    let folder = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let details =
        (!notes.is_empty()).then(|| (report_summary(path, &notes), report_groups(&notes)));
    toast.content(move |_, _, cx| {
        let this = cx.entity();
        let details = details.clone().map(|(summary, groups)| {
            let this = this.clone();
            Button::new("export-details")
                .ghost()
                .small()
                .label("Show details")
                .on_click(move |_, window, cx| {
                    this.update(cx, |n, cx| n.dismiss(window, cx));
                    crate::import_report::open_report(
                        "Export report",
                        REPORT_IDS,
                        summary.clone(),
                        groups.clone(),
                        window,
                        cx,
                    );
                })
        });
        let folder = folder.clone();
        let show = Button::new("export-show-folder")
            .ghost()
            .small()
            .label("Show in file manager")
            .on_click(move |_, window, cx| {
                this.update(cx, |n, cx| n.dismiss(window, cx));
                if !crate::open_external::open_path(&folder, cx) {
                    let failure = Notification::error("Could not open the file manager");
                    window.push_notification(failure, cx);
                }
            });
        gpui_kit::component::h_flex()
            .mt(crate::style::SPACE_2)
            .gap(crate::style::SPACE_2)
            .children(details)
            .child(show)
            .into_any_element()
    })
}

const REPORT_IDS: crate::import_report::ReportIds = crate::import_report::ReportIds {
    summary: "export-summary",
    group: "export-group",
    note: "export-note",
    scroll: "export-report-scroll",
    close: "export-report-close",
};

/// Writes the export atomically; the error names the file in plain words, as the CLI's does.
pub fn write_export(path: &Path, bytes: &[u8]) -> Result<(), String> {
    apinata_core::export::write_atomic(path, bytes).map_err(|e| e.to_string())
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
        ExportFormat::Postman { .. } => default_file_name(&name),
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
                    ExportFormat::Postman { keep_scripts } => {
                        let options =
                            apinata_core::export::PostmanOptions::new().keep_scripts(keep_scripts);
                        let out = apinata_core::export::postman_with(&collection, &options);
                        (out.json, out.notes)
                    }
                    ExportFormat::OpenApi => {
                        let out = apinata_core::export::openapi(&collection);
                        // The chosen file name decides the syntax, so renaming to .yaml is enough.
                        let text = if apinata_core::export::is_yaml_path(&path) {
                            out.yaml()?
                        } else {
                            out.json.clone()
                        };
                        (text, out.notes)
                    }
                    ExportFormat::Html => {
                        let out = apinata_core::export::html(&collection);
                        (out.html, out.notes)
                    }
                };
                write_export(&path, text.as_bytes())?;
                Ok::<_, String>((path, notes))
            });
            let note = match job.await {
                Ok((path, notes)) => exported_toast(&path, notes),
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

/// Where a folder export reads its environments from.
#[derive(Debug, Clone)]
pub enum EnvironmentSource {
    /// The collection at this root.
    Collection(PathBuf),
    /// The global environments directory.
    Global(PathBuf),
}

/// Prompts for a folder, then writes every saved environment of `source` there as Postman
/// environment files, named and numbered as `apinata export --env-dir` names them, replacing
/// files of the same name. `available` is how many environments there are; with none, a toast
/// says so instead of prompting. `dirty` adds a note that unsaved edits were not included.
pub fn export_environments(
    source: EnvironmentSource,
    available: usize,
    dirty: bool,
    window: &mut Window,
    cx: &mut App,
) {
    if available == 0 {
        window.push_notification(
            Notification::error("There are no environments to export"),
            cx,
        );
        return;
    }
    let reply = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Export Postman environments to".into()),
    });
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(paths))) = reply.await else {
                return;
            };
            let Some(dir) = paths.into_iter().next() else {
                return;
            };
            // Core reads the saved files, so the export is what is on disk, secrets blanked.
            let job = cx.background_spawn(async move {
                let files = match source {
                    EnvironmentSource::Collection(root) => {
                        let options = OpenOptions::new().ignore(IgnorePolicy::BrunoApp);
                        let collection =
                            Collection::open_with(&root, &options).map_err(|e| e.to_string())?;
                        apinata_core::export::postman_environment_files(&collection)
                    }
                    EnvironmentSource::Global(globals) => {
                        let globals =
                            GlobalEnvironments::open(&globals).map_err(|e| e.to_string())?;
                        apinata_core::export::postman_global_environment_files(&globals)
                    }
                };
                for file in &files {
                    write_export(&dir.join(&file.file_name), file.export.json.as_bytes())?;
                }
                Ok::<_, String>((dir, files.len()))
            });
            let note = match job.await {
                Ok((dir, count)) => {
                    ui::info_toast(environments_exported_message(&dir, count, dirty))
                }
                Err(e) => Notification::error(format!("Could not export the environments: {e}")),
            };
            cx.update(|window, cx| window.push_notification(note, cx))
                .ok();
        })
        .detach();
}
