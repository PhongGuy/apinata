//! Runs the bash tests for scripts/package-linux.sh, which need GNU tar and coreutils.

#[cfg(target_os = "linux")]
#[test]
fn package_linux_functions() {
    let script = xtask::workspace_root().join("scripts/tests/package-linux-test.sh");
    let out = std::process::Command::new("bash")
        .arg(&script)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
