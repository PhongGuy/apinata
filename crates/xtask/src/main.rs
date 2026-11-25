use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "Repository automation for apinata")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Type-check the headless crates for x86_64-pc-windows-gnu, skipping without mingw.
    CheckWindows {
        /// Fail instead of skipping when the cross compiler or target is missing.
        #[arg(long)]
        require: bool,
    },
    /// Install a desktop entry and icon for BINARY under an explicit data home (Linux).
    Desktop {
        /// The XDG data home the entry and icon go under.
        #[arg(long)]
        data_home: PathBuf,
        /// The app binary the entry launches; it need not exist yet.
        #[arg(long)]
        binary: PathBuf,
        /// The name launchers show.
        #[arg(long, default_value = "Apinata")]
        name: String,
        /// Remove the entry and icon instead.
        #[arg(long)]
        remove: bool,
    },
    /// Install a desktop entry and icon so a dev build shows the app icon (Linux).
    DevDesktop {
        /// Remove the dev desktop entry and icon instead.
        #[arg(long)]
        remove: bool,
    },
    /// Render the PNG, ICO and ICNS exports into assets/icon/generated.
    Icons {
        /// Report exports that differ from a fresh render instead of writing them.
        #[arg(long)]
        check: bool,
    },
    /// Write the complete cargo-packager config, with signing from the environment.
    PackagerConfig {
        /// Directory cargo-packager writes packages into.
        #[arg(long)]
        out_dir: PathBuf,
        /// File the JSON config is written to, readable only by the owner.
        #[arg(long)]
        output: PathBuf,
        /// Target triple whose release directory holds the binaries.
        #[arg(long)]
        target: Option<String>,
    },
    /// Write the updater's notes.json from published release bodies and this tag's notes.
    NotesJson {
        /// The release version, a stable semver.
        #[arg(long)]
        version: String,
        /// The release date, YYYY-MM-DD.
        #[arg(long)]
        date: String,
        /// This tag's generate-notes body.
        #[arg(long)]
        generated: PathBuf,
        /// Directory of releases API JSON pages.
        #[arg(long)]
        published: PathBuf,
        /// This tag's annotation, the source of the intro.
        #[arg(long)]
        intro: Option<PathBuf>,
        /// The label categories; .github/release.yml when omitted.
        #[arg(long)]
        categories: Option<PathBuf>,
        /// File notes.json is written to.
        #[arg(long)]
        output: PathBuf,
    },
    /// Write latest.json for the release artifacts in DIR.
    UpdateManifest {
        /// The release version, a stable semver.
        #[arg(long)]
        version: String,
        /// URL the artifacts are served under.
        #[arg(long)]
        base_url: String,
        /// Plain-text release notes file.
        #[arg(long)]
        notes: PathBuf,
        /// File of "<target-id> <file-name>" lines.
        #[arg(long)]
        map: PathBuf,
        /// Publication time, stated as given.
        #[arg(long)]
        pub_date: Option<String>,
        /// The updater's notes file in DIR, listed as the manifest's notes_asset.
        #[arg(long)]
        notes_asset: Option<String>,
        /// Directory holding the artifacts; latest.json is written here.
        dir: PathBuf,
    },
}

fn main() -> anyhow::Result<ExitCode> {
    match Cli::parse().command {
        Cmd::CheckWindows { require } => xtask::windows_check::run(require),
        Cmd::Desktop {
            data_home,
            binary,
            name,
            remove,
        } => xtask::dev_desktop::run_explicit(&data_home, &binary, &name, remove),
        Cmd::DevDesktop { remove } => xtask::dev_desktop::run(remove),
        Cmd::Icons { check } => xtask::icons::run(check),
        Cmd::PackagerConfig {
            out_dir,
            output,
            target,
        } => xtask::packager_config::run(&out_dir, &output, target.as_deref()),
        Cmd::NotesJson {
            version,
            date,
            generated,
            published,
            intro,
            categories,
            output,
        } => xtask::notes_json::run(
            &version,
            &date,
            &generated,
            &published,
            intro.as_deref(),
            categories.as_deref(),
            &output,
        ),
        Cmd::UpdateManifest {
            version,
            base_url,
            notes,
            map,
            pub_date,
            notes_asset,
            dir,
        } => xtask::update_manifest::run(
            &version,
            &base_url,
            &notes,
            &map,
            pub_date.as_deref(),
            notes_asset.as_deref(),
            &dir,
        ),
    }
}
