//! The Network tab's client certificate rows.

use crate::support;

use apinata_app::command::Edit;
use apinata_app::network_section::NetworkSection;
use apinata_core::{ClientCertificate, CollectionNetwork};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, ElementId, Entity, TestAppContext};
use std::path::PathBuf;
use support::*;

type Host = Entity<SectionHost<NetworkSection>>;

fn section(cx: &mut TestAppContext, certs: Vec<ClientCertificate>) -> (AnyWindowHandle, Host) {
    let (win, host) = host_section(cx, |w, cx| NetworkSection::new(PathBuf::from("/c"), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    let mut network = CollectionNetwork::default();
    network.certificates = certs;
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&network, false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    (win, host)
}

fn sent(cx: &mut TestAppContext, host: &Host) -> Option<Vec<ClientCertificate>> {
    host.read_with(cx, |h, _| {
        h.edits.iter().rev().find_map(|e| match e {
            Edit::ClientCertificates(c) => Some(c.clone()),
            _ => None,
        })
    })
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) {
    let id = id.into();
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, win);
}

#[gpui_kit::test]
fn an_added_row_is_sent_once_its_domain_and_paths_are_filled(cx: &mut TestAppContext) {
    let (win, host) = section(cx, vec![]);
    click(cx, win, "cert-add");
    type_at_end(cx, win, None, ("cert-domain", 0usize), "api.test");
    type_at_end(cx, win, None, ("cert-file", 0usize), "c.pem");
    cx.run_until_parked();
    assert_eq!(sent(cx, &host), None, "an unfilled row stays a local draft");
    type_at_end(cx, win, None, ("cert-key", 0usize), "k.pem");
    cx.run_until_parked();
    assert_eq!(
        sent(cx, &host),
        Some(vec![ClientCertificate::pem("api.test", "c.pem", "k.pem")])
    );
}

#[gpui_kit::test]
fn a_draft_row_survives_a_load_and_its_removal_sends_nothing(cx: &mut TestAppContext) {
    let loaded = ClientCertificate::pem("a.test", "", "");
    let (win, host) = section(cx, vec![loaded.clone()]);
    click(cx, win, "cert-add");
    type_at_end(cx, win, None, ("cert-domain", 1usize), "b.test");
    // A snapshot echo of the file's list keeps the draft and its typing.
    let s = host.read_with(cx, |h, _| h.view.clone());
    let mut network = CollectionNetwork::default();
    network.certificates = vec![loaded];
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&network, false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    let domain = s.read_with(cx, |s, cx| {
        s.cert_input(1, apinata_app::network_section::CertField::Domain)
            .map(|i| i.read(cx).value().to_string())
    });
    assert_eq!(domain.as_deref(), Some("b.test"));
    click(cx, win, ("cert-remove", 1usize));
    assert_eq!(
        sent(cx, &host),
        None,
        "neither the draft nor its removal is sent"
    );
    assert!(!present(cx, win, ("cert-domain", 1usize)));
}

#[gpui_kit::test]
fn picked_files_become_collection_relative(cx: &mut TestAppContext) {
    let (win, host) = section(cx, vec![ClientCertificate::pem("a.test", "", "")]);
    click(cx, win, ("cert-file-pick", 0usize));
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|o| {
        assert!(o.files && !o.directories && !o.multiple);
        Some(vec![PathBuf::from("/c/certs/c.pem")])
    });
    cx.run_until_parked();
    click(cx, win, ("cert-key-pick", 0usize));
    cx.simulate_path_prompt_response(|_| Some(vec![PathBuf::from("/keys/k.pem")]));
    cx.run_until_parked();
    assert_eq!(
        sent(cx, &host),
        Some(vec![ClientCertificate::pem(
            "a.test",
            "certs/c.pem",
            "/keys/k.pem"
        )])
    );
    let before = host.read_with(cx, |h, _| h.edits.len());
    click(cx, win, ("cert-file-pick", 0usize));
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    assert_eq!(host.read_with(cx, |h, _| h.edits.len()), before);
}

#[gpui_kit::test]
fn the_passphrase_is_sent_on_enter_only(cx: &mut TestAppContext) {
    let (win, host) = section(cx, vec![ClientCertificate::pem("a.test", "c.pem", "k.pem")]);
    type_at_end(cx, win, None, ("cert-passphrase", 0usize), "pp");
    cx.run_until_parked();
    assert_eq!(sent(cx, &host), None);
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    let mut want = ClientCertificate::pem("a.test", "c.pem", "k.pem");
    want.passphrase = "pp".into();
    assert_eq!(sent(cx, &host), Some(vec![want]));
}

