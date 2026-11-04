//! The app's update check: at most one automatic check a day over a persisted state, a
//! manual check that reports its result, the offer the notice shows, and the release
//! prepared for install when the app quits.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apinata_core::update::{self, InstallKind, Product, Relaunch, Staged, UpdateError};
use gpui_kit::{App, AppContext, Context, Entity, EventEmitter, Task, actions};
use serde::{Deserialize, Serialize};

use crate::persisted::Persisted;

actions!(
    apinata,
    [
        /// Checks for a newer release now and reports the result.
        CheckForUpdates
    ]
);

/// When updates were last checked and which release the user chose to skip.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateState {
    /// The start of the last check, in unix seconds.
    pub last_check: Option<u64>,
    /// The release version the user skipped.
    pub skipped_version: Option<String>,
}

/// A newer release, as the notice presents it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// The release version.
    pub version: String,
    /// Plain-text release notes.
    pub notes: Option<String>,
    /// The page the release downloads from.
    pub download_page: String,
}

/// What a checker found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    /// This build is the newest release.
    UpToDate,
    /// A newer release exists.
    Available(Offer),
}

/// The outcome of a manual check, for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    /// This build is the newest release.
    UpToDate,
    /// This version is available.
    Available(String),
    /// The check failed with this message.
    Failed(String),
    /// This build has no update check.
    Disabled,
}

/// The sentence a manual check's `report` shows the user, in a toast or in Preferences.
pub fn report_text(report: &Report) -> String {
    match report {
        Report::UpToDate => format!("Apinata {} is up to date.", env!("CARGO_PKG_VERSION")),
        Report::Available(version) => format!("Apinata {version} is available."),
        Report::Failed(message) => format!("Update check failed: {message}"),
        Report::Disabled => "Update checks are not configured for this build".to_string(),
    }
}

/// Emitted by [`Updates`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdatesEvent {
    /// A manual check finished with this report.
    Reported(Report),
}

/// Runs one check; the error is a message for the user.
pub type Checker = Arc<dyn Fn(&mut App) -> Task<Result<CheckResult, String>>>;

/// Downloads and verifies the offered `version` for this install.
pub type Stager = Arc<dyn Fn(&mut App, String) -> Task<Result<Prepared, String>>>;

/// Where installing the offered release stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallPhase {
    /// Nothing is being prepared.
    Idle,
    /// The release is downloading.
    Downloading,
    /// The release is verified and waits for the quit.
    Ready,
    /// Preparing failed with this message.
    Failed(String),
}

/// A verified release waiting to be installed when the app quits.
#[derive(Debug)]
pub struct Prepared {
    version: String,
    how: Install,
}

#[derive(Debug)]
enum Install {
    Staged(InstallKind, Staged),
    Only(Relaunch),
}

impl Prepared {
    /// The release `staged` for the install `kind`.
    pub fn new(version: impl Into<String>, kind: InstallKind, staged: Staged) -> Self {
        Prepared {
            version: version.into(),
            how: Install::Staged(kind, staged),
        }
    }

    /// Installs nothing and relaunches as given; for tests.
    pub fn relaunch_only(version: impl Into<String>, relaunch: Relaunch) -> Self {
        Prepared {
            version: version.into(),
            how: Install::Only(relaunch),
        }
    }

    /// The release version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// `update::install_app` for a real release; `Ok(relaunch)` for `relaunch_only`.
    pub fn install(self) -> Result<Relaunch, UpdateError> {
        match self.how {
            Install::Staged(kind, staged) => update::install_app(&kind, staged),
            Install::Only(relaunch) => Ok(relaunch),
        }
    }

    // The release holds `version` throughout, including the staged file inside it.
    fn holds(&self, version: &str) -> bool {
        self.version == version
            && match &self.how {
                Install::Staged(_, staged) => staged.version == version,
                Install::Only(_) => true,
            }
    }

    // A release that will not be installed leaves no staged file behind; failure is harmless.
    fn discard(self) {
        if let Install::Staged(_, staged) = self.how {
            let _ = std::fs::remove_file(&staged.path);
        }
    }
}

/// The least time between automatic checks.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// The update check's state, its checker and the offer currently shown.
pub struct Updates {
    state: Entity<Persisted<UpdateState>>,
    checker: Option<Checker>,
    offer: Option<Offer>,
    report: Option<Report>,
    notes_shown: bool,
    in_flight: Option<Task<()>>,
    stager: Option<Stager>,
    phase: InstallPhase,
    prepared: Option<Prepared>,
    armed: bool,
    preparing: Option<Task<()>>,
}

impl EventEmitter<UpdatesEvent> for Updates {}

