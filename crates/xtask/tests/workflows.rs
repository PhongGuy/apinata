//! Structural checks on the GitHub workflows: what actionlint cannot know about this project.

use std::path::PathBuf;

use yaml_serde::Value;

fn workflow_path(name: &str) -> PathBuf {
    xtask::workspace_root().join(".github/workflows").join(name)
}

fn load(name: &str) -> Value {
    let text = std::fs::read_to_string(workflow_path(name)).unwrap();
    yaml_serde::from_str(&text).unwrap()
}

fn steps(workflow: &Value, job: &str) -> Vec<Value> {
    workflow["jobs"][job]["steps"]
        .as_sequence()
        .unwrap()
        .clone()
}

fn str_of(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn strs(v: &Value) -> Vec<&str> {
    v.as_sequence().unwrap().iter().map(str_of).collect()
}

/// The `(if, run)` pairs of a job's steps that call make, in order.
fn make_steps(workflow: &Value, job: &str) -> Vec<(String, String)> {
    steps(workflow, job)
        .iter()
        .filter(|s| str_of(&s["run"]).trim_start().starts_with("make"))
        .map(|s| {
            (
                str_of(&s["if"]).to_owned(),
                str_of(&s["run"]).trim().to_owned(),
            )
        })
        .collect()
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

fn workflow_files() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(workflow_path(""))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".yml"))
        .collect();
    names.sort();
    names
}

#[test]
fn every_action_is_pinned_to_a_commit_with_its_version_noted() {
    for name in workflow_files() {
        let text = std::fs::read_to_string(workflow_path(&name)).unwrap();
        for line in text
            .lines()
            .filter(|l| l.trim_start().trim_start_matches("- ").starts_with("uses:"))
        {
            let spec = line.split("uses:").nth(1).unwrap().trim();
            let (action, rest) = spec
                .split_once('@')
                .unwrap_or_else(|| panic!("{name}: unpinned {spec}"));
            let sha: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
            assert!(
                sha.len() == 40
                    && sha
                        .chars()
                        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                "{name}: {action} is pinned to '{sha}', not a commit SHA"
            );
            assert!(
                rest.contains(" # "),
                "{name}: {action} lacks a '# version' comment"
            );
        }
    }
}

#[test]
fn every_workflow_reads_only_by_default_and_never_keeps_checkout_credentials() {
    for name in workflow_files() {
        let wf = load(&name);
        assert_eq!(
            str_of(&wf["permissions"]["contents"]),
            "read",
            "{name}: top-level permissions"
        );
        for (job, body) in wf["jobs"].as_mapping().unwrap() {
            for step in body["steps"].as_sequence().unwrap() {
                if str_of(&step["uses"]).starts_with("actions/checkout@") {
                    assert_eq!(
                        step["with"]["persist-credentials"],
                        Value::Bool(false),
                        "{name}: {job:?} keeps checkout credentials"
                    );
                }
            }
        }
    }
}

#[test]
fn every_job_has_a_timeout() {
    for name in workflow_files() {
        let wf = load(&name);
        for (job, body) in wf["jobs"].as_mapping().unwrap() {
            let minutes = body["timeout-minutes"].as_u64();
            assert!(
                matches!(minutes, Some(1..=90)),
                "{name}: {job:?} has timeout-minutes {:?}, want 1 to 90",
                body["timeout-minutes"]
            );
        }
    }
}

#[test]
fn linux_builds_free_runner_disk_and_skip_debug_info() {
    // A GPUI debug build overflows the hosted runner's free disk, so Linux jobs that build
    // clear preinstalled toolchains first and CI compiles without debug info.
    let ci = load("ci.yml");
    assert_eq!(str_of(&ci["env"]["CARGO_PROFILE_DEV_DEBUG"]), "0");
    assert_eq!(str_of(&ci["env"]["CARGO_INCREMENTAL"]), "0");
    for name in workflow_files() {
        let wf = load(&name);
        for (job, body) in wf["jobs"].as_mapping().unwrap() {
            let job = str_of(job);
            if !str_of(&body["runs-on"]).starts_with("ubuntu") || make_steps(&wf, job).is_empty() {
                continue;
            }
            let names: Vec<String> = steps(&wf, job)
                .iter()
                .map(|s| str_of(&s["name"]).to_owned())
                .collect();
            let free = names.iter().position(|n| n == "Free disk space");
            let first_make = steps(&wf, job)
                .iter()
                .position(|s| str_of(&s["run"]).trim_start().starts_with("make"));
            assert!(
                matches!((free, first_make), (Some(f), Some(m)) if f < m),
                "{name}: {job} must free disk space before building"
            );
        }
    }
}

