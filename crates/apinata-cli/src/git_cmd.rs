//! `apinata git status`, `diff`, `clone`, `branches`, `switch`, `commit`, `pull` and `push`: print
//! what core reports about a collection's changes and branches, clone repositories with progress,
//! commit, and fast-forward or push the current branch. Nothing here runs git itself.

use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use apinata_core::{
    CancelFlag, CloneError, CloneProgress, CloneRequest, FetchError, FileDiff, FileStatus,
    GitOptions, GitState, LineKind, RemoteBranch, RepoStatus, SwitchError, SyncError, Upstream,
};
use serde::Serialize;

use crate::{EXIT_USAGE, GitAction, GitArgs, collection_arg, run, trust};

/// Run `args`: exit 0 when printed or when git or a repository is missing (a note on stderr),
/// 1 when git fails or the file is refused, 3 when the collection cannot be found.
pub fn run(args: &GitArgs) -> ExitCode {
    match &args.action {
        GitAction::Status {
            path,
            collection,
            json,
        } => status(path.as_deref(), collection.as_deref(), *json),
        GitAction::Diff { file, json } => diff(file, *json),
        GitAction::Clone { url, dir } => clone(url, dir.as_deref()),
        GitAction::Branches {
            path,
            collection,
            fetch,
            json,
        } => branches(path.as_deref(), collection.as_deref(), *fetch, *json),
        GitAction::Switch { branch, collection } => switch(branch, collection.as_deref()),
        GitAction::Commit {
            message,
            all,
            paths,
            collection,
            json,
        } => commit(message, *all, paths, collection.as_deref(), *json),
        GitAction::Pull { collection, json } => pull(collection.as_deref(), *json),
        GitAction::Push {
            collection,
            set_upstream,
            json,
        } => push(collection.as_deref(), set_upstream.as_deref(), *json),
    }
}

/// The root of the collection named by the positional and `--collection` (the working directory
/// when neither is given), with `form` in the hint when none is found.
fn named_root(
    path: Option<&Path>,
    flag: Option<&Path>,
    form: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let named = collection_arg::pick(path, flag)?;
    collection_arg::with_hint(trust::collection_root(named.as_deref()), form)
}

fn status(path: Option<&Path>, flag: Option<&Path>, json: bool) -> ExitCode {
    let form = collection_arg::optional_hint("apinata git status [COLLECTION]");
    let root = match named_root(path, flag, &form) {
        Ok(root) => root,
        Err(e) => return usage(&e),
    };
    let state = match apinata_core::collection_status(&root) {
        Ok(state) => state,
        Err(e) => return failed(&e),
    };
    if json {
        return print_json(&state);
    }
    match &state {
        GitState::Repo(repo) => out!("{}", status_text(repo)),
        other => return quiet(other, &root),
    }
    ExitCode::SUCCESS
}

fn diff(file: &Path, json: bool) -> ExitCode {
    let cwd = match std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .context("cannot read the working directory")
    {
        Ok(cwd) => cwd,
        Err(e) => return usage(&e),
    };
    let root = match run::find_root(&cwd) {
        Ok(root) => root.to_path_buf(),
        Err(e) => return usage(&e),
    };
    // The file is placed lexically, so `..` or an absolute path elsewhere is refused before
    // git runs; core repeats its own check on the relative path.
    let Some(rel) = inside(&root, &cwd.join(file)) else {
        eprintln!(
            "error: {} is outside the collection {}",
            file.display(),
            root.display()
        );
        return ExitCode::from(EXIT_USAGE);
    };
    if rel.as_os_str().is_empty() {
        eprintln!(
            "error: {} is the collection, not a request file",
            file.display()
        );
        return ExitCode::from(EXIT_USAGE);
    }
    // Core reports a missing git or repository only as an error here, so ask status first.
    match apinata_core::collection_status(&root) {
        Ok(GitState::Repo(_)) => {}
        Ok(other) => return quiet(&other, &root),
        Err(e) => return failed(&e),
    }
    match apinata_core::file_diff(&root, &rel) {
        Ok(diff) if json => print_json(&diff),
        Ok(diff) => {
            out!("{}", diff_text(&diff));
            ExitCode::SUCCESS
        }
        Err(e) => failed(&e),
    }
}

