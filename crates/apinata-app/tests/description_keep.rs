//! A `description` key survives editing and saving: on a request or environment variable,
//! which the app does not show, and a multi-line or empty one in a described table.

use apinata_app::env_table::{env_rows, env_vars_from};
use apinata_app::kv_table::{
    KvRow, header_rows, headers_from, multipart_from, multipart_rows, param_rows, params_from,
    var_rows, vars_from,
};
use apinata_core::{BodySpec, Collection, RequestFile};
use std::fs;
use tempfile::TempDir;

const REQUEST: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://x.test\nruntime:\n  variables:\n    - name: token\n      value: abc\n      description: Why it exists\n";

const ENV: &str =
    "name: dev\nvariables:\n  - name: host\n    value: x\n    description: The API host\n";

#[test]
fn a_request_variable_keeps_its_description_through_an_edit() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("a.yml");
    fs::write(&path, REQUEST).unwrap();
    let mut request = RequestFile::load(&path).expect("loads");
    let base = request.vars();
    let mut rows = var_rows(&base);
    rows[0].value = "xyz".into();
    request.set_vars(vars_from(&base, &rows));
    request.save().expect("saves");
    let disk = fs::read_to_string(&path).unwrap();
    assert!(disk.contains("value: xyz"), "{disk}");
    assert!(disk.contains("description: Why it exists"), "{disk}");
}

#[test]
fn an_environment_variable_keeps_its_description_through_an_edit() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    )
    .unwrap();
    fs::create_dir(dir.path().join("environments")).unwrap();
    fs::write(dir.path().join("environments/dev.yml"), ENV).unwrap();
    let mut c = Collection::open(dir.path()).expect("opens");
    let env = c.environment_mut("dev").expect("dev");
    let base = env.variables();
    let mut rows = env_rows(&base);
    rows[0].value = "y".into();
    env.set_variables(env_vars_from(&base, &rows));
    env.save().expect("saves");
    let disk = fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap();
    assert!(disk.contains("value: y"), "{disk}");
    assert!(disk.contains("description: The API host"), "{disk}");
}

// Three entries per table: a multi-line description, an empty one and none, so one edit to
// the third row shows what the first two keep.
const ENTRIES: &str = "      - name: a\n        value: \"1\"\n        description: |-\n          line1\n          line2\n      - name: b\n        value: \"2\"\n        description: \"\"\n      - name: c\n        value: \"3\"\n";

fn described_request(body: &str) -> String {
    let indent = |text: &str, by: &str| {
        text.lines()
            .map(|l| format!("{by}{l}\n"))
            .collect::<String>()
    };
    format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: https://x.test\n  params:\n{}  headers:\n{ENTRIES}  body:\n    type: {body}\n    data:\n{}",
        indent(
            &ENTRIES.replace("value: \"", "type: query\n        value: \""),
            ""
        ),
        indent(
            &if body == "multipart-form" {
                ENTRIES.replace("value: \"", "type: text\n        value: \"")
            } else {
                ENTRIES.to_owned()
            },
            "  "
        ),
    )
}

// Edit the third row of every described table the way the app does: the single-line inputs
// show each description with CR and LF removed, and the edit is a new value on row 2.
fn edit_third_rows(request: &mut RequestFile) {
    let shown = |mut rows: Vec<KvRow>| {
        for row in &mut rows {
            row.description.retain(|c| c != '\n' && c != '\r');
        }
        rows[2].value = "9".into();
        rows
    };
    let headers = request.headers();
    request
        .set_headers(headers_from(&headers, &shown(header_rows(&headers))))
        .unwrap();
    let params = request.params();
    request.set_params(params_from(&params, &shown(param_rows(&params))));
    request.set_body(match request.body() {
        BodySpec::FormUrlEncoded(kv) => {
            BodySpec::FormUrlEncoded(headers_from(&kv, &shown(header_rows(&kv))))
        }
        BodySpec::Multipart(f) => {
            BodySpec::Multipart(multipart_from(&f, &shown(multipart_rows(&f))))
        }
        other => panic!("unexpected body {other:?}"),
    });
}

fn edited(body: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("a.yml");
    fs::write(&path, described_request(body)).unwrap();
    let mut request = RequestFile::load(&path).expect("loads");
    edit_third_rows(&mut request);
    request.save().expect("saves");
    (dir, path)
}

#[test]
fn a_multi_line_description_survives_an_edit_to_another_row() {
    for body in ["form-urlencoded", "multipart-form"] {
        let (_dir, path) = edited(body);
        let disk = fs::read_to_string(&path).unwrap();
        assert_eq!(disk.matches("value: \"9\"").count(), 3, "{disk}");
        let request = RequestFile::load(&path).expect("reloads");
        let multi = Some("line1\nline2");
        assert_eq!(request.headers()[0].description.as_deref(), multi, "{disk}");
        assert_eq!(request.params()[0].description.as_deref(), multi, "{disk}");
        let body_note = match request.body() {
            BodySpec::FormUrlEncoded(kv) => kv[0].description.clone(),
            BodySpec::Multipart(f) => f[0].description.clone(),
            other => panic!("unexpected body {other:?}"),
        };
        assert_eq!(body_note.as_deref(), multi, "{disk}");
    }
}

#[test]
fn an_empty_description_keeps_its_key_through_an_edit_to_another_row() {
    for body in ["form-urlencoded", "multipart-form"] {
        let (_dir, path) = edited(body);
        let disk = fs::read_to_string(&path).unwrap();
        assert_eq!(disk.matches("value: \"9\"").count(), 3, "{disk}");
        assert_eq!(disk.matches("description: \"\"").count(), 3, "{disk}");
    }
}
