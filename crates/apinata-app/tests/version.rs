use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// `--version` must answer before GPUI starts, so it works with no display at all.
#[test]
fn version_flag_answers_without_a_display() {
    for flag in ["--version", "-V"] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_apinata-app"))
            .arg(flag)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("apinata-app starts");
        // Without the flag handled, the app would try to open a window; never hang on that.
        let deadline = Instant::now() + Duration::from_secs(30);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() > deadline {
                child.kill().unwrap();
                panic!("{flag}: apinata-app did not exit within 30 s");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{flag}: {out:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("apinata-app {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}