impl Updates {
    /// Updates over `state`; without a checker they are disabled.
    pub fn new(state: Entity<Persisted<UpdateState>>, checker: Option<Checker>) -> Self {
        Updates {
            state,
            checker,
            offer: None,
            report: None,
            notes_shown: false,
            in_flight: None,
            stager: None,
            phase: InstallPhase::Idle,
            prepared: None,
            armed: false,
            preparing: None,
        }
    }

    /// The real checker over `build_config(Product::App)`; `None` when updates are Disabled.
    /// A configuration that is present but invalid gives a checker that reports why.
    pub fn core_checker() -> Option<Checker> {
        let cfg = match update::build_config(Product::App) {
            Ok(cfg) => cfg,
            Err(UpdateError::Disabled) => return None,
            Err(e) => {
                let message = e.to_string();
                return Some(Arc::new(move |_| Task::ready(Err(message.clone()))));
            }
        };
        Some(Arc::new(move |cx: &mut App| {
            let cfg = cfg.clone();
            let download_page = cfg.download_page.clone();
            // reqwest needs the tokio runtime, so the check runs there and is awaited here.
            let job = crate::tokio_bridge::spawn(cx, async move { update::check(&cfg).await });
            cx.background_spawn(async move {
                match job.await {
                    Ok(Ok(Some(a))) => Ok(CheckResult::Available(Offer {
                        version: a.version,
                        notes: a.notes,
                        download_page,
                    })),
                    Ok(Ok(None)) => Ok(CheckResult::UpToDate),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(e) => Err(e.to_string()),
                }
            })
        }))
    }

    /// Replaces the checker; `None` disables updates.
    pub fn set_checker(&mut self, checker: Option<Checker>, cx: &mut Context<Self>) {
        self.checker = checker;
        cx.notify();
    }

    /// Whether this build can check for updates.
    pub fn enabled(&self) -> bool {
        self.checker.is_some()
    }

    /// The persisted check state.
    pub fn state(&self) -> &Entity<Persisted<UpdateState>> {
        &self.state
    }

    /// The offer the notice shows, if any.
    pub fn offer(&self) -> Option<&Offer> {
        self.offer.as_ref()
    }

    /// The outcome of the last manual check.
    pub fn last_report(&self) -> Option<&Report> {
        self.report.as_ref()
    }

    /// Automatic: only when `enabled_setting`, a checker exists, and `last_check` is more than
    /// CHECK_INTERVAL before `now`. Records `last_check = now`; hides skipped versions; errors are logged.
    pub fn check_if_due(&mut self, enabled_setting: bool, now: SystemTime, cx: &mut Context<Self>) {
        if !enabled_setting || self.checker.is_none() {
            return;
        }
        let now = unix_secs(now);
        // A last check in the future means the clock moved back, so it does not block one now.
        let due = match self.state.read(cx).get().last_check {
            Some(last) => now
                .checked_sub(last)
                .is_none_or(|since| since > CHECK_INTERVAL.as_secs()),
            None => true,
        };
        if due {
            self.start(false, now, cx);
        }
    }

    /// Manual: ignores the setting and the skip; records `last_check`; sets the report and emits it.
    pub fn check_now(&mut self, cx: &mut Context<Self>) {
        if self.checker.is_none() {
            self.finish_report(Report::Disabled, cx);
            return;
        }
        self.start(true, unix_secs(SystemTime::now()), cx);
    }

    /// Whether the notice shows the offer's release notes.
    pub fn notes_shown(&self) -> bool {
        self.notes_shown
    }

    /// Shows or hides the offer's release notes.
    pub fn toggle_notes(&mut self, cx: &mut Context<Self>) {
        self.notes_shown = !self.notes_shown;
        cx.notify();
    }

    /// Hides the offer for this run.
    pub fn later(&mut self, cx: &mut Context<Self>) {
        self.set_offer(None);
        cx.notify();
    }

    /// Records the offered version as skipped and hides it.
    pub fn skip(&mut self, cx: &mut Context<Self>) {
        if let Some(offer) = self.offer.take() {
            self.notes_shown = false;
            self.state.update(cx, |p, cx| {
                p.update(cx, |s| s.skipped_version = Some(offer.version))
            });
        }
        cx.notify();
    }

    /// Replaces the stager; `None` means this install cannot update in place.
    pub fn set_stager(&mut self, stager: Option<Stager>, cx: &mut Context<Self>) {
        self.stager = stager;
        cx.notify();
    }

    /// A stager is installed and an offer is showing.
    pub fn can_install(&self) -> bool {
        self.stager.is_some() && self.offer.is_some()
    }

    /// Where installing the offered release stands.
    pub fn install_phase(&self) -> &InstallPhase {
        &self.phase
    }

