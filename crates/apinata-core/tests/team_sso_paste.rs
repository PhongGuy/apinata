//! Single sign-on finished by pasting the address the browser ended on: which pastes are
//! refused and why, a valid paste signing in, and a paste racing the loopback listener.

use crate::common::potluck::*;
use crate::team_sso::{
    callback_addr, file, fresh, land, nothing_kept, param, port_closes, redeem_bodies, redeemed,
};
use apinata_core::CancelHandle;
use apinata_core::team::{self, PasteRefused, TeamError};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const LONG: Duration = Duration::from_secs(10);
const REDEEM: &str = "/api/v1/oidc/redeem";
const EXPIRES: &str = "2099-01-01T00:00:00Z";

/// Gives `finish` time to start waiting before a paste arrives.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}

/// Plays a browser landing on the loopback that may already have closed, as when a paste won.
async fn land_quietly(addr: SocketAddr, query: String) {
    let _ = tokio::task::spawn_blocking(move || {
        let mut s = TcpStream::connect_timeout(&addr, LONG).ok()?;
        s.set_read_timeout(Some(LONG)).ok()?;
        s.set_write_timeout(Some(LONG)).ok()?;
        write!(s, "GET /callback?{query} HTTP/1.1\r\nHost: {addr}\r\n\r\n").ok()?;
        let mut out = String::new();
        s.read_to_string(&mut out).ok()
    })
    .await;
}

#[tokio::test]
async fn each_bad_paste_is_refused_for_its_reason_and_the_wait_goes_on() {
    let (s, dir, secrets) = fresh().await;
    answer(&s, "POST", REDEEM, redeemed(9, EXPIRES)).await;
    let api = client(&s);
    let pending = team::begin_sso(&api).await.unwrap();
    let start = pending.url().to_owned();
    let (state, addr) = (param(&start, "state"), callback_addr(&start));
    let port = addr.port();
    let paste = pending.paste_handle();
    let cancel = CancelHandle::new();
    let team_file = file(&dir);
    let at = |rest: &str| format!("http://127.0.0.1:{port}{rest}");
    let cases = [
        (
            "forged-code is not an address".to_owned(),
            PasteRefused::NotThisAddress,
        ),
        (
            format!("http://localhost:{port}/callback?code=forged-code&state={state}"),
            PasteRefused::NotThisAddress,
        ),
        (
            format!("https://127.0.0.1:{port}/callback?code=forged-code&state={state}"),
            PasteRefused::NotThisAddress,
        ),
        (
            format!(
                "http://127.0.0.1:{}/callback?code=forged-code&state={state}",
                port.wrapping_add(1)
            ),
            PasteRefused::NotThisAddress,
        ),
        (
            at(&format!("/other?code=forged-code&state={state}")),
            PasteRefused::NotThisAddress,
        ),
        (
            format!("http://forged-code@127.0.0.1:{port}/callback?code=c&state={state}"),
            PasteRefused::NotThisAddress,
        ),
        (at("/callback?code=forged-code"), PasteRefused::WrongState),
        (
            at("/callback?code=forged-code&state=forged-state"),
            PasteRefused::WrongState,
        ),
        (
            at(&format!(
                "/callback?code=forged-code&state={state}&state={state}"
            )),
            PasteRefused::WrongState,
        ),
        (
            at("/callback?error=account_not_linked&state=forged-state"),
            PasteRefused::WrongState,
        ),
        (
            at(&format!("/callback?state={state}")),
            PasteRefused::NoCode,
        ),
        (
            at(&format!("/callback?code=&state={state}")),
            PasteRefused::NoCode,
        ),
        (
            at(&format!("/callback?code=&code=forged-code&state={state}")),
            PasteRefused::NoCode,
        ),
        (
            at(&format!("/callback?code=forged-code&code=c&state={state}")),
            PasteRefused::NoCode,
        ),
        (
            at(&format!("/callback?error=&code=forged-code&state={state}")),
            PasteRefused::NoCode,
        ),
        (
            at(&format!(
                "/callback?error=oidc_refused&error=x&state={state}"
            )),
            PasteRefused::NoCode,
        ),
        (
            at(&format!("/callback/?code=forged-code&state={state}")),
            PasteRefused::NotThisAddress,
        ),
        (
            at(&format!("/%63allback?code=forged-code&state={state}")),
            PasteRefused::NotThisAddress,
        ),
        (
            format!("http://[::1]:{port}/callback?code=forged-code&state={state}"),
            PasteRefused::NotThisAddress,
        ),
        (
            at(&format!("/callback#code=forged-code&state={state}")),
            PasteRefused::WrongState,
        ),
    ];
    let user = async {
        settle().await;
        for (text, want) in &cases {
            let refused = paste.submit(text).unwrap_err();
            assert_eq!(&refused, want, "{text}");
            let shown = refused.to_string();
            assert!(!shown.is_empty());
            assert!(
                !shown.contains("forged") && !shown.contains(&state),
                "{shown}"
            );
        }
        paste
            .submit(&format!(
                "  {}\n",
                at(&format!("/callback?code=c&state={state}"))
            ))
            .unwrap();
    };
    let (got, ()) = tokio::join!(
        pending.finish(&api, &secrets, &team_file, LONG, &cancel),
        user
    );
    assert_eq!(got.unwrap().token_id, "9");
    assert_eq!(hits(&s, "POST", REDEEM).await, 1);
    assert_eq!(redeem_bodies(&s).await[0]["code"], "c");
    assert!(port_closes(addr), "the listener is still open");
    // The wait is over, so even a valid paste is now too late.
    let again = at(&format!("/callback?code=c&state={state}"));
    assert_eq!(paste.submit(&again), Err(PasteRefused::Finished));
}