#[gpui_kit::test]
fn enabled_and_remove(cx: &mut TestAppContext) {
    let (win, host) = section(cx, vec![ClientCertificate::pem("a.test", "c.pem", "k.pem")]);
    click(cx, win, ("cert-enabled", 0usize));
    let mut off = ClientCertificate::pem("a.test", "c.pem", "k.pem");
    off.enabled = false;
    assert_eq!(sent(cx, &host), Some(vec![off]));
    click(cx, win, ("cert-remove", 0usize));
    assert_eq!(sent(cx, &host), Some(vec![]));
}

#[gpui_kit::test]
fn pkcs12_rows_are_read_only_and_kept_in_place(cx: &mut TestAppContext) {
    let p12 = ClientCertificate::pkcs12("p.test", "c.p12");
    let (win, host) = section(
        cx,
        vec![
            p12.clone(),
            ClientCertificate::pem("a.test", "c.pem", "k.pem"),
        ],
    );
    assert!(present(cx, win, ("cert-pkcs12-hint", 0usize)));
    assert!(
        !present(cx, win, ("cert-domain", 0usize)) && !present(cx, win, ("cert-remove", 0usize))
    );
    let s = host.read_with(cx, |h, _| h.view.clone());
    assert!(s.read_with(cx, |s, _| {
        s.cert_input(0, apinata_app::network_section::CertField::Domain)
            .is_none()
    }));
    type_at_end(cx, win, None, ("cert-domain", 1usize), "x");
    cx.run_until_parked();
    assert_eq!(
        sent(cx, &host),
        Some(vec![
            p12,
            ClientCertificate::pem("a.testx", "c.pem", "k.pem")
        ])
    );
}

#[gpui_kit::test]
fn a_picked_file_carries_the_committed_passphrase(cx: &mut TestAppContext) {
    let (win, host) = section(cx, vec![ClientCertificate::pem("a.test", "c.pem", "k.pem")]);
    type_at_end(cx, win, None, ("cert-passphrase", 0usize), "pp");
    click(cx, win, ("cert-file-pick", 0usize));
    cx.simulate_path_prompt_response(|_| Some(vec![PathBuf::from("/c/d.pem")]));
    cx.run_until_parked();
    assert_eq!(
        sent(cx, &host),
        Some(vec![ClientCertificate::pem("a.test", "d.pem", "k.pem")])
    );
}

