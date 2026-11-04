use super::*;
use crate::legacy::plan_conversion;
use std::fs;
use std::io;
use std::path::Path;

pub(super) const REQUEST: &str =
    "meta {\n  name: a\n  type: http\n  seq: 1\n}\n\nget {\n  url: https://example.com\n}\n";

pub(super) fn legacy(root: &Path) {
    for (rel, text) in [
        (
            "bruno.json",
            r#"{"version": "1", "name": "Legacy", "type": "collection"}"#.to_owned(),
        ),
        ("a.bru", REQUEST.to_owned()),
        ("f/b.bru", REQUEST.replace("name: a", "name: b")),
        (
            "environments/Local.bru",
            "vars {\n  host: http://localhost\n}\n".to_owned(),
        ),
        (".env", "TOKEN=x\n".to_owned()),
    ] {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}

#[test]
fn a_failure_while_staging_or_before_the_rename_leaves_nothing() {
    for fail_at in ["write", "rename"] {
        let src = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        legacy(src.path());
        let plan = plan_conversion(src.path()).unwrap();
        let mut writes = 0;
        let err = convert_with(
            &plan,
            ConvertTarget::NewFolderIn(parent.path().to_path_buf()),
            &mut |step| match step {
                Step::Write(_) if fail_at == "write" => {
                    writes += 1;
                    if writes == 2 {
                        Err(io::Error::other("injected"))
                    } else {
                        Ok(())
                    }
                }
                Step::Rename(_) if fail_at == "rename" => Err(io::Error::other("injected")),
                _ => Ok(()),
            },
        )
        .unwrap_err();
        assert!(
            matches!(&err, ConvertError::Io { message, .. } if message.contains("injected")),
            "{fail_at}: {err:?}"
        );
        assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0, "{fail_at}");
    }
}