#[test]
fn ci_runs_the_make_gate_on_linux_for_master_and_pull_requests() {
    let ci = load("ci.yml");
    let on = &ci["on"];
    // Every change to master and every pull request runs the gate; dispatch reruns it by hand.
    assert_eq!(
        on["push"]["branches"],
        Value::Sequence(vec![Value::String("master".into())]),
        "pushes to master run the gate"
    );
    assert!(
        on.get("pull_request").is_some(),
        "pull requests run the gate"
    );
    assert!(on.get("workflow_dispatch").is_some());
    assert_eq!(ci["concurrency"]["cancel-in-progress"], Value::Bool(true));
    for (job, body) in ci["jobs"].as_mapping().unwrap() {
        assert_eq!(
            str_of(&body["runs-on"]),
            "ubuntu-24.04",
            "{job:?} runs off Linux"
        );
    }
    assert_eq!(
        make_steps(&ci, "gate"),
        pairs(&[("", "make check icons-check windows-check workflows-lint")])
    );
}

#[test]
fn release_runs_on_version_tags_and_on_demand_as_a_dry_run() {
    let rel = load("release.yml");
    assert_eq!(strs(&rel["on"]["push"]["tags"]), ["v*"]);
    let inputs = &rel["on"]["workflow_dispatch"]["inputs"];
    assert_eq!(str_of(&inputs["dry_run"]["type"]), "boolean");
    assert_eq!(inputs["dry_run"]["default"], Value::Bool(true));
    assert_eq!(
        str_of(&inputs["platforms"]["default"]),
        "linux,windows,macos"
    );
    let guard: String = steps(&rel, "guard")
        .iter()
        .map(|s| str_of(&s["run"]).to_owned())
        .collect();
    assert!(
        guard.contains(r#"scripts/release.sh guard "$GITHUB_REF_TYPE" "$GITHUB_REF_NAME" "$DRY_RUN" "$PLATFORMS""#),
        "{guard}"
    );
}

#[test]
fn release_builds_the_guard_matrix_with_make() {
    let rel = load("release.yml");
    let build = &rel["jobs"]["build"];
    assert_eq!(str_of(&build["needs"]), "guard");
    assert_eq!(
        str_of(&build["strategy"]["matrix"]),
        "${{ fromJSON(needs.guard.outputs.matrix) }}"
    );
    assert_eq!(
        str_of(&rel["jobs"]["guard"]["outputs"]["matrix"]),
        "${{ steps.guard.outputs.matrix }}"
    );
    assert_eq!(
        make_steps(&rel, "build"),
        pairs(&[
            ("runner.os == 'Linux'", "make package"),
            ("runner.os != 'Linux'", "make package-release")
        ])
    );
}

#[test]
fn only_the_draft_step_can_write_and_it_skips_dry_runs() {
    let rel = load("release.yml");
    for (job, body) in rel["jobs"].as_mapping().unwrap() {
        let write = str_of(&body["permissions"]["contents"]) == "write";
        assert_eq!(
            write,
            str_of(job) == "checksums",
            "{job:?} contents permission"
        );
    }
    let draft: Vec<Value> = steps(&rel, "checksums")
        .into_iter()
        .filter(|s| str_of(&s["run"]).contains("gh release create"))
        .collect();
    assert_eq!(draft.len(), 1);
    assert_eq!(
        str_of(&draft[0]["if"]),
        "github.ref_type == 'tag' && !inputs.dry_run"
    );
    assert!(str_of(&draft[0]["run"]).contains("--draft"));
}

#[test]
fn secrets_reach_only_the_packaging_step_env_and_are_documented() {
    let text = std::fs::read_to_string(workflow_path("release.yml")).unwrap();
    let docs = std::fs::read_to_string(xtask::workspace_root().join("RELEASING.md")).unwrap();
    let mut names = Vec::new();
    for part in text.split("secrets.").skip(1) {
        let name: String = part
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        assert!(
            docs.contains(&name),
            "secret {name} is not documented in RELEASING.md"
        );
        names.push(name);
    }
    assert_eq!(
        names,
        [
            "WINDOWS_CERTIFICATE",
            "WINDOWS_CERTIFICATE_PASSWORD",
            "CARGO_PACKAGER_SIGN_PRIVATE_KEY",
            "CARGO_PACKAGER_SIGN_PRIVATE_KEY_PASSWORD"
        ]
    );
    let rel = load("release.yml");
    for (job, body) in rel["jobs"].as_mapping().unwrap() {
        for step in body["steps"].as_sequence().unwrap() {
            assert!(
                !str_of(&step["run"]).contains("secrets."),
                "{job:?} puts a secret in a run script"
            );
            let env = yaml_serde::to_string(&step["env"]).unwrap();
            if env.contains("secrets.") {
                let run = str_of(&step["run"]);
                assert!(
                    run == "make package-release"
                        || run == "cargo packager signer sign update/latest.json",
                    "{job:?} passes secrets to another step"
                );
            }
        }
    }
}

#[test]
fn release_builds_see_the_update_urls_only_while_packaging() {
    let rel = load("release.yml");
    for step in steps(&rel, "build") {
        let packaging = str_of(&step["run"]).starts_with("make package");
        for var in ["APINATA_UPDATE_MANIFEST_URL", "APINATA_DOWNLOAD_PAGE_URL"] {
            let want = format!("${{{{ vars.{var} }}}}");
            let got = str_of(&step["env"][var]);
            assert_eq!(got == want, packaging, "{var} on {:?}", step["name"]);
        }
    }
    let ci = std::fs::read_to_string(workflow_path("ci.yml")).unwrap();
    assert!(
        !ci.contains("APINATA_UPDATE_MANIFEST_URL"),
        "CI builds stay unconfigured"
    );
}

#[test]
fn the_manifest_is_signed_after_the_sums_and_uploaded_last() {
    let rel = load("release.yml");
    let all = steps(&rel, "checksums");
    let at = |needle: &str| {
        all.iter()
            .position(|s| str_of(&s["run"]).contains(needle))
            .unwrap_or_else(|| panic!("no step runs {needle}"))
    };
    let (sums, manifest, sign, draft) = (
        at("release.sh sums"),
        at("release.sh manifest"),
        at("signer sign"),
        at("gh release create"),
    );
    assert!(sums < manifest && manifest < sign && sign < draft);
    // Only tag runs sign, so a dry run from a branch never yields a signed manifest; the
    // toolchain and cargo-packager steps serve only the signing and share the gate.
    let gate = "github.ref_type == 'tag' && vars.APINATA_UPDATE_BASE_URL != ''";
    let toolchain = all
        .iter()
        .position(|s| str_of(&s["uses"]).starts_with("dtolnay/rust-toolchain@"))
        .expect("a toolchain step");
    for i in [
        toolchain,
        at("cargo install --locked cargo-packager"),
        manifest,
        sign,
    ] {
        assert_eq!(str_of(&all[i]["if"]), gate, "step {i}");
    }
    let run = str_of(&all[draft]["run"]);
    let create = run.find("gh release create").unwrap();
    let json = run
        .find("update/latest.json\n")
        .expect("uploads latest.json");
    let sig = run
        .rfind("update/latest.json.sig")
        .expect("uploads the signature");
    assert!(
        create < json && json < sig,
        "artifacts, then latest.json, then its signature"
    );
}

#[test]
fn each_build_uploads_its_platform_directory_flat() {
    // A wildcard directory in the upload path roots the artifact above the platform
    // directory, so the merged download nests files where the sums, the manifest and
    // the draft upload, which all read dist flat, never see them.
    let rel = load("release.yml");
    let uploads: Vec<Value> = steps(&rel, "build")
        .into_iter()
        .filter(|s| str_of(&s["uses"]).starts_with("actions/upload-artifact@"))
        .collect();
    assert_eq!(uploads.len(), 1);
    assert_eq!(
        str_of(&uploads[0]["with"]["path"]),
        "target/dist/${{ runner.os == 'Linux' && 'linux' || runner.os == 'macOS' && 'macos' || 'windows' }}/"
    );
}

#[test]
fn the_draft_uses_generated_notes_from_full_history() {
    // The notes count commits since the previous tag, so the checkout needs every
    // commit and tag, and the draft takes the generated file instead of GitHub's.
    let rel = load("release.yml");
    let all = steps(&rel, "checksums");
    let checkout = all
        .iter()
        .find(|s| str_of(&s["uses"]).starts_with("actions/checkout@"))
        .expect("a checkout");
    assert_eq!(checkout["with"]["fetch-depth"], Value::Number(0.into()));
    let draft = all
        .iter()
        .map(|s| str_of(&s["run"]))
        .find(|r| r.contains("gh release create"))
        .unwrap();
    let notes = draft
        .find(r#"scripts/release.sh notes "$GITHUB_REF_NAME" "$GITHUB_REPOSITORY" dist release-notes.md"#)
        .expect("drafts the notes");
    let create = draft.find("gh release create").unwrap();
    assert!(notes < create, "{draft}");
    assert!(draft.contains("--notes-file release-notes.md"), "{draft}");
    assert!(!draft.contains("--generate-notes"), "{draft}");
}
