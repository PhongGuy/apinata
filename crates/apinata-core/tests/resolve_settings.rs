//! Request settings with Bruno's coercions, and warnings for what Apinata does not run.

use crate::common;

use apinata_core::Warning;
use common::{at, setup};
use std::time::Duration;

/// A request file whose `top` follows `http:` at the top level.
fn http(top: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test/\n{top}")
}

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

#[test]
fn settings_follow_brunos_coercions() {
    let set = http(
        "settings:\n  timeout: 2500\n  followRedirects: false\n  maxRedirects: 2\n  forwardAuthorizationHeader: false\n  encodeUrl: true\n",
    );
    let inherit = http(
        "settings:\n  timeout: inherit\n  followRedirects: inherit\n  maxRedirects: inherit\n  forwardAuthorizationHeader: inherit\n",
    );
    let odd = http(
        "settings:\n  timeout: -5\n  followRedirects: \"false\"\n  maxRedirects: 2.9\n  forwardAuthorizationHeader: \"false\"\n",
    );
    let junk = http(
        "settings:\n  timeout: \"100\"\n  followRedirects: 0\n  maxRedirects: -1\n  forwardAuthorizationHeader: no-thanks\n",
    );
    let plain = http("");
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("plain.yml", &plain),
        ("set.yml", &set),
        ("inherit.yml", &inherit),
        ("odd.yml", &odd),
        ("junk.yml", &junk),
    ]);
    let s = |rel: &str| {
        let r = at(&c, &ctx, rel).expect(rel);
        (
            r.timeout(),
            r.follow_redirects(),
            r.max_redirects(),
            r.forward_authorization_header(),
        )
    };
    assert_eq!(s("plain.yml"), (None, true, 5, true));
    assert_eq!(
        s("set.yml"),
        (Some(Duration::from_millis(2500)), false, 2, false)
    );
    assert_eq!(s("inherit.yml"), (None, true, 5, true));
    assert_eq!(s("odd.yml"), (None, false, 2, false));
    assert_eq!(s("junk.yml"), (None, true, 5, true));
}

#[test]
fn scripts_actions_and_encode_url_false_produce_warnings() {
    let scripts = "  scripts:\n    - type: before-request\n      code: x\n";
    let actions = "  actions:\n    - type: set-variable\n";
    let folder = |block: &str| format!("info:\n  name: f\n  type: folder\nrequest:\n{block}");
    let (fs, fa) = (folder(scripts), folder(actions));
    let plain = http("");
    let own = http("runtime:\n  scripts:\n    - type: tests\n      code: t\n");
    let own_actions = http("runtime:\n  actions:\n    - type: set-variable\n");
    let empty = http("runtime:\n  scripts: []\n  actions: []\n");
    let all = http(
        "runtime:\n  scripts:\n    - type: tests\n      code: t\n  actions:\n    - type: set-variable\nsettings:\n  encodeUrl: false\n",
    );
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("fs/folder.yml", &fs),
        ("fa/folder.yml", &fa),
        ("plain.yml", &plain),
        ("own.yml", &own),
        ("act.yml", &own_actions),
        ("empty.yml", &empty),
        ("fs/inherited.yml", &plain),
        ("fa/inherited.yml", &plain),
        ("fs/all.yml", &all),
    ]);
    let w = |rel: &str| at(&c, &ctx, rel).expect(rel).warnings().to_vec();
    assert!(w("plain.yml").is_empty());
    assert!(w("empty.yml").is_empty());
    assert_eq!(w("own.yml"), [Warning::ScriptsNotRun]);
    assert_eq!(w("act.yml"), [Warning::ActionsNotRun]);
    assert_eq!(w("fs/inherited.yml"), [Warning::ScriptsNotRun]);
    assert_eq!(w("fa/inherited.yml"), [Warning::ActionsNotRun]);
    assert_eq!(
        w("fs/all.yml"),
        [
            Warning::ScriptsNotRun,
            Warning::ActionsNotRun,
            Warning::EncodeUrlIgnored
        ]
    );
    let root = format!("{ROOT}request:\n{scripts}{actions}");
    let (_dir2, c2, ctx2) = setup(&[("opencollection.yml", &root), ("plain.yml", &plain)]);
    assert_eq!(
        at(&c2, &ctx2, "plain.yml").expect("resolves").warnings(),
        [Warning::ScriptsNotRun, Warning::ActionsNotRun]
    );
    assert_eq!(Warning::ScriptsNotRun.to_string(), "scripts not executed");
}