    /// Prepares the offered release, or reuses the one prepared for it, then calls `on_done`
    /// (deferred, never inside this update) with whether it is ready.
    pub fn prepare(
        &mut self,
        on_done: impl FnOnce(bool, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let (Some(stager), Some(offer)) = (self.stager.clone(), self.offer.as_ref()) else {
            cx.defer(move |cx| on_done(false, cx));
            return;
        };
        if self.phase == InstallPhase::Downloading {
            cx.defer(move |cx| on_done(false, cx));
            return;
        }
        let version = offer.version.clone();
        if self.prepared.as_ref().is_some_and(|p| p.version == version) {
            self.phase = InstallPhase::Ready;
            cx.notify();
            cx.defer(move |cx| on_done(true, cx));
            return;
        }
        // A release prepared for an older offer is stale; the new offer is fetched afresh.
        self.armed = false;
        if let Some(stale) = self.prepared.take() {
            stale.discard();
        }
        self.phase = InstallPhase::Downloading;
        cx.notify();
        let job = stager(cx, version.clone());
        self.preparing = Some(cx.spawn(async move |this, cx| {
            let result = job.await;
            let ok = this
                .update(cx, |u, cx| u.finish_prepare(&version, result, cx))
                .unwrap_or(false);
            cx.update(|cx| on_done(ok, cx));
        }));
    }

    /// The release prepared for install, if any.
    pub fn prepared(&self) -> Option<&Prepared> {
        self.prepared.as_ref()
    }

    /// Marks the prepared release for install at the next quit, or clears the mark.
    pub fn set_armed(&mut self, armed: bool) {
        self.armed = armed;
    }

    /// A release is prepared and marked for install.
    pub fn armed(&self) -> bool {
        self.armed && self.prepared.is_some()
    }

    /// The prepared release when armed; clears both.
    pub fn take_armed(&mut self) -> Option<Prepared> {
        if !self.armed() {
            return None;
        }
        self.armed = false;
        self.phase = InstallPhase::Idle;
        self.prepared.take()
    }

    // Only a release for the version asked for, which is still the one offered, is kept;
    // anything else would install a release the notice no longer shows.
    fn finish_prepare(
        &mut self,
        requested: &str,
        result: Result<Prepared, String>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.preparing = None;
        let offered = self.offer.as_ref().map(|o| o.version.as_str());
        let ok = match result {
            Ok(prepared) if !prepared.holds(requested) || offered != Some(requested) => {
                prepared.discard();
                self.phase = InstallPhase::Failed(format!(
                    "the downloaded release is not version {}",
                    offered.unwrap_or(requested)
                ));
                false
            }
            Ok(prepared) => {
                self.prepared = Some(prepared);
                self.phase = InstallPhase::Ready;
                true
            }
            Err(message) => {
                self.phase = InstallPhase::Failed(message);
                false
            }
        };
        cx.notify();
        ok
    }

    // The time is recorded before the check runs, so a failing server is tried once a day.
    // A check already in flight makes this a no-op.
    fn start(&mut self, manual: bool, now: u64, cx: &mut Context<Self>) {
        let Some(checker) = self.checker.clone() else {
            return;
        };
        if self.in_flight.is_some() {
            return;
        }
        self.state
            .update(cx, |p, cx| p.update(cx, |s| s.last_check = Some(now)));
        let job = checker(cx);
        self.in_flight = Some(cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |u, cx| u.finish(manual, result, cx));
        }));
    }

    fn finish(
        &mut self,
        manual: bool,
        result: Result<CheckResult, String>,
        cx: &mut Context<Self>,
    ) {
        self.in_flight = None;
        match (result, manual) {
            (Ok(CheckResult::UpToDate), true) => {
                self.set_offer(None);
                self.finish_report(Report::UpToDate, cx);
            }
            (Ok(CheckResult::Available(offer)), true) => {
                let report = Report::Available(offer.version.clone());
                self.set_offer(Some(offer));
                self.finish_report(report, cx);
            }
            (Err(e), true) => self.finish_report(Report::Failed(e), cx),
            (Ok(CheckResult::UpToDate), false) => self.set_offer(None),
            (Ok(CheckResult::Available(offer)), false) => {
                let skipped = self.state.read(cx).get().skipped_version.as_deref();
                if skipped != Some(offer.version.as_str()) {
                    self.set_offer(Some(offer));
                }
            }
            (Err(e), false) => eprintln!("apinata: update check: {e}"),
        }
        cx.notify();
    }

    // Each offer starts with its notes folded away, and a different version disarms the
    // prepared release so it is installed only after a fresh press.
    fn set_offer(&mut self, offer: Option<Offer>) {
        if let (Some(new), Some(old)) = (&offer, &self.offer)
            && new.version != old.version
        {
            self.armed = false;
        }
        self.offer = offer;
        self.notes_shown = false;
    }

    fn finish_report(&mut self, report: Report, cx: &mut Context<Self>) {
        self.report = Some(report.clone());
        cx.emit(UpdatesEvent::Reported(report));
        cx.notify();
    }
}

fn unix_secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}