#[gpui_kit::test]
fn a_file_outside_the_collection_stays_absolute_with_a_notice(cx: &mut TestAppContext) {
    let notice = |cx: &mut TestAppContext, win| {
        cx.update_window(win, |_, w, _| {
            w.try_find(("cert-outside", 0usize)).is_some()
        })
        .unwrap()
    };
    let (win, host) = section(cx, vec![ClientCertificate::pem("a.test", "", "")]);
    click(cx, win, ("cert-file-pick", 0usize));
    cx.simulate_path_prompt_response(|_| Some(vec![PathBuf::from("/elsewhere/cert.pem")]));
    cx.run_until_parked();
    frame(cx, win);
    assert_eq!(
        sent(cx, &host),
        Some(vec![ClientCertificate::pem(
            "a.test",
            "/elsewhere/cert.pem",
            ""
        )])
    );
    assert!(notice(cx, win));
    let text = cx
        .update_window(win, |_, w, _| {
            w.find(("cert-outside", 0usize)).label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(
        text.as_deref(),
        Some("Outside the collection: saved as an absolute path")
    );
    click(cx, win, ("cert-file-pick", 0usize));
    cx.simulate_path_prompt_response(|_| Some(vec![PathBuf::from("/c/certs/cert.pem")]));
    cx.run_until_parked();
    frame(cx, win);
    assert_eq!(
        sent(cx, &host),
        Some(vec![ClientCertificate::pem("a.test", "certs/cert.pem", "")])
    );
    assert!(!notice(cx, win));
}

#[gpui_kit::test]
fn an_absolute_path_inside_the_collection_shows_no_notice(cx: &mut TestAppContext) {
    let notice = |cx: &mut TestAppContext, win| {
        cx.update_window(win, |_, w, _| {
            w.try_find(("cert-outside", 0usize)).is_some()
        })
        .unwrap()
    };
    let (win, _host) = section(
        cx,
        vec![ClientCertificate::pem("a.test", "/c/certs/cert.pem", "")],
    );
    assert!(!notice(cx, win), "a loaded path inside the root");
    let (win, _host) = section(cx, vec![ClientCertificate::pem("a.test", "", "")]);
    type_at_end(cx, win, None, ("cert-file", 0usize), "/elsewhere/cert.pem");
    cx.run_until_parked();
    frame(cx, win);
    assert!(notice(cx, win), "a typed path outside the root");
}

#[gpui_kit::test]
fn a_template_passphrase_shows_and_a_literal_stays_masked(cx: &mut TestAppContext) {
    use apinata_app::network_section::CertField;
    let cert = |p: &str| {
        let mut c = ClientCertificate::pem("a.test", "c.pem", "k.pem");
        c.passphrase = p.into();
        c
    };
    for (passphrase, masked) in [("{{process.env.P}}", false), ("pp", true)] {
        let (_win, host) = section(cx, vec![cert(passphrase)]);
        let s = host.read_with(cx, |h, _| h.view.clone());
        let input = s.read_with(cx, |s, _| {
            s.cert_input(0, CertField::Passphrase).cloned().unwrap()
        });
        assert_eq!(
            input.read_with(cx, |i, _| i.presentation().is_masked()),
            masked,
            "{passphrase}"
        );
    }
}

// The root as opened through a symlink and a certificate path as the picker resolves it
// name the same collection, so no notice shows.
#[cfg(unix)]
#[gpui_kit::test]
fn a_resolved_path_inside_a_symlinked_collection_shows_no_notice(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir_all(real.join("certs")).unwrap();
    std::fs::write(real.join("certs").join("cert.pem"), "pem").unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let (win, host) = host_section(cx, |w, cx| NetworkSection::new(link.clone(), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    let cert = real.join("certs").join("cert.pem");
    let mut network = CollectionNetwork::default();
    network.certificates = vec![ClientCertificate::pem("a.test", cert.to_string_lossy(), "")];
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&network, false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    let shown = cx
        .update_window(win, |_, w, _| {
            w.try_find(("cert-outside", 0usize)).is_some()
        })
        .unwrap();
    assert!(!shown, "{} is inside {}", cert.display(), link.display());
}

// A path that does not exist has no canonical form, so only the lexical test applies: a
// missing file elsewhere or under the resolved root is flagged, one under the root as
// opened is not.
#[cfg(unix)]
#[gpui_kit::test]
fn a_missing_certificate_falls_back_to_the_plain_comparison(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir_all(real.join("certs")).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let (win, host) = host_section(cx, |w, cx| NetworkSection::new(link.clone(), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    let paths = [
        dir.path().join("elsewhere").join("gone.pem"),
        link.join("certs").join("gone.pem"),
        real.join("certs").join("gone.pem"),
    ];
    let mut network = CollectionNetwork::default();
    network.certificates = paths
        .iter()
        .map(|p| ClientCertificate::pem("a.test", p.to_string_lossy(), ""))
        .collect();
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&network, false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    let shown: Vec<bool> = cx
        .update_window(win, |_, w, _| {
            (0..paths.len())
                .map(|ix| w.try_find(("cert-outside", ix)).is_some())
                .collect()
        })
        .unwrap();
    assert_eq!(shown, [true, false, true], "{paths:?}");
}

// The outside check runs when a row's paths change, not on every render: a file vanishing
// under an unchanged path keeps the row's answer until the path is edited.
#[cfg(unix)]
#[gpui_kit::test]
fn the_outside_check_runs_once_per_path_not_per_render(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir_all(real.join("certs")).unwrap();
    let cert = real.join("certs").join("cert.pem");
    std::fs::write(&cert, "pem").unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let (win, host) = host_section(cx, |w, cx| NetworkSection::new(link.clone(), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    let mut network = CollectionNetwork::default();
    network.certificates = vec![ClientCertificate::pem("a.test", cert.to_string_lossy(), "")];
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&network, false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    let shown = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, _| {
            w.try_find(("cert-outside", 0usize)).is_some()
        })
        .unwrap()
    };
    assert!(!shown(cx), "the resolved file lies inside the collection");
    // Without its file the path has no canonical form; a recheck would flag it.
    std::fs::remove_file(&cert).unwrap();
    s.update(cx, |_, cx| cx.notify());
    frame(cx, win);
    assert!(!shown(cx), "a render alone re-ran the outside check");
    type_at_end(cx, win, None, ("cert-file", 0usize), "x");
    cx.run_until_parked();
    frame(cx, win);
    assert!(shown(cx), "an edited path is checked again");
}
