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
        if str_of(job) == "macos-clippy" {
            continue;
        }
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

/// The gate on the jobs that publish: tag runs that are not dry runs.
const PUBLISH_GATE: &str = "github.ref_type == 'tag' && !inputs.dry_run";

#[test]
fn only_the_draft_job_can_write_and_it_skips_dry_runs() {
    let rel = load("release.yml");
    for (job, body) in rel["jobs"].as_mapping().unwrap() {
        let write = str_of(&body["permissions"]["contents"]) == "write";
        assert_eq!(write, str_of(job) == "draft", "{job:?} contents permission");
    }
    let draft = &rel["jobs"]["draft"];
    assert_eq!(str_of(&draft["if"]), PUBLISH_GATE);
    let create: Vec<Value> = steps(&rel, "draft")
        .into_iter()
        .filter(|s| str_of(&s["run"]).contains("gh release create"))
        .collect();
    assert_eq!(create.len(), 1);
    assert!(str_of(&create[0]["run"]).contains("--draft"));
}

/// The name of a job's deployment environment, written either as a string or a mapping.
fn environment(job: &Value) -> &str {
    let env = &job["environment"];
    env.as_str().unwrap_or_else(|| str_of(&env["name"]))
}

#[test]
fn only_the_signing_job_binds_the_release_environment() {
    let rel = load("release.yml");
    for (job, body) in rel["jobs"].as_mapping().unwrap() {
        let release = environment(body) == "release";
        assert_eq!(release, str_of(job) == "sign", "{job:?} environment");
    }
    // The environment admits only version tags, so a dry run never asks for it.
    assert_eq!(str_of(&rel["jobs"]["sign"]["if"]), PUBLISH_GATE);
}

#[test]
fn no_job_but_signing_references_the_signing_key() {
    let rel = load("release.yml");
    for (job, body) in rel["jobs"].as_mapping().unwrap() {
        let text = yaml_serde::to_string(body).unwrap();
        let reads = text.contains("CARGO_PACKAGER_SIGN_PRIVATE_KEY");
        assert_eq!(reads, str_of(job) == "sign", "{job:?} signing key");
    }
}

#[test]
fn the_signing_job_only_downloads_signs_and_uploads() {
    // Nothing from the repository and nothing compiled runs beside the key: the signer is
    // the distro minisign at a pinned version, installed in a step that sees no secrets.
    let rel = load("release.yml");
    let sign = &rel["jobs"]["sign"];
    assert_eq!(
        strs(&sign["needs"]),
        ["guard", "assemble"],
        "sign takes the assembled files"
    );
    let mut runs = Vec::new();
    for step in steps(&rel, "sign") {
        let uses = str_of(&step["uses"]);
        let allowed = [
            "actions/download-artifact@",
            "actions/upload-artifact@",
            "actions/attest@",
        ];
        assert!(
            uses.is_empty() || allowed.iter().any(|a| uses.starts_with(a)),
            "sign uses {uses}"
        );
        if !str_of(&step["run"]).is_empty() {
            runs.push(str_of(&step["run"]).trim().to_owned());
        }
    }
    assert_eq!(runs.len(), 2, "install and sign: {runs:?}");
    assert_eq!(runs[0], MINISIGN_INSTALL);
    assert!(runs[1].contains("minisign -S"), "{}", runs[1]);
    let install = steps(&rel, "sign")
        .into_iter()
        .find(|s| str_of(&s["run"]).trim() == MINISIGN_INSTALL)
        .unwrap();
    assert!(install["env"].is_null(), "the install step sees no secrets");
}

/// Noble's minisign, pinned to the package version the fixture signature was made with.
const MINISIGN_INSTALL: &str = "sudo apt-get update -qq\n\
    sudo apt-get install -y -qq --no-install-recommends minisign=0.11-1";

#[test]
fn the_signing_key_is_piped_never_written_or_passed_as_an_argument() {
    // The password goes in on stdin and the key through a pipe, both from shell builtins,
    // so neither lands on disk or in a process's arguments.
    let rel = load("release.yml");
    let all = steps(&rel, "sign");
    let step = &all[step_running(&all, "minisign -S")];
    assert_eq!(str_of(&step["shell"]), "bash", "pipefail on");
    let run = str_of(&step["run"]);
    for piece in [
        r#"printf '%s\n' "$CARGO_PACKAGER_SIGN_PRIVATE_KEY_PASSWORD" |"#,
        r#"-s <(printf '%s' "$CARGO_PACKAGER_SIGN_PRIVATE_KEY" | base64 -d)"#,
        "-m update/latest.json -x update/latest.json.minisig",
        "base64 -w0 update/latest.json.minisig > update/latest.json.sig",
        "rm update/latest.json.minisig",
    ] {
        assert!(run.contains(piece), "missing {piece:?} in {run}");
    }
}

