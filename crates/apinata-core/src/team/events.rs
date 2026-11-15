//! The team server's change notices: an SSE stream of ref updates, followed with bounded
//! reconnect backoff and a polling fallback while the stream is down.

use std::time::Duration;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use reqwest::redirect::Policy;
use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;
use tokio::time::{Instant, Interval, MissedTickBehavior, interval_at};

use super::auth::TeamAuth;
use crate::sse::SseParser;

/// Where and how to follow a workspace's change notices.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EventsConfig {
    url: String,
    auth: TeamAuth,
    branch: String,
    first: Duration,
    max: Duration,
    poll_every: Duration,
    idle_limit: Duration,
}

impl EventsConfig {
    /// Backoff 1 s doubling to 60 s, a poll every 60 s while down, 45 s idle limit (three keep-alives).
    pub fn new(
        events_url: impl Into<String>,
        auth: TeamAuth,
        branch: impl Into<String>,
    ) -> EventsConfig {
        EventsConfig {
            url: events_url.into(),
            auth,
            branch: branch.into(),
            first: Duration::from_secs(1),
            max: Duration::from_secs(60),
            poll_every: Duration::from_secs(60),
            idle_limit: Duration::from_secs(45),
        }
    }

    /// The same config with other timings, for tests.
    #[doc(hidden)]
    pub fn timings(
        self,
        first: Duration,
        max: Duration,
        poll_every: Duration,
        idle_limit: Duration,
    ) -> EventsConfig {
        EventsConfig {
            first,
            max,
            poll_every,
            idle_limit,
            ..self
        }
    }
}

/// What the notice stream tells the sync loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TeamNotice {
    /// The stream is open.
    Connected,
    /// The followed branch moved to `sha`.
    Ref {
        /// The commit the branch now points at.
        sha: String,
    },
    /// The stream was lost or could not be opened; polls follow until it is back.
    Disconnected,
    /// The stream is still down: time to check the remote by hand.
    Poll,
}

/// Why [`follow_events`] stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EventsEnd {
    /// The server refused the token (401).
    SignedOut,
    /// The workspace is not there (404).
    Gone,
}

/// One `ref` event's data.
#[derive(Deserialize)]
struct RefData {
    #[serde(rename = "ref")]
    name: String,
    sha: String,
}

/// How one connection attempt went.
enum Attempt {
    Open(Response),
    Failed,
    End(EventsEnd),
}

/// Follows `GET <events_url>` with the Bearer header until a 401 or 404; never returns otherwise.
pub async fn follow_events(
    config: EventsConfig,
    mut on_notice: impl FnMut(TeamNotice) + Send,
) -> EventsEnd {
    // A client that cannot be built fails every attempt, so the loop falls back to polling.
    // Each attempt opens a fresh connection: one a failed attempt left idle may never answer.
    let client = Client::builder()
        .redirect(Policy::none())
        .pool_max_idle_per_host(0)
        .connect_timeout(Duration::from_secs(10))
        .build()
        .ok();
    let mut rng = StdRng::from_os_rng();
    let wanted = format!("refs/heads/{}", config.branch);
    let mut attempt: u32 = 0;
    // Set while down: the poll period runs across every wait until a stream delivers again.
    let mut polls: Option<Interval> = None;
    loop {
        let tried = polling(
            connect(client.as_ref(), &config),
            polls.as_mut(),
            &mut on_notice,
        );
        match tried.await {
            Attempt::End(end) => return end,
            // Only a stream that delivered counts as a connect, so an empty 200 keeps backing off.
            Attempt::Open(res) => {
                if read_stream(res, &config, &wanted, &mut on_notice).await {
                    attempt = 0;
                    polls = None;
                }
            }
            Attempt::Failed => {}
        }
        let polls = polls.get_or_insert_with(|| {
            on_notice(TeamNotice::Disconnected);
            let mut every = interval_at(Instant::now() + config.poll_every, config.poll_every);
            every.set_missed_tick_behavior(MissedTickBehavior::Delay);
            every
        });
        let delay = reconnect_delay(attempt, config.first, config.max, rng.random::<f64>());
        attempt = attempt.saturating_add(1);
        polling(tokio::time::sleep(delay), Some(polls), &mut on_notice).await;
    }
}

async fn connect(client: Option<&Client>, config: &EventsConfig) -> Attempt {
    let Some(client) = client else {
        return Attempt::Failed;
    };
    // A server that accepts but never sends a head is as dead as a silent stream, so the head
    // gets the same idle limit and its expiry is a failed attempt that backs off.
    let sent = tokio::time::timeout(
        config.idle_limit,
        client
            .get(&config.url)
            .bearer_auth(config.auth.token())
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send(),
    )
    .await;
    // Transport errors are dropped unread: their text could carry the request's header.
    let Ok(Ok(res)) = sent else {
        return Attempt::Failed;
    };
    match res.status() {
        StatusCode::UNAUTHORIZED => Attempt::End(EventsEnd::SignedOut),
        StatusCode::NOT_FOUND => Attempt::End(EventsEnd::Gone),
        status if status.is_success() => Attempt::Open(res),
        _ => Attempt::Failed,
    }
}

/// Reports Connected on the first chunk, then the branch's ref events, until the stream ends,
/// fails, or stays silent past the idle limit; true when any chunk came. The server's `retry:`
/// field is ignored; the backoff is fixed.
async fn read_stream(
    mut res: Response,
    config: &EventsConfig,
    wanted: &str,
    on_notice: &mut impl FnMut(TeamNotice),
) -> bool {
    let mut parser = SseParser::new();
    let mut delivered = false;
    while let Ok(Ok(Some(chunk))) = tokio::time::timeout(config.idle_limit, res.chunk()).await {
        if !delivered {
            delivered = true;
            on_notice(TeamNotice::Connected);
        }
        for event in parser.push(&chunk) {
            if event.event.as_deref() != Some("ref") {
                continue;
            }
            if let Ok(data) = serde_json::from_str::<RefData>(&event.data)
                && data.name == wanted
            {
                on_notice(TeamNotice::Ref { sha: data.sha });
            }
        }
    }
    delivered
}

/// Runs `work` to its end, emitting a Poll on each tick of `polls` meanwhile.
async fn polling<T>(
    work: impl Future<Output = T>,
    polls: Option<&mut Interval>,
    on_notice: &mut impl FnMut(TeamNotice),
) -> T {
    let Some(polls) = polls else {
        return work.await;
    };
    tokio::pin!(work);
    loop {
        tokio::select! {
            out = &mut work => return out,
            _ = polls.tick() => on_notice(TeamNotice::Poll),
        }
    }
}

/// min(first * 2^attempt, max) scaled by 0.5 + roll / 2, for `roll` in 0..=1.
#[doc(hidden)]
pub fn reconnect_delay(attempt: u32, first: Duration, max: Duration, roll: f64) -> Duration {
    let base = 2u32
        .checked_pow(attempt)
        .and_then(|factor| first.checked_mul(factor))
        .map_or(max, |d| d.min(max));
    // A roll that is not a number would panic the scaling; it takes the full delay.
    let roll = if roll.is_nan() {
        1.0
    } else {
        roll.clamp(0.0, 1.0)
    };
    base.mul_f64(0.5 + roll / 2.0)
}
