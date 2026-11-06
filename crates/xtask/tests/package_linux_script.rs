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

/// Under `pipefail`, `cmd | grep -q` fails whenever grep exits on its first match while
/// `cmd` still writes, since `cmd` then dies of SIGPIPE. Scripts must grep captured text.
#[test]
fn pipefail_scripts_never_pipe_into_a_quiet_grep() {
    let mut offenders = Vec::new();
    let mut dirs = vec![xtask::workspace_root().join("scripts")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "sh") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            if !text.contains("pipefail") {
                continue;
            }
            for (n, line) in text.lines().enumerate() {
                let piped_quiet = line.split('|').skip(1).any(|stage| {
                    let mut words = stage.split_whitespace();
                    words.next() == Some("grep")
                        && words.any(|w| {
                            w == "--quiet"
                                || (w.starts_with('-') && !w.starts_with("--") && w.contains('q'))
                        })
                });
                if piped_quiet {
                    offenders.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(offenders.is_empty(), "{}", offenders.join("\n"));
}
