//! Exporting a finished run from the runner tab: ask where to save, render core's report in the
//! chosen format off the UI thread and write it atomically, so the file equals the CLI's.

use std::path::Path;

use apinata_core::ResultsReport;
use chrono::{DateTime, Utc};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, AppContext as _, Entity, Window};

use crate::export::write_export;
use crate::runner_model::RunnerModel;
use crate::ui;

/// A file format a run's results export to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    /// JUnit XML, for CI dashboards.
    Junit,
    /// The JSON report.
    Json,
    /// A self-contained HTML page.
    Html,
}

impl ReportFormat {
    /// The save dialog's suggested file name.
    pub fn file_name(self) -> &'static str {
        match self {
            ReportFormat::Junit => "results.xml",
            ReportFormat::Json => "results.json",
            ReportFormat::Html => "results.html",
        }
    }

    /// The button's label.
    pub fn label(self) -> &'static str {
        match self {
            ReportFormat::Junit => "JUnit XML",
            ReportFormat::Json => "JSON",
            ReportFormat::Html => "HTML",
        }
    }
}

/// Renders `report` in `format`, stamped with `now` and `hostname` the way the CLI stamps its
/// files; `bail` is whether the run stopped at its first failure.
pub fn render(
    report: &ResultsReport,
    format: ReportFormat,
    bail: bool,
    now: DateTime<Utc>,
    hostname: &str,
) -> String {
    match format {
        ReportFormat::Json => report.to_json(),
        ReportFormat::Junit => {
            let stamp = now.format("%Y-%m-%dT%H:%M:%S%.3f").to_string();
            report.to_junit(&stamp, hostname, bail)
        }
        ReportFormat::Html => {
            let stamp = now.format("%Y-%m-%d %H:%M:%S UTC").to_string();
            report.to_html(&stamp)
        }
    }
}

/// Prompts for a file, then writes the finished run of `model` there in `format` and toasts
/// the outcome. Does nothing until the run has finished.
pub fn export_results(
    model: &Entity<RunnerModel>,
    format: ReportFormat,
    window: &mut Window,
    cx: &mut App,
) {
    let (Some(report), bail, root) = model.read_with(cx, |m, _| {
        (m.export_report(), m.ran_with_bail(), m.root().to_path_buf())
    }) else {
        return;
    };
    let dir = root
        .parent()
        .map(Path::to_path_buf)
        .or_else(dirs::home_dir)
        .unwrap_or_default();
    let reply = cx.prompt_for_new_path(&dir, Some(format.file_name()));
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(path))) = reply.await else {
                return;
            };
            let job = cx.background_spawn(async move {
                let host = gethostname::gethostname();
                let text = render(&report, format, bail, Utc::now(), &host.to_string_lossy());
                write_export(&path, text.as_bytes())?;
                Ok::<_, String>(path)
            });
            let note = match job.await {
                Ok(path) => ui::info_toast(format!("Exported to {}", path.display())),
                Err(e) => Notification::error(format!("Could not export the results: {e}")),
            };
            cx.update(|window, cx| window.push_notification(note, cx))
                .ok();
        })
        .detach();
}
