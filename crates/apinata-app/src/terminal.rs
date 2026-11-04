//! Opening a terminal in a directory: a pure per-platform resolver plus an injected launcher.
//! Only `main.rs` installs the real one, so tests never spawn anything.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{App, Global};

/// The OS whose terminal conventions apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Linux and other Unix desktops.
    Linux,
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
}

impl Platform {
    /// The platform this build runs on.
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

/// A program to start, with its arguments and working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// The executable name.
    pub program: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// The directory it starts in.
    pub cwd: PathBuf,
}

const LINUX_TERMINALS: [&str; 4] = ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"];

/// The command that opens a terminal in `dir`, or `None` when no terminal is found.
/// `env` reads an environment variable and `exists` says whether a program is on the path.
pub fn resolve(
    platform: Platform,
    dir: &Path,
    env: &dyn Fn(&str) -> Option<String>,
    exists: &dyn Fn(&str) -> bool,
) -> Option<Launch> {
    let launch = |program: &str, args: Vec<String>| Launch {
        program: program.into(),
        args,
        cwd: dir.to_path_buf(),
    };
    let dir_arg = dir.to_string_lossy().into_owned();
    match platform {
        Platform::Linux => {
            // $TERMINAL may carry arguments ("kitty -1"); only its first word is looked up.
            let wanted = env("TERMINAL").unwrap_or_default();
            let mut words = wanted.split_whitespace();
            let custom = words.next().filter(|p| exists(p));
            match custom {
                Some(p) => Some(launch(p, words.map(String::from).collect())),
                None => LINUX_TERMINALS
                    .into_iter()
                    .find(|p| exists(p))
                    .map(|p| launch(p, Vec::new())),
            }
        }
        Platform::MacOs => Some(launch(
            "open",
            vec!["-a".into(), "Terminal".into(), dir_arg],
        )),
        Platform::Windows if exists("wt") => Some(launch("wt", vec!["-d".into(), dir_arg])),
        Platform::Windows => Some(launch(
            "cmd",
            ["/c", "start", "cmd", "/k"].map(String::from).to_vec(),
        )),
    }
}

/// Starts a program with arguments in a directory; an error is shown to the user.
pub type TerminalLauncher = Arc<dyn Fn(&str, &[String], &Path) -> Result<(), String>>;

type EnvLookup = dyn Fn(&str) -> Option<String>;

/// Everything `open_in` needs from the host.
#[derive(Clone)]
pub struct Setup {
    /// The platform's conventions.
    pub platform: Platform,
    /// Reads an environment variable.
    pub env: Arc<EnvLookup>,
    /// Whether a program is on the path.
    pub exists: Arc<dyn Fn(&str) -> bool>,
    /// Starts the resolved command.
    pub launch: TerminalLauncher,
}

impl Setup {
    /// The real host: the process environment, the `PATH` search and a spawned process.
    pub fn system() -> Setup {
        Setup {
            platform: Platform::current(),
            env: Arc::new(|k| std::env::var(k).ok()),
            exists: Arc::new(on_path),
            launch: Arc::new(|program, args, cwd| {
                let mut cmd = std::process::Command::new(program);
                cmd.args(args)
                    .current_dir(cwd)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                // Its own process group, so closing the app does not signal the terminal.
                #[cfg(unix)]
                std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
                let mut child = cmd.spawn().map_err(|e| e.to_string())?;
                // Reap the child when it exits so it never lingers as a zombie.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                Ok(())
            }),
        }
    }
}

struct Installed(Setup);

impl Global for Installed {}

/// Installs the host; without one `open_in` reports that no terminal is available.
pub fn install(setup: Setup, cx: &mut App) {
    cx.set_global(Installed(setup));
}

/// Opens a terminal in `dir`. `Err` holds the text for an error toast.
pub fn open_in(dir: &Path, cx: &App) -> Result<(), String> {
    let none = || "No terminal found".to_string();
    let setup = &cx.try_global::<Installed>().ok_or_else(none)?.0;
    let launch = resolve(
        setup.platform,
        dir,
        setup.env.as_ref(),
        setup.exists.as_ref(),
    )
    .ok_or_else(none)?;
    (setup.launch)(&launch.program, &launch.args, &launch.cwd)
        .map_err(|e| format!("Could not open a terminal: {e}"))
}

// Whether `program` is an existing file in a `PATH` directory.
fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path)
        .any(|d| d.join(program).is_file() || d.join(format!("{program}.exe")).is_file())
}
