//! `apinata self-update`: check the signed release manifest and replace a standalone binary.

use std::path::PathBuf;
use std::process::ExitCode;

use apinata_core::update::{
    self, Available, InstallKind, Os, Product, Reason, UpdateConfig, UpdateError,
};

use crate::{Format, SelfUpdateArgs};

/// Exit code for an update reported by `--check`.
const EXIT_AVAILABLE: u8 = 2;

/// Checks for a newer release and, for a standalone copy, installs it after confirmation.
/// Exits 0 when up to date, installed, declined or not replaceable, 2 when `--check` finds an
/// update, and 3 on any error.
pub fn run(args: &SelfUpdateArgs) -> ExitCode {
    match self_update(args) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(crate::EXIT_USAGE)
        }
    }
}

fn self_update(args: &SelfUpdateArgs) -> anyhow::Result<u8> {
    let cfg = update::build_config(Product::Cli)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let current = &cfg.current_version;
    let json = matches!(args.format, Format::Json);
    let found = runtime.block_on(update::check(&cfg))?;
    if json {
        print_json(current, found.as_ref())?;
    }
    let Some(available) = found else {
        if !json {
            outln!("Apinata {current} is up to date.");
        }
        return Ok(0);
    };
    let exe = executable()?;
    let kind = classify(&exe);

    if !json {
        outln!(
            "Apinata {} is available (this is {current}).",
            available.version
        );
        print_notes(&available, &cfg);
    }
    // A copy the updater does not own is updated by whatever installed it, so the user is
    // pointed there instead; that is a normal outcome, not an error.
    let target = match kind {
        InstallKind::Standalone(target) => target,
        InstallKind::NotOwned(reason) => return Ok(not_owned(&reason, &cfg, json)),
        _ => return Ok(not_owned(&Reason::Unrecognised, &cfg, json)),
    };
    if args.check {
        return Ok(EXIT_AVAILABLE);
    }
    if !args.yes && !confirm(&available) {
        outln!("Nothing was installed.");
        return Ok(0);
    }
    install(&runtime, &available, &target)?;
    outln!("Installed Apinata {}.", available.version);
    Ok(0)
}

// JSON output keeps stdout to the document, so the pointer goes to stderr there.
fn not_owned(reason: &Reason, cfg: &UpdateConfig, json: bool) -> u8 {
    let text = format!(
        "This copy cannot update itself: {reason}.\nDownload it from {}",
        cfg.download_page
    );
    if json {
        eprintln!("{text}");
    } else {
        outln!("{text}");
    }
    0
}

/// The offer's notes as plain text: each release newer than this build under its heading,
/// then where older notes are. An offer whose manifest lists no notes asset shows its
/// one-line notes as before.
fn print_notes(available: &Available, cfg: &UpdateConfig) {
    let Some(notes) = &available.release_notes else {
        if available.notes_error.is_some() {
            outln!("Release notes couldn't be loaded.");
        }
        for line in available.notes.iter().flat_map(|n| n.lines()) {
            outln!("  {line}");
        }
        return;
    };
    for release in &notes.releases {
        outln!();
        outln!("{}", release.heading());
        if let Some(intro) = &release.intro {
            outln!("  {intro}");
        }
        for group in &release.groups {
            outln!("  {}", group.title);
            for item in &group.items {
                outln!("    - {}", item.line());
            }
        }
    }
    if notes.older_not_listed {
        outln!();
        outln!("Older versions: see {}", cfg.download_page);
    }
}

/// `{current, available, notes, notes_error}`, where `notes` lists the releases newer than
/// this build, newest first, or is null when none loaded.
fn print_json(current: &str, available: Option<&Available>) -> anyhow::Result<()> {
    let notes = available
        .and_then(|a| a.release_notes.as_ref())
        .map(|n| serde_json::to_value(&n.releases))
        .transpose()?;
    let doc = serde_json::json!({
        "current": current,
        "available": available.map(|a| &a.version),
        "notes": notes,
        "notes_error": available.and_then(|a| a.notes_error.as_ref()),
    });
    outln!("{}", serde_json::to_string_pretty(&doc)?);
    Ok(())
}

/// Stages the download beside `target`, so the swap is a rename within one directory.
fn install(
    runtime: &tokio::runtime::Runtime,
    available: &Available,
    target: &std::path::Path,
) -> Result<(), UpdateError> {
    let dir = target
        .parent()
        .ok_or_else(|| UpdateError::NotWritable(target.to_path_buf()))?;
    let staged = runtime.block_on(update::download(available, dir, |_, _| {}))?;
    update::install_standalone(staged, target)
}

/// Asks on stdin; only `y` or `yes`, in any case, agrees, and end of input declines.
fn confirm(available: &Available) -> bool {
    out!("Install Apinata {}? [y/N] ", available.version);
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    let answer = answer.trim();
    answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes")
}

/// This executable's canonical path. Debug builds let tests point it at a temporary copy.
fn executable() -> Result<PathBuf, UpdateError> {
    #[cfg(debug_assertions)]
    let exe = match std::env::var_os("APINATA_UPDATE_TEST_EXE") {
        Some(path) => Ok(PathBuf::from(path)),
        None => std::env::current_exe(),
    };
    #[cfg(not(debug_assertions))]
    let exe = std::env::current_exe();
    let io = |path: PathBuf, e: std::io::Error| UpdateError::Io {
        path,
        message: e.to_string(),
    };
    let exe = exe.map_err(|e| io(PathBuf::from("apinata"), e))?;
    std::fs::canonicalize(&exe).map_err(|e| io(exe, e))
}

fn classify(exe: &std::path::Path) -> InstallKind {
    let Some(os) = Os::current() else {
        return InstallKind::NotOwned(Reason::Unrecognised);
    };
    let app = if os == Os::Windows {
        "apinata-app.exe"
    } else {
        "apinata-app"
    };
    let sibling_app = exe.with_file_name(app).exists();
    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
    update::install_kind(Product::Cli, os, exe, appimage.as_deref(), sibling_app)
}
