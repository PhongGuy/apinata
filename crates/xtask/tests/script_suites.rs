//! Runs the bash suites under scripts/tests, which need GNU make, tar and coreutils.
#![cfg(target_os = "linux")]

fn run_suite(name: &str) {
    let script = xtask::workspace_root().join("scripts/tests").join(name);
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

#[test]
fn makefile_targets() {
    run_suite("makefile-test.sh");
}

#[test]
fn release_script() {
    run_suite("release-test.sh");
}

#[test]
fn actionlint_script() {
    run_suite("actionlint-test.sh");
}

#[test]
fn install_script() {
    run_suite("install-test.sh");
}

#[test]
fn demo_script() {
    run_suite("demo-test.sh");
}

#[test]
fn fonts_script() {
    run_suite("fetch-fonts-test.sh");
}

#[test]
fn public_suffix_list_script() {
    run_suite("update-public-suffix-list-test.sh");
}