#[test]
fn no_job_builds_or_ships_a_signer_binary() {
    let text = std::fs::read_to_string(workflow_path("release.yml")).unwrap();
    assert!(!text.contains("--root signer"), "assemble builds no signer");
    assert!(!text.contains("name: signer"));
}

#[test]
fn secrets_reach_only_the_signing_step_env_and_are_documented() {
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
                    run.contains("minisign -S"),
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

/// The index of the first step in `all` whose script contains `needle`.
fn step_running(all: &[Value], needle: &str) -> usize {
    all.iter()
        .position(|s| str_of(&s["run"]).contains(needle))
        .unwrap_or_else(|| panic!("no step runs {needle}"))
}

#[test]
fn the_manifest_is_written_before_signing_and_uploaded_last() {
    let rel = load("release.yml");
    let assemble = steps(&rel, "assemble");
    let (sums, manifest) = (
        step_running(&assemble, "release.sh sums"),
        step_running(&assemble, "release.sh manifest"),
    );
    assert!(sums < manifest);
    // Only configured tag runs build a manifest; the toolchain serves only that and
    // shares the gate.
    let gate = "github.ref_type == 'tag' && vars.APINATA_UPDATE_BASE_URL != ''";
    let toolchain = assemble
        .iter()
        .position(|s| str_of(&s["uses"]).starts_with("dtolnay/rust-toolchain@"))
        .expect("a toolchain step");
    for i in [toolchain, manifest] {
        assert_eq!(str_of(&assemble[i]["if"]), gate, "assemble step {i}");
    }
    assert_eq!(strs(&rel["jobs"]["draft"]["needs"]), ["guard", "sign"]);
    let draft = steps(&rel, "draft");
    let run = str_of(&draft[step_running(&draft, "gh release create")]["run"]);
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
    let all = steps(&rel, "draft");
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
        .find(
            r#"scripts/release.sh notes "$GITHUB_REF_NAME" "$GITHUB_REPOSITORY" release-notes.md"#,
        )
        .expect("drafts the notes");
    let create = draft.find("gh release create").unwrap();
    assert!(notes < create, "{draft}");
    assert!(draft.contains("--notes-file release-notes.md"), "{draft}");
    assert!(!draft.contains("--generate-notes"), "{draft}");
}

#[test]
fn release_jobs_restore_no_cache() {
    // A cache written by another run could poison a release file before it is signed.
    let rel = load("release.yml");
    for (job, _) in rel["jobs"].as_mapping().unwrap() {
        for step in steps(&rel, str_of(job)) {
            let uses = str_of(&step["uses"]);
            assert!(
                !uses.starts_with("Swatinem/rust-cache@") && !uses.starts_with("actions/cache"),
                "{job:?} uses {uses}"
            );
            assert!(step["with"].get("cache").is_none(), "{job:?} {uses} caches");
        }
    }
}

#[test]
fn the_signing_job_ends_by_attesting_every_release_file() {
    let rel = load("release.yml");
    let all = steps(&rel, "sign");
    let last = all.last().unwrap();
    assert_eq!(
        str_of(&last["uses"]),
        "actions/attest@1e69f48acb82d1966a394da916b4c1698aa569d6"
    );
    let subjects: Vec<&str> = str_of(&last["with"]["subject-path"]).lines().collect();
    assert_eq!(subjects, ["dist/*", "update/*"]);
    // Attesting needs an OIDC token and write access to attestations; only this job has it.
    for (job, body) in rel["jobs"].as_mapping().unwrap() {
        let perms = &body["permissions"];
        let attests = ["id-token", "attestations", "artifact-metadata"]
            .iter()
            .any(|p| perms.get(p).is_some());
        assert_eq!(attests, str_of(job) == "sign", "{job:?} permissions");
    }
    let perms = &rel["jobs"]["sign"]["permissions"];
    let mut got: Vec<(&str, &str)> = perms
        .as_mapping()
        .unwrap()
        .iter()
        .map(|(k, v)| (str_of(k), str_of(v)))
        .collect();
    got.sort();
    assert_eq!(
        got,
        [
            ("artifact-metadata", "write"),
            ("attestations", "write"),
            ("id-token", "write")
        ]
    );
}

#[test]
fn windows_builds_are_never_code_signed() {
    let text = std::fs::read_to_string(workflow_path("release.yml")).unwrap();
    assert!(!text.contains("WINDOWS_CERTIFICATE"), "release.yml");
}

#[test]
fn ci_audits_the_workflows_with_a_pinned_zizmor() {
    let ci = load("ci.yml");
    let audit = steps(&ci, "zizmor")
        .into_iter()
        .find(|s| str_of(&s["uses"]).starts_with("zizmorcore/zizmor-action@"))
        .expect("a zizmor step");
    let with = &audit["with"];
    let version = str_of(&with["version"]);
    assert!(
        semver::Version::parse(version).is_ok(),
        "zizmor version {version:?} is not exact"
    );
    assert_eq!(str_of(&with["inputs"]), ".github/workflows");
    // Findings fail the job in the log; uploading SARIF would need security-events: write.
    assert_eq!(with["advanced-security"], Value::Bool(false));
}

#[test]
fn the_toolchain_action_is_pinned_to_its_master_branch() {
    // The stable branch is rewritten and orphans old pins; master only gains commits.
    for name in workflow_files() {
        let text = std::fs::read_to_string(workflow_path(&name)).unwrap();
        for line in text
            .lines()
            .filter(|l| l.contains("dtolnay/rust-toolchain@"))
        {
            assert!(
                line.trim_end().ends_with(
                    "dtolnay/rust-toolchain@683b588c487a69d01f6c6ddf40f036ff682cc2c8 # master"
                ),
                "{name}: {line}"
            );
        }
        let wf = load(&name);
        for (job, body) in wf["jobs"].as_mapping().unwrap() {
            for step in body["steps"].as_sequence().unwrap() {
                if str_of(&step["uses"]).starts_with("dtolnay/rust-toolchain@") {
                    assert_eq!(
                        str_of(&step["with"]["toolchain"]),
                        "stable",
                        "{name}: {job:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_release_run_is_never_cancelled_halfway() {
    let rel = load("release.yml");
    assert!(!str_of(&rel["concurrency"]["group"]).is_empty());
    assert_eq!(rel["concurrency"]["cancel-in-progress"], Value::Bool(false));
}

/// The recipe lines of a Makefile target, with the leading tab removed.
fn make_recipe(target: &str) -> Vec<String> {
    let text = std::fs::read_to_string(xtask::workspace_root().join("Makefile")).unwrap();
    text.lines()
        .skip_while(|l| !l.starts_with(&format!("{target}:")))
        .skip(1)
        .take_while(|l| l.starts_with('\t'))
        .map(|l| l.trim_start_matches('\t').to_owned())
        .collect()
}

#[test]
fn the_gate_holds_the_lock_and_builds_core_without_its_default_features() {
    // A manifest edit without a lock commit must fail rather than resolve silently.
    for target in ["lint", "test"] {
        let recipe = make_recipe(target);
        assert!(
            recipe.iter().any(|l| l.contains("--locked")),
            "make {target} is not --locked: {recipe:?}"
        );
    }
    let check = std::fs::read_to_string(xtask::workspace_root().join("Makefile")).unwrap();
    let prereqs = check
        .lines()
        .find_map(|l| l.strip_prefix("check:"))
        .expect("a check target");
    assert!(
        prereqs.split_whitespace().any(|t| t == "core-check"),
        "make check skips core-check: {prereqs}"
    );
    assert_eq!(
        make_recipe("core-check"),
        [
            "$(CARGO) check -p apinata-core --no-default-features --locked",
            "RUSTDOCFLAGS=-Dwarnings $(CARGO) doc --no-deps -p apinata-core --locked",
        ]
    );
}

#[test]
fn dependabot_bumps_the_pinned_actions_monthly_in_one_pull_request() {
    let text =
        std::fs::read_to_string(xtask::workspace_root().join(".github/dependabot.yml")).unwrap();
    let config: Value = yaml_serde::from_str(&text).unwrap();
    let actions = config["updates"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|u| str_of(&u["package-ecosystem"]) == "github-actions")
        .expect("a github-actions entry");
    assert_eq!(str_of(&actions["directory"]), "/");
    assert_eq!(str_of(&actions["schedule"]["interval"]), "monthly");
    assert_eq!(
        strs(&actions["groups"]["actions"]["patterns"]),
        ["*"],
        "all action bumps arrive as one grouped pull request"
    );
}

#[test]
fn macos_clippy_runs_only_when_a_dispatch_asks_for_it() {
    // macOS minutes bill at ten times Linux, so the job is opt-in and off by default.
    let ci = load("ci.yml");
    let input = &ci["on"]["workflow_dispatch"]["inputs"]["macos_clippy"];
    assert_eq!(str_of(&input["type"]), "boolean");
    assert_eq!(input["default"], Value::Bool(false));
    let job = &ci["jobs"]["macos-clippy"];
    assert_eq!(str_of(&job["runs-on"]), "macos-latest");
    assert_eq!(str_of(&job["if"]), "inputs.macos_clippy");
    let runs: Vec<String> = steps(&ci, "macos-clippy")
        .iter()
        .map(|s| str_of(&s["run"]).trim().to_owned())
        .filter(|r| !r.is_empty())
        .collect();
    assert_eq!(
        runs,
        ["cargo clippy --workspace --all-targets --locked -- -Dwarnings"]
    );
}
