//! The team file's cross-process lock: another process holding it makes an update busy until
//! that process goes, and two processes updating at once keep every change.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use apinata_core::team::{Account, Binding, Role, ServerUrl, TeamError, TeamStore};
use tempfile::TempDir;

const FILE_VAR: &str = "APINATA_TEAM_LOCK_CHILD_FILE";
const DIR_VAR: &str = "APINATA_TEAM_LOCK_CHILD_DIR";
const TAG_VAR: &str = "APINATA_TEAM_LOCK_CHILD_TAG";

fn account() -> Account {
    let server = ServerUrl::parse("http://127.0.0.1:9").unwrap();
    Account::new(&server, "ada", "Ada", "t1", false)
}

fn binding(root: &str) -> Binding {
    Binding::new(PathBuf::from(root), &account(), "core", "api", Role::Write)
}

// Polls for `file` to appear, failing the test after 30 s.
fn wait_for(file: &Path, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !file.exists() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A child process killed when the guard drops, so a failed assertion never leaves it running.
struct Killed(Child);

impl Killed {
    // Waits up to 60 s for the child to exit, true when it succeeded.
    fn finish(&mut self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return status.success();
            }
            assert!(Instant::now() < deadline, "the child never finished");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// Re-runs this binary as the child test `name`, pointed at the team file and a signal dir.
fn spawn_child(name: &str, team_file: &Path, dir: &Path, tag: &str) -> Killed {
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            &format!("team_store_lock::{name}"),
            "--exact",
            "--test-threads=1",
        ])
        .env(FILE_VAR, team_file)
        .env(DIR_VAR, dir)
        .env(TAG_VAR, tag)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    Killed(child)
}

fn child_env() -> Option<(PathBuf, PathBuf, String)> {
    let file = std::env::var_os(FILE_VAR)?;
    let dir = std::env::var_os(DIR_VAR)?;
    let tag = std::env::var(TAG_VAR).unwrap_or_default();
    Some((file.into(), dir.into(), tag))
}

// The child half of the busy test: it holds the team file mid-update until told to stop or
// killed. Run on its own, it does nothing.
#[test]
fn child_holds_the_team_file() {
    let Some((file, dir, _)) = child_env() else {
        return;
    };
    TeamStore::update(&file, |_| {
        std::fs::write(dir.join("held"), "").unwrap();
        wait_for(&dir.join("release"), "never released");
    })
    .unwrap();
}

// The child half of the race: it says it is ready, and once told to go it binds 20 folders
// named by its tag, one update each. Run on its own, it does nothing.
#[test]
fn child_binds_twenty_folders() {
    let Some((file, dir, tag)) = child_env() else {
        return;
    };
    std::fs::write(dir.join(format!("ready-{tag}")), "").unwrap();
    wait_for(&dir.join("go"), "never told to go");
    for i in 0..20 {
        TeamStore::update(&file, |t| t.put_binding(binding(&format!("/{tag}/{i}")))).unwrap();
    }
}

#[test]
fn another_process_mid_update_makes_this_one_busy_until_it_is_killed() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("nested/team.json");
    let child = spawn_child("child_holds_the_team_file", &file, dir.path(), "");
    wait_for(&dir.path().join("held"), "the child never took the lock");

    let started = Instant::now();
    let busy = TeamStore::update(&file, |t| t.put_account(account()));
    let waited = started.elapsed();
    if let Err(e) = &busy {
        assert_eq!(
            e.to_string(),
            "Another Apinata window or command is changing the team file; try again."
        );
    }
    assert!(matches!(busy, Err(TeamError::Busy)), "{busy:?}");
    assert!(waited >= Duration::from_millis(1500), "waited {waited:?}");
    assert!(TeamStore::load(&file).unwrap().accounts().is_empty());

    drop(child);
    TeamStore::update(&file, |t| t.put_account(account())).unwrap();
    assert_eq!(TeamStore::load(&file).unwrap().accounts().len(), 1);
    assert!(
        file.with_file_name("team.json.lock").exists(),
        "the lock file stays"
    );
}

#[test]
fn two_processes_updating_at_once_keep_every_change() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("team.json");
    let mut children = [
        spawn_child("child_binds_twenty_folders", &file, dir.path(), "a"),
        spawn_child("child_binds_twenty_folders", &file, dir.path(), "b"),
    ];
    // Both wait at the start line, so their updates always overlap.
    for tag in ["a", "b"] {
        wait_for(
            &dir.path().join(format!("ready-{tag}")),
            "a child never got ready",
        );
    }
    std::fs::write(dir.path().join("go"), "").unwrap();
    for child in &mut children {
        assert!(child.finish(), "a child failed");
    }
    let bound = TeamStore::load(&file).unwrap().bindings().len();
    assert_eq!(bound, 40, "every update survives");
}