fn tree(dir: &Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.insert(
                    path.strip_prefix(dir).unwrap().to_path_buf(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }
    out
}

#[test]
fn an_in_place_write_failure_removes_what_it_wrote_and_keeps_the_originals() {
    let src = tempfile::tempdir().unwrap();
    legacy(src.path());
    let before = tree(src.path());
    let plan = plan_conversion(src.path()).unwrap();
    let mut writes = 0;
    let err = convert_with(&plan, ConvertTarget::InPlace, &mut |step| match step {
        Step::Write(_) => {
            writes += 1;
            if writes == 3 {
                Err(io::Error::other("injected"))
            } else {
                Ok(())
            }
        }
        _ => Ok(()),
    })
    .unwrap_err();
    assert!(
        matches!(&err, ConvertError::Io { message, .. } if message.contains("injected")),
        "{err:?}"
    );
    assert_eq!(tree(src.path()), before);
}

#[test]
fn opencollection_yml_is_written_last_and_bruno_json_deleted_last() {
    let src = tempfile::tempdir().unwrap();
    legacy(src.path());
    // z.yml sorts after opencollection.yml, so a plain sorted write order would fail this test.
    fs::write(
        src.path().join("z.bru"),
        REQUEST.replace("name: a", "name: z"),
    )
    .unwrap();
    let plan = plan_conversion(src.path()).unwrap();
    let (mut written, mut removed) = (Vec::new(), Vec::new());
    convert_with(&plan, ConvertTarget::InPlace, &mut |step| {
        match step {
            Step::Write(p) => written.push(p.to_path_buf()),
            Step::Remove(p) => removed.push(p.to_path_buf()),
            Step::Rename(p) => panic!("in place never renames, yet renamed {p:?}"),
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(written.last(), Some(&src.path().join("opencollection.yml")));
    assert_eq!(removed.last(), Some(&src.path().join("bruno.json")));
    assert_eq!(removed.first(), Some(&src.path().join("a.bru")));
}

#[test]
fn a_target_appearing_mid_write_keeps_the_foreign_file_and_removes_our_writes() {
    let src = tempfile::tempdir().unwrap();
    legacy(src.path());
    let plan = plan_conversion(src.path()).unwrap();
    let mut before = tree(src.path());
    let foreign = src.path().join("f/b.yml");
    // Another writer creates f/b.yml just before this call writes it.
    let err = convert_with(&plan, ConvertTarget::InPlace, &mut |step| {
        if let Step::Write(p) = step
            && p == foreign
        {
            fs::write(p, "foreign\n")?;
        }
        Ok(())
    })
    .unwrap_err();
    assert!(
        matches!(&err, ConvertError::TargetExists { paths } if *paths == [std::path::PathBuf::from("f/b.yml")]),
        "{err:?}"
    );
    before.insert("f/b.yml".into(), b"foreign\n".to_vec());
    assert_eq!(tree(src.path()), before);
}

#[test]
fn a_delete_failure_stops_with_the_yaml_in_place() {
    let src = tempfile::tempdir().unwrap();
    legacy(src.path());
    let plan = plan_conversion(src.path()).unwrap();
    let mut removes = 0;
    let err = convert_with(&plan, ConvertTarget::InPlace, &mut |step| match step {
        Step::Remove(_) => {
            removes += 1;
            if removes == 2 {
                Err(io::Error::other("injected"))
            } else {
                Ok(())
            }
        }
        _ => Ok(()),
    })
    .unwrap_err();
    assert!(
        matches!(&err, ConvertError::Io { path, .. } if *path == src.path().join("environments/Local.bru")),
        "{err:?}"
    );
    assert!(!src.path().join("a.bru").exists());
    assert!(
        src.path().join("environments/Local.bru").is_file()
            && src.path().join("bruno.json").is_file()
    );
    let c = crate::Collection::open(src.path()).unwrap();
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
}

#[test]
fn a_source_edited_during_the_write_phase_is_kept_and_reported() {
    let src = tempfile::tempdir().unwrap();
    legacy(src.path());
    let plan = plan_conversion(src.path()).unwrap();
    let edited = src.path().join("f/b.bru");
    // The edit lands after the up-front check but before its own delete.
    let err = convert_with(&plan, ConvertTarget::InPlace, &mut |step| match step {
        Step::Remove(p) if p == src.path().join("a.bru") => fs::write(&edited, "edited\n"),
        _ => Ok(()),
    })
    .unwrap_err();
    assert!(
        matches!(&err, ConvertError::SourceChanged { path } if path == Path::new("f/b.bru")),
        "{err:?}"
    );
    assert_eq!(fs::read_to_string(&edited).unwrap(), "edited\n");
    assert!(!src.path().join("a.bru").exists());
}

#[cfg(unix)]
#[test]
fn a_copied_file_swapped_for_a_symlink_after_planning_is_refused() {
    let src = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    legacy(src.path());
    fs::write(outside.path().join("secret"), "secret\n").unwrap();
    let plan = plan_conversion(src.path()).unwrap();
    let env = src.path().join(".env");
    let err = convert_with(
        &plan,
        ConvertTarget::NewFolder(parent.path().join("out")),
        &mut |step| {
            if let Step::Write(p) = step
                && p.ends_with(".env")
            {
                fs::remove_file(&env)?;
                std::os::unix::fs::symlink(outside.path().join("secret"), &env)?;
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert!(
        matches!(&err, ConvertError::SourceChanged { path } if path == Path::new(".env")),
        "{err:?}"
    );
    assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
}

#[test]
fn a_copied_file_edited_after_planning_is_refused_and_leaves_nothing() {
    for during in [false, true] {
        let src = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        legacy(src.path());
        let plan = plan_conversion(src.path()).unwrap();
        let env = src.path().join(".env");
        if !during {
            fs::write(&env, "TOKEN=changed\n").unwrap();
        }
        let err = convert_with(
            &plan,
            ConvertTarget::NewFolder(parent.path().join("out")),
            &mut |step| match step {
                Step::Write(p) if during && p.ends_with(".env") => {
                    fs::write(&env, "TOKEN=changed\n")
                }
                _ => Ok(()),
            },
        )
        .unwrap_err();
        assert!(
            matches!(&err, ConvertError::SourceChanged { path } if path == Path::new(".env")),
            "during={during}: {err:?}"
        );
        assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
        // A new folder never deletes sources.
        assert_eq!(fs::read_to_string(&env).unwrap(), "TOKEN=changed\n");
        assert!(src.path().join("a.bru").is_file() && src.path().join("bruno.json").is_file());
    }
}

#[test]
fn new_folder_io_errors_name_the_target_not_the_staging_path() {
    for victim in ["a.yml", ".env"] {
        let src = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        legacy(src.path());
        let plan = plan_conversion(src.path()).unwrap();
        let dir = parent.path().join("out");
        let err = convert_with(
            &plan,
            ConvertTarget::NewFolder(dir.clone()),
            &mut |step| match step {
                Step::Write(p) if p.ends_with(victim) => Err(io::Error::other("injected")),
                _ => Ok(()),
            },
        )
        .unwrap_err();
        let ConvertError::Io { path, .. } = &err else {
            panic!("{victim}: {err:?}");
        };
        assert!(path.starts_with(&dir), "{victim}: {path:?}");
        assert!(
            !path.to_string_lossy().contains(".apinata-convert"),
            "{path:?}"
        );
        assert_eq!(path, &dir.join(victim));
    }
}
