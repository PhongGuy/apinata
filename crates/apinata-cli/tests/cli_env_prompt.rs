//! `env set --secret` on a terminal: the hidden prompt, driven through a pseudo-terminal that is
//! the child's controlling terminal. The keychain is off, so the set fails by design; what is
//! checked is that the prompt appears, the typed value is read without echo, and nothing is saved.

#![cfg(target_os = "linux")]

use crate::common;

use common::{ROOT, apinata_std, write_tree};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const LIMIT: Duration = Duration::from_secs(20);
const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: example.test\n";

/// A pty pair: the master end for the test and the slave end for the child.
fn open_pty() -> (std::fs::File, std::fs::File) {
    // SAFETY: plain libc calls on a descriptor this function owns; every result is checked.
    unsafe {
        let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        assert!(master >= 0, "posix_openpt failed");
        let master = OwnedFd::from_raw_fd(master);
        assert_eq!(libc::grantpt(master.as_raw_fd()), 0);
        assert_eq!(libc::unlockpt(master.as_raw_fd()), 0);
        let mut name = [0 as libc::c_char; 128];
        assert_eq!(
            libc::ptsname_r(master.as_raw_fd(), name.as_mut_ptr(), name.len()),
            0
        );
        let path = std::ffi::CStr::from_ptr(name.as_ptr()).to_str().unwrap();
        let slave = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        (std::fs::File::from(master), slave)
    }
}

/// Whether the terminal behind the master still echoes typed characters.
fn echo_is_on(master: &std::fs::File) -> bool {
    // SAFETY: tcgetattr only fills the zeroed struct for a descriptor that stays open.
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        assert_eq!(libc::tcgetattr(master.as_raw_fd(), &mut t), 0);
        t.c_lflag & libc::ECHO != 0
    }
}

#[test]
fn env_set_secret_prompts_on_a_terminal_without_echo() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)]);
    let (mut master, slave) = open_pty();
    let mut cmd = apinata_std();
    cmd.current_dir(dir.path())
        .args(["env", "set", "--secret", "--no-keychain", "dev", "apikey"])
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave.try_clone().unwrap()));
    // SAFETY: setsid and TIOCSCTTY run between fork and exec and touch only the child's own
    // session, giving it the pty as the controlling terminal that `/dev/tty` names.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().unwrap();
    drop(cmd);
    // Only the child holds the slave now, so the master reads end when it exits.
    drop(slave);

    // Everything the child prints, collected on a thread so every wait below has a timeout.
    let (tx, rx) = mpsc::channel();
    let mut reader = master.try_clone().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 256];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut seen = String::new();
    let deadline = Instant::now() + LIMIT;
    let mut pump = |seen: &mut String, until: &dyn Fn(&str) -> bool| {
        while !until(seen) {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left.max(Duration::from_millis(1))) {
                Ok(bytes) => seen.push_str(&String::from_utf8_lossy(&bytes)),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("timed out; output so far: {seen:?}");
                }
            }
        }
    };

    pump(&mut seen, &|s| s.contains("Secret value:"));
    // The prompt can print just before echo goes off; type only once it is off.
    let echo_deadline = Instant::now() + LIMIT;
    while echo_is_on(&master) {
        if Instant::now() >= echo_deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("echo never went off");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    master.write_all(b"hunter2\n").unwrap();
    pump(&mut seen, &|_| false);

    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(3), "{seen:?}");
    assert!(seen.contains("keychain"), "{seen:?}");
    assert!(!seen.contains("hunter2"), "the value was echoed: {seen:?}");
    assert!(!seen.contains("no secret value"), "{seen:?}");
    let file = std::fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap();
    assert_eq!(file, DEV);
}