#[tokio::test]
async fn a_pasted_error_with_the_right_state_ends_the_wait_with_its_error() {
    let (s, dir, secrets) = fresh().await;
    let api = client(&s);
    let pending = team::begin_sso(&api).await.unwrap();
    let start = pending.url().to_owned();
    let (state, addr) = (param(&start, "state"), callback_addr(&start));
    let paste = pending.paste_handle();
    let text = format!(
        "http://127.0.0.1:{}/callback?error=account_not_linked&state={state}",
        addr.port()
    );
    let (cancel, team_file) = (CancelHandle::new(), file(&dir));
    let (got, sent) = tokio::join!(
        pending.finish(&api, &secrets, &team_file, LONG, &cancel),
        async {
            settle().await;
            paste.submit(&text)
        }
    );
    assert_eq!(sent, Ok(()));
    assert_eq!(got.unwrap_err(), TeamError::AccountNotLinked);
    assert_eq!(paste.submit(&text), Err(PasteRefused::Finished));
    assert_eq!(hits(&s, "POST", REDEEM).await, 0);
    nothing_kept(&s, &secrets, &file(&dir));
}

#[tokio::test]
async fn a_paste_after_a_timeout_is_finished() {
    let (s, dir, secrets) = fresh().await;
    let api = client(&s);
    let pending = team::begin_sso(&api).await.unwrap();
    let start = pending.url().to_owned();
    let (state, addr) = (param(&start, "state"), callback_addr(&start));
    let paste = pending.paste_handle();
    let short = Duration::from_millis(100);
    let got = pending
        .finish(&api, &secrets, &file(&dir), short, &CancelHandle::new())
        .await;
    assert_eq!(got.unwrap_err(), TeamError::SsoTimeout);
    let text = format!(
        "http://127.0.0.1:{}/callback?code=c&state={state}",
        addr.port()
    );
    assert_eq!(paste.submit(&text), Err(PasteRefused::Finished));
    // A handle outlives a sign-in that was dropped without finishing, and is finished too.
    let dropped = team::begin_sso(&api).await.unwrap();
    let late = dropped.paste_handle();
    let state = param(dropped.url(), "state");
    let port = callback_addr(dropped.url()).port();
    drop(dropped);
    let text = format!("http://127.0.0.1:{port}/callback?code=c&state={state}");
    assert_eq!(late.submit(&text), Err(PasteRefused::Finished));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_paste_racing_the_listener_redeems_exactly_one_code() {
    for _ in 0..5 {
        let (s, dir, secrets) = fresh().await;
        answer(&s, "POST", REDEEM, redeemed(9, EXPIRES)).await;
        let api = client(&s);
        let pending = team::begin_sso(&api).await.unwrap();
        let start = pending.url().to_owned();
        let (state, addr) = (param(&start, "state"), callback_addr(&start));
        let paste = pending.paste_handle();
        let text = format!(
            "http://127.0.0.1:{}/callback?code=from-paste&state={state}",
            addr.port()
        );
        let cancel = CancelHandle::new();
        let paster = tokio::spawn(async move {
            settle().await;
            paste.submit(&text)
        });
        let team_file = file(&dir);
        let (got, _) = tokio::join!(
            pending.finish(&api, &secrets, &team_file, LONG, &cancel),
            async {
                settle().await;
                land_quietly(addr, format!("code=from-listener&state={state}")).await
            }
        );
        let sent = paster.await.unwrap();
        assert_eq!(got.unwrap().token_id, "9");
        let bodies = redeem_bodies(&s).await;
        assert_eq!(bodies.len(), 1);
        let code = bodies[0]["code"].as_str().unwrap().to_owned();
        // A paste that reports success is the answer that was redeemed.
        match sent {
            Ok(()) => assert_eq!(code, "from-paste"),
            Err(PasteRefused::Finished) => assert_eq!(code, "from-listener"),
            Err(other) => panic!("unexpected refusal {other:?}"),
        }
        assert!(port_closes(addr), "the listener is still open");
    }
}

#[tokio::test]
async fn a_paste_after_the_listener_won_is_finished_and_only_the_browser_code_is_redeemed() {
    let (s, dir, secrets) = fresh().await;
    answer(&s, "POST", REDEEM, redeemed(9, EXPIRES)).await;
    let api = client(&s);
    let pending = team::begin_sso(&api).await.unwrap();
    let start = pending.url().to_owned();
    let (state, addr) = (param(&start, "state"), callback_addr(&start));
    let paste = pending.paste_handle();
    let (cancel, team_file) = (CancelHandle::new(), file(&dir));
    let (got, page) = tokio::join!(
        pending.finish(&api, &secrets, &team_file, LONG, &cancel),
        land(addr, format!("code=from-listener&state={state}"))
    );
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert_eq!(got.unwrap().token_id, "9");
    let text = format!(
        "http://127.0.0.1:{}/callback?code=from-paste&state={state}",
        addr.port()
    );
    assert_eq!(paste.submit(&text), Err(PasteRefused::Finished));
    let bodies = redeem_bodies(&s).await;
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["code"], "from-listener");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_browser_return_just_after_a_paste_is_not_redeemed() {
    let (s, dir, secrets) = fresh().await;
    answer(&s, "POST", REDEEM, redeemed(9, EXPIRES)).await;
    let api = client(&s);
    let pending = team::begin_sso(&api).await.unwrap();
    let start = pending.url().to_owned();
    let (state, addr) = (param(&start, "state"), callback_addr(&start));
    let paste = pending.paste_handle();
    let text = format!(
        "http://127.0.0.1:{}/callback?code=from-paste&state={state}",
        addr.port()
    );
    let (cancel, team_file) = (CancelHandle::new(), file(&dir));
    let (got, sent) = tokio::join!(
        pending.finish(&api, &secrets, &team_file, LONG, &cancel),
        async {
            settle().await;
            let sent = paste.submit(&text);
            land_quietly(addr, format!("code=from-listener&state={state}")).await;
            sent
        }
    );
    assert_eq!(sent, Ok(()));
    assert_eq!(got.unwrap().token_id, "9");
    let bodies = redeem_bodies(&s).await;
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["code"], "from-paste");
    assert!(port_closes(addr), "the listener is still open");
}