/// Run `work` on a blocking thread with a [`CancelFlag`] that SIGINT sets. Git sits in its own
/// process group and never sees the terminal's Ctrl+C, so core must be told to stop it.
fn with_interrupt<T: Send + 'static>(
    work: impl FnOnce(&CancelFlag) -> T + Send + 'static,
) -> Result<T, std::io::Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let cancel = CancelFlag::new();
        // The handler must exist before the work starts, or an early Ctrl+C kills the process.
        #[cfg(unix)]
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let flag = cancel.clone();
        let watcher = tokio::spawn(async move {
            #[cfg(unix)]
            interrupt.recv().await;
            #[cfg(not(unix))]
            let _ = tokio::signal::ctrl_c().await;
            flag.cancel();
        });
        let done = tokio::task::spawn_blocking(move || work(&cancel)).await;
        watcher.abort();
        done.map_err(std::io::Error::other)
    })
}

/// Exit code of a run ended by Ctrl+C.
const EXIT_CANCELLED: u8 = 130;

fn clone(url: &str, dir: Option<&Path>) -> ExitCode {
    let (parent, name) = match clone_target(url, dir) {
        Ok(target) => target,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    eprintln!("Cloning {}", apinata_core::redact_url(url));
    let req = CloneRequest::new(url, parent, name);
    let result = with_interrupt(move |cancel| {
        // Lines already shown live are not repeated in the error.
        let mut shown = std::collections::HashSet::new();
        let mut progress = |p: CloneProgress| match p.percent {
            Some(percent) => eprintln!("{} {percent}%", p.phase),
            None => {
                eprintln!("{}", p.phase);
                shown.insert(p.phase);
            }
        };
        let result = apinata_core::clone_repo(&req, cancel, &GitOptions::new(), &mut progress);
        (result, shown)
    });
    let (result, shown) = match result {
        Ok(done) => done,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    match result {
        Ok(path) => {
            let path = std::fs::canonicalize(&path).unwrap_or(path);
            outln!("Cloned into {}", path.display());
            if !path.join("opencollection.yml").is_file() {
                eprintln!(
                    "note: {} has no opencollection.yml at its root; the clone was kept.",
                    path.display()
                );
            }
            ExitCode::SUCCESS
        }
        Err(CloneError::Cancelled) => {
            eprintln!("cancelled");
            ExitCode::from(EXIT_CANCELLED)
        }
        Err(
            e @ (CloneError::InvalidUrl { .. }
            | CloneError::InvalidName { .. }
            | CloneError::Exists(_)),
        ) => {
            eprintln!("error: {e}");
            ExitCode::from(EXIT_USAGE)
        }
        Err(CloneError::Failed { stderr }) => {
            let unseen: Vec<&str> = stderr
                .lines()
                .filter(|l| {
                    let l = l.trim();
                    !l.contains('%') && !shown.contains(l.strip_prefix("remote: ").unwrap_or(l))
                })
                .collect();
            if unseen.is_empty() {
                eprintln!("error: git clone failed");
            } else {
                eprintln!("error: {}", unseen.join("\n"));
            }
            auth_hint(&stderr);
            ExitCode::from(EXIT_USAGE)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// Git texts that mean the credentials or the host key were refused.
const AUTH_MARKERS: [&str; 5] = [
    "Authentication failed",
    "could not read Username",
    "terminal prompts disabled",
    "Permission denied",
    "Host key verification failed",
];

/// Print the credential hint when git's `stderr` shows a refused login or host key.
fn auth_hint(stderr: &str) {
    if AUTH_MARKERS.iter().any(|m| stderr.contains(m)) {
        eprintln!(
            "hint: set up a credential helper or ssh-agent, then retry; to accept a new host key, connect once with ssh."
        );
    }
}

/// The parent folder and the new folder's name for `clone`, or the usage message.
fn clone_target(url: &str, dir: Option<&Path>) -> Result<(PathBuf, String), String> {
    let Some(dir) = dir else {
        // Validate first so a bad URL gets core's message rather than a naming one.
        apinata_core::validate_clone_url(url).map_err(|e| e.to_string())?;
        let name = apinata_core::default_folder_name(url)
            .ok_or_else(|| "cannot name a folder after the URL; pass a folder".to_owned())?;
        return Ok((PathBuf::from("."), name));
    };
    let bad = || format!("{} is not a folder name", dir.display());
    let name = dir.file_name().and_then(|n| n.to_str()).ok_or_else(bad)?;
    let parent = match dir.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    Ok((parent, name.to_owned()))
}

fn branches(path: Option<&Path>, flag: Option<&Path>, fetch: bool, json: bool) -> ExitCode {
    let form = collection_arg::optional_hint("apinata git branches [COLLECTION]");
    let root = match named_root(path, flag, &form) {
        Ok(root) => root,
        Err(e) => return usage(&e),
    };
    if fetch {
        let work_root = root.clone();
        let fetched = with_interrupt(move |cancel| {
            let opts = GitOptions::new();
            for remote in apinata_core::remotes(&work_root, &opts).map_err(|e| e.to_string())? {
                match apinata_core::fetch(&work_root, &remote, cancel, &opts) {
                    Ok(()) => {}
                    Err(FetchError::Cancelled) => return Ok(false),
                    Err(FetchError::Failed { stderr }) => return Err(stderr.trim_end().to_owned()),
                    Err(e) => return Err(e.to_string()),
                }
            }
            Ok(true)
        });
        match fetched {
            Ok(Ok(true)) => {}
            Ok(Ok(false)) => {
                eprintln!("cancelled");
                return ExitCode::from(EXIT_CANCELLED);
            }
            Ok(Err(message)) => {
                eprintln!("error: {message}");
                return ExitCode::from(EXIT_USAGE);
            }
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    }
    let list = match apinata_core::remote_branches(&root, &GitOptions::new()) {
        Ok(list) => list,
        Err(e) => return failed(&e),
    };
    if json {
        return print_json(&list);
    }
    for b in &list {
        match &b.local {
            Some(local) => outln!("{}/{} (tracked by {local})", b.remote, b.name),
            None => outln!("{}/{}", b.remote, b.name),
        }
    }
    ExitCode::SUCCESS
}

fn switch(wanted: &str, collection: Option<&Path>) -> ExitCode {
    let root = match collection_arg::with_hint(
        trust::collection_root(collection),
        "pass the collection with --collection <DIR>",
    ) {
        Ok(root) => root,
        Err(e) => return usage(&e),
    };
    let opts = GitOptions::new();
    let list = match apinata_core::remote_branches(&root, &opts) {
        Ok(list) => list,
        Err(e) => return failed(&e),
    };
    // The exact `<remote>/<name>` spelling wins; a bare name must match exactly one remote.
    let exact: Vec<&RemoteBranch> = list
        .iter()
        .filter(|b| format!("{}/{}", b.remote, b.name) == wanted)
        .collect();
    let by_name: Vec<&RemoteBranch> = list.iter().filter(|b| b.name == wanted).collect();
    let branch = match (exact.as_slice(), by_name.as_slice()) {
        ([one], _) | ([], [one]) => *one,
        ([], []) => {
            eprintln!("error: no remote branch named {wanted}; try apinata git branches --fetch");
            return ExitCode::from(EXIT_USAGE);
        }
        _ => {
            eprintln!("error: {wanted} is ambiguous; use <remote>/<name>");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    match apinata_core::switch_to_remote(&root, branch, &opts) {
        Ok(switched) => {
            let shown = &switched.branch;
            outln!("Switched to {shown}");
            let picked = format!("{}/{}", branch.remote, branch.name);
            match &switched.upstream {
                Upstream::Set => eprintln!("note: {shown} now tracks {picked}."),
                Upstream::Other { upstream } => eprintln!(
                    "note: {shown} tracks {upstream}, not {picked}; its upstream was left as is."
                ),
                _ => {}
            }
            if !root.join("opencollection.yml").is_file() {
                eprintln!("note: {shown} has no opencollection.yml at the collection root.");
            }
            ExitCode::SUCCESS
        }
        Err(SwitchError::Dirty) => {
            eprintln!("error: Commit or discard your changes first");
            ExitCode::from(EXIT_USAGE)
        }
        Err(SwitchError::Failed { stderr }) => {
            eprintln!("error: {}", stderr.trim_end());
            ExitCode::from(EXIT_USAGE)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// Cut a commit id to the 7 characters shown to people.
fn short(id: &str) -> &str {
    id.get(..7).unwrap_or(id)
}

/// Which sync command ran, for the hints and notes that differ between them.
#[derive(Clone, Copy, PartialEq)]
enum Sync {
    Pull,
    Push,
}

/// The note for a push whose outcome Ctrl+C left unknown.
const MAY_HAVE_PUSHED: &str = "the push may already have reached the remote";

/// Print a finished sync result, or its error with the matching exit code. The work reports
/// beside its result whether Ctrl+C arrived; `moved` says whether a pull changed the branch.
fn finish<T: Serialize>(
    result: Result<(Result<T, SyncError>, bool), std::io::Error>,
    json: bool,
    kind: Sync,
    moved: impl FnOnce(&T) -> bool,
    show: impl FnOnce(&T),
) -> ExitCode {
    match result {
        Ok((Ok(done), cancelled)) => {
            let late = cancelled && kind == Sync::Pull && moved(&done);
            let code = if json {
                print_json(&done)
            } else {
                show(&done);
                ExitCode::SUCCESS
            };
            // Core finishes a fast-forward it has started rather than leave the files half moved.
            if late {
                eprintln!("note: Ctrl+C came after the fast-forward had started; it finished");
            }
            code
        }
        Ok((Err(SyncError::Cancelled), _)) => {
            match kind {
                Sync::Pull => eprintln!("cancelled"),
                Sync::Push => eprintln!("cancelled; {MAY_HAVE_PUSHED}"),
            }
            ExitCode::from(EXIT_CANCELLED)
        }
        Ok((Err(e), cancelled)) => {
            eprintln!("error: {e}");
            explain(&e, kind, cancelled);
            ExitCode::from(EXIT_USAGE)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// The lines after a sync error that say what happened remotely or what to run next.
fn explain(error: &SyncError, kind: Sync, cancelled: bool) {
    match error {
        SyncError::Diverged { .. } => {
            eprintln!("hint: merge or rebase in a terminal, then run apinata git push");
        }
        SyncError::Rejected { reason, stderr } => {
            for line in stderr.lines().filter(|l| l.starts_with("remote:")) {
                eprintln!("{line}");
            }
            if reason == "non-fast-forward" || reason == "fetch first" {
                eprintln!("hint: run apinata git pull first, then push again");
            }
        }
        SyncError::NoUpstream { branch, gone } if kind == Sync::Push => match gone {
            None => {
                eprintln!("hint: run apinata git push --set-upstream <REMOTE> to publish {branch}")
            }
            Some(u) => {
                eprintln!("hint: run apinata git push --set-upstream <REMOTE> to re-create {u}")
            }
        },
        SyncError::Partial { upstream, .. } => eprintln!(
            "note: the files hold {upstream}, but the branch did not move; run git reset --merge HEAD to restore a clean tree, then apinata git pull again"
        ),
        SyncError::Failed { stderr } => auth_hint(stderr),
        SyncError::Busy { .. } if cancelled && kind == Sync::Push => {
            eprintln!("note: {MAY_HAVE_PUSHED}");
        }
        _ => {}
    }
}

fn sync_root(collection: Option<&Path>) -> Result<PathBuf, anyhow::Error> {
    collection_arg::with_hint(
        trust::collection_root(collection),
        "pass the collection with --collection <DIR>",
    )
}

fn pull(collection: Option<&Path>, json: bool) -> ExitCode {
    let root = match sync_root(collection) {
        Ok(root) => root,
        Err(e) => return usage(&e),
    };
    let result = with_interrupt(move |cancel| {
        let done = apinata_core::pull(&root, cancel, &GitOptions::new());
        (done, cancel.is_cancelled())
    });
    finish(
        result,
        json,
        Sync::Pull,
        |p| p.from != p.to,
        |p| {
            let (b, u) = (&p.branch, &p.upstream);
            if p.from == p.to {
                outln!("{b} is up to date with {u}");
            } else {
                outln!(
                    "Fast-forwarded {b} to {u} ({}..{})",
                    short(&p.from),
                    short(&p.to)
                );
            }
            if p.from == p.to && p.ahead > 0 {
                let n = p.ahead;
                let noun = if n > 1 { "commits" } else { "commit" };
                eprintln!("note: {b} is {n} {noun} ahead of {u}");
            }
        },
    )
}

fn push(collection: Option<&Path>, set_upstream: Option<&str>, json: bool) -> ExitCode {
    let root = match sync_root(collection) {
        Ok(root) => root,
        Err(e) => return usage(&e),
    };
    let remote = set_upstream.map(str::to_owned);
    let result = with_interrupt(move |cancel| {
        let done = apinata_core::push(&root, remote.as_deref(), cancel, &GitOptions::new());
        (done, cancel.is_cancelled())
    });
    finish(
        result,
        json,
        Sync::Push,
        |_| false,
        |p| {
            let (b, u) = (&p.branch, &p.upstream);
            let mut line = match &p.from {
                None => format!("Pushed {b} to {u} (new branch)"),
                Some(from) if *from == p.to => format!("{u} is up to date"),
                Some(from) => format!("Pushed {b} to {u} ({}..{})", short(from), short(&p.to)),
            };
            if p.upstream_set {
                line.push_str(&format!("; {b} now tracks {u}"));
            }
            outln!("{line}");
        },
    )
}

fn commit(
    message: &str,
    all: bool,
    paths: &[PathBuf],
    collection: Option<&Path>,
    json: bool,
) -> ExitCode {
    if message.trim().is_empty() {
        eprintln!("error: the commit message is empty");
        return ExitCode::from(EXIT_USAGE);
    }
    let root = match collection_arg::with_hint(
        trust::collection_root(collection),
        "pass the collection with --collection <DIR>",
    ) {
        Ok(root) => root,
        Err(e) => return usage(&e),
    };
    let cwd = match std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .context("cannot read the working directory")
    {
        Ok(cwd) => cwd,
        Err(e) => return usage(&e),
    };
    // Paths are placed inside the collection lexically, as `diff` does, so `..` or a path
    // elsewhere is refused before git runs.
    let mut specs = Vec::new();
    for path in paths {
        let Some(rel) = inside(&root, &cwd.join(path)) else {
            eprintln!(
                "error: {} is outside the collection {}",
                path.display(),
                root.display()
            );
            return ExitCode::from(EXIT_USAGE);
        };
        specs.push(if rel.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            rel
        });
    }
    if all {
        specs.push(PathBuf::from("."));
    }
    let options = GitOptions::new();
    match apinata_core::collection_status_with(&root, &options) {
        Ok(GitState::Repo(_)) => {}
        Ok(GitState::Unavailable) => {
            eprintln!("error: git was not found on PATH, so nothing can be committed.");
            return ExitCode::from(EXIT_USAGE);
        }
        Ok(_) => {
            eprintln!("error: {} is not in a git repository.", root.display());
            return ExitCode::from(EXIT_USAGE);
        }
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(EXIT_USAGE);
        }
    }
    if let Err(e) = apinata_core::stage(&root, &specs, &options) {
        eprintln!("error: {e}");
        return ExitCode::from(EXIT_USAGE);
    }
    match apinata_core::commit(&root, message, &options) {
        Ok(done) if json => print_json(&done),
        Ok(done) => {
            outln!("{} {}", done.short_id, done.subject);
            ExitCode::SUCCESS
        }
        Err(apinata_core::CommitError::EmptyMessage) => {
            eprintln!("error: the commit message is empty");
            ExitCode::from(EXIT_USAGE)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// `path` relative to `root` when what remains is only plain names, rebuilt from its components
/// so doubled slashes and `.` drop out. Empty when `path` is the root itself.
fn inside(root: &Path, path: &Path) -> Option<PathBuf> {
    let rel = path.strip_prefix(root).ok()?;
    let plain = rel.components().all(|c| matches!(c, Component::Normal(_)));
    plain.then(|| rel.components().collect())
}

/// The note for a state with nothing to show, on stderr with exit 0.
fn quiet(state: &GitState, root: &Path) -> ExitCode {
    match state {
        GitState::Unavailable => {
            eprintln!("git was not found on PATH, so changes cannot be shown.");
        }
        _ => eprintln!("{} is not in a git repository.", root.display()),
    }
    ExitCode::SUCCESS
}

fn usage(error: &anyhow::Error) -> ExitCode {
    eprintln!("error: {error:#}");
    ExitCode::from(EXIT_USAGE)
}

fn failed(error: &apinata_core::GitError) -> ExitCode {
    eprintln!("error: {error}");
    ExitCode::from(EXIT_USAGE)
}

fn print_json(value: &impl Serialize) -> ExitCode {
    match serde_json::to_string_pretty(value) {
        Ok(json) => {
            outln!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// The serde name of a unit-like value or the `kind` tag of a status: the word printed for it.
fn word(value: &impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(word)) => word,
        Ok(serde_json::Value::Object(map)) => map
            .get("kind")
            .and_then(|kind| kind.as_str())
            .unwrap_or_default()
            .to_owned(),
        _ => String::new(),
    }
}

/// One line per changed file, the truncation note, then where `HEAD` stands.
fn status_text(repo: &RepoStatus) -> String {
    let mut text = String::new();
    for (path, status) in &repo.files {
        let mut line = format!("{:<10} {}", word(status), path.display());
        if let FileStatus::Renamed { from } = status {
            line.push_str(&format!(" (from {})", from.display()));
        }
        text.push_str(&line);
        text.push('\n');
    }
    if repo.truncated {
        text.push_str(&format!(
            "showing the first {} changed files\n",
            apinata_core::GIT_FILE_CAP
        ));
    }
    let place = match (&repo.branch, &repo.head) {
        (Some(branch), Some(head)) => format!("on {branch} at {head}"),
        (None, Some(head)) => format!("detached at {head}"),
        (Some(branch), None) => format!("on {branch}, no commits yet"),
        (None, None) => "no commits yet".to_owned(),
    };
    text.push_str(&place);
    text.push('\n');
    text
}

/// The changed sections, then a unified diff with `a/` and `b/` headers and counted hunks.
fn diff_text(diff: &FileDiff) -> String {
    let mut text = String::new();
    if !diff.sections.is_empty() {
        let names: Vec<String> = diff.sections.iter().map(word).collect();
        text.push_str(&format!("sections: {}\n", names.join(", ")));
    }
    if diff.binary {
        text.push_str("binary file changed\n");
        return text;
    }
    if diff.hunks.is_empty() {
        text.push_str("no content change\n");
        return text;
    }
    let old_path = match &diff.status {
        Some(FileStatus::Renamed { from }) => from,
        _ => &diff.path,
    };
    let side = |present: bool, prefix: &str, path: &Path| {
        if present {
            format!("{prefix}/{}", path.display())
        } else {
            "/dev/null".to_owned()
        }
    };
    text.push_str(&format!(
        "--- {}\n",
        side(diff.old.is_some(), "a", old_path)
    ));
    text.push_str(&format!(
        "+++ {}\n",
        side(diff.new.is_some(), "b", &diff.path)
    ));
    for hunk in &diff.hunks {
        let count = |kind| {
            hunk.lines
                .iter()
                .filter(|l| l.kind == LineKind::Context || l.kind == kind)
                .count()
        };
        text.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            hunk.old_start,
            count(LineKind::Removed),
            hunk.new_start,
            count(LineKind::Added)
        ));
        for line in &hunk.lines {
            let mark = match line.kind {
                LineKind::Added => '+',
                LineKind::Removed => '-',
                _ => ' ',
            };
            text.push(mark);
            text.push_str(&line.text);
            text.push('\n');
            if line.no_newline {
                text.push_str("\\ No newline at end of file\n");
            }
        }
    }
    text
}
