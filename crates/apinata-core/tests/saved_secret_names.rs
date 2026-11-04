use apinata_core::Collection;
use std::fs;

const DEV: &str =
    "name: dev\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: token\n";

fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    )
    .unwrap();
    fs::create_dir(dir.path().join("environments")).unwrap();
    fs::write(dir.path().join("environments/dev.yml"), DEV).unwrap();
    dir
}

#[test]
fn saved_secret_names_read_the_file_not_the_draft() {
    let dir = tree();
    let mut c = Collection::open(dir.path()).unwrap();
    let env = c.environment_mut("dev").unwrap();
    let plain = env.variables().into_iter().filter(|v| !v.secret).collect();
    env.set_variables(plain);
    assert_eq!(c.saved_secret_names("dev"), ["token"]);
    c.environment_mut("dev").unwrap().save().unwrap();
    assert!(c.saved_secret_names("dev").is_empty());
}

#[test]
fn unknown_missing_or_broken_files_have_no_saved_secrets() {
    let dir = tree();
    let c = Collection::open(dir.path()).unwrap();
    assert!(c.saved_secret_names("nope").is_empty());
    fs::write(dir.path().join("environments/dev.yml"), "variables: [\n").unwrap();
    assert!(c.saved_secret_names("dev").is_empty());
    fs::remove_file(dir.path().join("environments/dev.yml")).unwrap();
    assert!(c.saved_secret_names("dev").is_empty());
}

#[test]
fn a_failed_read_is_told_apart_from_a_file_without_secrets() {
    let dir = tree();
    let c = Collection::open(dir.path()).unwrap();
    assert_eq!(
        c.try_saved_secret_names("dev").as_deref(),
        Some(&["token".to_owned()][..])
    );
    fs::write(dir.path().join("environments/dev.yml"), "name: dev\n").unwrap();
    assert_eq!(c.try_saved_secret_names("dev"), Some(Vec::new()));
    assert_eq!(c.try_saved_secret_names("nope"), None);
    fs::write(dir.path().join("environments/dev.yml"), "variables: [\n").unwrap();
    assert_eq!(c.try_saved_secret_names("dev"), None);
    fs::remove_file(dir.path().join("environments/dev.yml")).unwrap();
    assert_eq!(c.try_saved_secret_names("dev"), None);
}

#[test]
fn saved_variables_read_the_file_not_the_draft() {
    let dir = tree();
    let mut c = Collection::open(dir.path()).unwrap();
    let env = c.environment_mut("dev").unwrap();
    let mut vars = env.variables();
    vars[1].enabled = false;
    env.set_variables(vars);
    let saved = c.try_saved_variables("dev").expect("readable");
    let rows: Vec<_> = saved
        .iter()
        .map(|v| (v.name.as_str(), v.enabled, v.secret))
        .collect();
    assert_eq!(rows, [("host", true, false), ("token", true, true)]);
    assert!(c.try_saved_variables("nope").is_none());
    fs::write(dir.path().join("environments/dev.yml"), "variables: [\n").unwrap();
    assert!(c.try_saved_variables("dev").is_none());
}
