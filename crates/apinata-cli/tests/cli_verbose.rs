//! `run --verbose` prints one redacted line per exchange to stderr and leaves stdout alone.

use crate::common;

use common::{ROOT, apinata, write_tree};
use std::process::Output;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn plain(name: &str, seq: u32, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    )
}

/// The report with every `*_ms` timing removed, since two runs never time alike.
fn untimed(stdout: &[u8]) -> serde_json::Value {
    fn strip(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(map) => {
                map.retain(|k, _| !k.ends_with("_ms"));
                map.values_mut().for_each(strip);
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut v = serde_json::from_slice(stdout).expect("stdout is only the report");
    strip(&mut v);
    v
}

fn ends_in_ms(line: &str) -> bool {
    line.rsplit(' ')
        .next()
        .and_then(|t| t.strip_suffix("ms"))
        .is_some_and(|n| n.parse::<u64>().is_ok())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verbose_writes_one_redacted_line_per_exchange_to_stderr() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - secret: true\n    name: key\n",
        ),
        (
            "a.yml",
            &plain("A", 1, &format!("{}/ok?key={{{{key}}}}", s.uri())),
        ),
        ("b.yml", &plain("B", 2, "http://127.0.0.1:1/refused")),
    ]);
    let run = |verbose: bool| -> Output {
        let mut cmd = apinata();
        cmd.arg("run").arg(dir.path()).args([
            "--env",
            "dev",
            "--env-var",
            "key=SUPERSECRET",
            "--format",
            "json",
        ]);
        if verbose {
            cmd.arg("--verbose");
        }
        cmd.output().expect("runs")
    };
    let quiet = run(false);
    let loud = run(true);
    assert_eq!(quiet.status.code(), loud.status.code());
    assert_eq!(untimed(&quiet.stdout), untimed(&loud.stdout));
    let stderr = String::from_utf8_lossy(&loud.stderr);
    assert!(!stderr.contains("SUPERSECRET"), "{stderr}");
    assert!(
        !String::from_utf8_lossy(&quiet.stderr)
            .lines()
            .any(|l| l.starts_with('['))
    );
    let lines: Vec<&str> = stderr.lines().filter(|l| l.starts_with('[')).collect();
    assert_eq!(lines.len(), 2, "{stderr}");
    assert!(
        lines[0].starts_with(&format!("[runner] GET {}/ok?key=[redacted] 200 ", s.uri())),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].starts_with("[runner] GET http://127.0.0.1:1/refused error: "),
        "{}",
        lines[1]
    );
    assert!(lines.iter().all(|l| ends_in_ms(l)), "{stderr}");
}

/// The `--verbose` lines of `cmd` run with a JSON report, after checking stderr never holds `secret`.
fn verbose_lines(cmd: &mut assert_cmd::Command, secret: &str) -> Vec<String> {
    let out = cmd
        .args(["--format", "json", "--verbose"])
        .output()
        .expect("runs");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!stderr.contains(secret), "{stderr}");
    stderr
        .lines()
        .filter(|l| l.starts_with('['))
        .map(str::to_owned)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verbose_hides_a_secret_declared_by_a_parent_environment() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/base.yml",
            "name: base\nvariables:\n  - secret: true\n    name: key\n",
        ),
        (
            "environments/dev.yml",
            "name: dev\nextends: base\nvariables:\n  - name: other\n    value: x\n",
        ),
        (
            "a.yml",
            &plain("A", 1, &format!("{}/ok?key={{{{key}}}}", s.uri())),
        ),
    ]);
    let mut cmd = apinata();
    cmd.arg("run")
        .arg(dir.path())
        .args(["--env", "dev", "--env-var", "key=PARENTSECRET"]);
    let lines = verbose_lines(&mut cmd, "PARENTSECRET");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("/ok?key=[redacted] 200 "), "{}", lines[0]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verbose_hides_a_global_secret_supplied_on_the_command_line() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let profile = write_tree(&[(
        "global-environments/shared.yml",
        "name: shared\nvariables:\n  - secret: true\n    name: gkey\n",
    )]);
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &plain("A", 1, &format!("{}/ok?key={{{{gkey}}}}", s.uri())),
        ),
    ]);
    let mut cmd = apinata();
    cmd.env("APINATA_PROFILE_DIR", profile.path())
        .arg("run")
        .arg(dir.path())
        .args([
            "--global-env",
            "shared",
            "--global-env-var",
            "gkey=GLOBALSECRET",
        ]);
    let lines = verbose_lines(&mut cmd, "GLOBALSECRET");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("/ok?key=[redacted] 200 "), "{}", lines[0]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verbose_hides_a_global_secret_supplied_through_env_var() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let profile = write_tree(&[(
        "global-environments/shared.yml",
        "name: shared\nvariables:\n  - secret: true\n    name: gkey\n",
    )]);
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &plain("A", 1, &format!("{}/ok?key={{{{gkey}}}}", s.uri())),
        ),
    ]);
    let mut cmd = apinata();
    cmd.env("APINATA_PROFILE_DIR", profile.path())
        .arg("run")
        .arg(dir.path())
        .args([
            "--global-env",
            "shared",
            "--env-var",
            "gkey=WRONGFLAGSECRET",
        ]);
    let lines = verbose_lines(&mut cmd, "WRONGFLAGSECRET");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("/ok?key=[redacted] 200 "), "{}", lines[0]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verbose_keeps_an_exchange_with_a_capped_url_on_one_line() {
    let s = MockServer::start().await;
    Mock::given(path("/long"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    // A URL past the 8 KiB log cap is stored with a multi-line truncation marker.
    let url = format!("{}/long?pad={}", s.uri(), "a".repeat(9 * 1024));
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &plain("A", 1, &url)),
    ]);
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--format", "json", "--verbose"])
        .output()
        .expect("runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "{stderr}");
    assert!(
        lines[0].starts_with(&format!("[runner] GET {}/long?pad=aaa", s.uri())),
        "{}",
        lines[0]
    );
    assert!(lines[0].contains("[truncated: kept "), "{}", lines[0]);
    assert!(lines[0].contains(" 200 "), "{}", lines[0]);
    assert!(ends_in_ms(lines[0]), "{}", lines[0]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verbose_names_the_address_each_exchange_reached() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &plain("A", 1, &format!("{}/ok", s.uri()))),
    ]);
    let lines = verbose_lines(apinata().arg("run").arg(dir.path()), "no-secret");
    assert_eq!(lines.len(), 1, "{lines:?}");
    let want = format!("{}/ok 200 {} ", s.uri(), s.address());
    assert!(lines[0].contains(&want), "{}", lines[0]);
    assert!(ends_in_ms(&lines[0]), "{}", lines[0]);
}
