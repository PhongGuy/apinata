//! `scrub_example_userinfo`: removes a username and password from saved examples' recorded
//! URLs, as a dry run unless asked to write, and never touches the request's own URL.

use crate::common::write_tree;

use apinata_core::{Collection, scrub_example_userinfo};
use std::fs;
use std::time::{Duration, SystemTime};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
const PASSWORD: &str = "pa55word";

/// A request whose own URL and first example carry userinfo; its second example has none.
const LEAKY: &str = "info:
  name: leaky
  type: http
  seq: 1

http:
  method: GET
  url: https://u:pa55word@x.test/

examples:
  - name: one
    request:
      url: https://u:pa55word@x.test/a
      method: GET
    response:
      status: 200
  - name: two
    request:
      url: https://x.test/b
      method: GET
    response:
      status: 200
";

/// A request with an example and no userinfo anywhere.
const CLEAN: &str = "info:
  name: clean
  type: http
  seq: 2

http:
  method: GET
  url: https://x.test/

examples:
  - name: plain
    request:
      url: https://x.test/c
      method: GET
    response:
      status: 200
";

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("leaky.yml", LEAKY),
        ("f/clean.yml", CLEAN),
    ])
}

fn report_text(r: &apinata_core::ScrubReport) -> String {
    r.changes()
        .iter()
        .map(|c| format!("{} {} {}", c.item(), c.example(), c.url()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_dry_run_lists_the_scrubbed_urls_and_writes_nothing() {
    let dir = tree();
    let mut c = Collection::open(dir.path()).unwrap();
    let r = scrub_example_userinfo(&mut c, false).unwrap();
    assert!(!r.written());
    assert_eq!(report_text(&r), "leaky.yml one https://x.test/a");
    assert!(!report_text(&r).contains(PASSWORD));
    assert_eq!(
        fs::read_to_string(dir.path().join("leaky.yml")).unwrap(),
        LEAKY
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("f/clean.yml")).unwrap(),
        CLEAN
    );
}

#[test]
fn writing_scrubs_the_example_and_keeps_the_request_url() {
    let dir = tree();
    let mut c = Collection::open(dir.path()).unwrap();
    let r = scrub_example_userinfo(&mut c, true).unwrap();
    assert!(r.written());
    assert_eq!(report_text(&r), "leaky.yml one https://x.test/a");
    let want = LEAKY.replace("url: https://u:pa55word@x.test/a", "url: https://x.test/a");
    assert_eq!(
        fs::read_to_string(dir.path().join("leaky.yml")).unwrap(),
        want
    );
    let again = Collection::open(dir.path()).unwrap();
    let file = again
        .request(&apinata_core::ItemPath::new("leaky.yml").unwrap())
        .unwrap();
    assert_eq!(file.url(), "https://u:pa55word@x.test/");
    assert_eq!(
        file.examples()[0].request.as_ref().unwrap().url,
        "https://x.test/a"
    );
}

#[test]
fn a_file_without_userinfo_in_its_examples_is_not_rewritten() {
    let dir = tree();
    let clean = dir.path().join("f/clean.yml");
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    fs::File::options()
        .write(true)
        .open(&clean)
        .unwrap()
        .set_modified(old)
        .unwrap();
    let mut c = Collection::open(dir.path()).unwrap();
    scrub_example_userinfo(&mut c, true).unwrap();
    assert_eq!(fs::metadata(&clean).unwrap().modified().unwrap(), old);
    assert_eq!(fs::read_to_string(&clean).unwrap(), CLEAN);
}

#[test]
fn nothing_to_scrub_is_an_empty_report() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("f/clean.yml", CLEAN)]);
    let mut c = Collection::open(dir.path()).unwrap();
    let r = scrub_example_userinfo(&mut c, true).unwrap();
    assert!(r.changes().is_empty());
    assert!(!r.written());
}
