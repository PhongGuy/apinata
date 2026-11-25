//! Release notes for an offered update: the `notes.json` format, the manifest's pointer to
//! it, and the cleaning and selection applied before anything is shown.

use semver::Version;
use serde::{Deserialize, Serialize};

use super::allowed;

/// The only `notes.json` schema this client reads.
pub const NOTES_SCHEMA: u32 = 1;

/// The longest text kept for any one notes string, in characters.
const TEXT_CAP: usize = 500;

/// A whole `notes.json` document.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseNotes {
    /// The format version; [`NOTES_SCHEMA`] when written by [`ReleaseNotes::new`].
    pub schema: u32,
    /// The releases, newest first.
    pub releases: Vec<NotesRelease>,
}

impl ReleaseNotes {
    /// A schema 1 document over `releases`.
    pub fn new(releases: Vec<NotesRelease>) -> Self {
        ReleaseNotes {
            schema: NOTES_SCHEMA,
            releases,
        }
    }
}

/// The notes for one release.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotesRelease {
    /// The release version.
    pub version: String,
    /// The release before this one; absent for the first release.
    #[serde(default)]
    pub previous: Option<String>,
    /// The release date, `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// An optional sentence introducing the release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    /// The changes, grouped.
    #[serde(default)]
    pub groups: Vec<NotesGroup>,
}

impl NotesRelease {
    /// The notes for `version`, following `previous`, without an intro.
    pub fn new(
        version: impl Into<String>,
        previous: Option<String>,
        date: Option<String>,
        groups: Vec<NotesGroup>,
    ) -> Self {
        NotesRelease {
            version: version.into(),
            previous,
            date,
            intro: None,
            groups,
        }
    }

    /// The same notes with `intro`.
    pub fn with_intro(mut self, intro: impl Into<String>) -> Self {
        self.intro = Some(intro.into());
        self
    }

    /// The heading a release is shown under: its version, then its date in parentheses.
    pub fn heading(&self) -> String {
        match &self.date {
            Some(date) => format!("{} ({date})", self.version),
            None => self.version.clone(),
        }
    }
}

/// What kind of change a group lists; an unknown or missing kind reads as `Other`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupKind {
    /// New features.
    Features,
    /// Bug fixes.
    Fixes,
    /// Usability changes.
    Usability,
    /// Performance work.
    Performance,
    /// Anything else.
    #[default]
    #[serde(other)]
    Other,
}

impl GroupKind {
    fn is_other(&self) -> bool {
        *self == GroupKind::Other
    }
}

/// One titled group of changes.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotesGroup {
    /// What kind of change it lists; `Other` is written as no kind, which reads back the same.
    #[serde(default, skip_serializing_if = "GroupKind::is_other")]
    pub kind: GroupKind,
    /// The English title shown above the items.
    pub title: String,
    /// The changes.
    #[serde(default)]
    pub items: Vec<NotesItem>,
}

impl NotesGroup {
    /// A group of `kind` titled `title`.
    pub fn new(kind: GroupKind, title: impl Into<String>, items: Vec<NotesItem>) -> Self {
        NotesGroup {
            kind,
            title: title.into(),
            items,
        }
    }
}

/// One change.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotesItem {
    /// The plain-text description.
    pub text: String,
    /// The pull request number, shown as text and never as a link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<u64>,
}

impl NotesItem {
    /// A change described by `text`, with no pull request.
    pub fn new(text: impl Into<String>) -> Self {
        NotesItem {
            text: text.into(),
            pr: None,
        }
    }

    /// The same change with pull request `pr`.
    pub fn with_pr(mut self, pr: u64) -> Self {
        self.pr = Some(pr);
        self
    }

    /// The item as one line of plain text: `text (#pr)`, or `text` alone.
    pub fn line(&self) -> String {
        match self.pr {
            Some(pr) => format!("{} (#{pr})", self.text),
            None => self.text.clone(),
        }
    }
}

/// The notes an offer shows: the releases newer than this build, newest first.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateNotes {
    /// The releases newer than the installed version, newest first, cleaned for display.
    pub releases: Vec<NotesRelease>,
    /// The list may not reach back to the installed version, so older notes live on the
    /// release page.
    pub older_not_listed: bool,
}

impl UpdateNotes {
    /// Notes over already selected `releases`.
    pub fn new(releases: Vec<NotesRelease>, older_not_listed: bool) -> Self {
        UpdateNotes {
            releases,
            older_not_listed,
        }
    }
}

/// The manifest's `notes_asset`, once it has passed the URL and checksum rules.
#[derive(Debug, Clone)]
pub(crate) struct NotesAsset {
    pub(crate) url: String,
    pub(crate) size: u64,
    pub(crate) sha256: String,
}

#[derive(Deserialize)]
struct RawAsset {
    url: String,
    size: u64,
    sha256: String,
}

/// Reads the manifest's `notes_asset` value. It is checked here rather than in the manifest
/// type so that a malformed one costs the notes, never the update.
pub(crate) fn notes_asset(value: serde_json::Value) -> Result<NotesAsset, String> {
    let raw: RawAsset =
        serde_json::from_value(value).map_err(|e| format!("notes_asset is malformed: {e}"))?;
    let url = url::Url::parse(&raw.url).map_err(|e| format!("notes URL {:?}: {e}", raw.url))?;
    if !allowed(&url) {
        return Err(format!("the notes URL must be https: {}", raw.url));
    }
    // A `latest/` URL follows whatever release is newest, so it could serve another
    // release's notes than the one the manifest signed.
    if url
        .path_segments()
        .is_some_and(|mut s| s.any(|seg| seg == "latest"))
    {
        return Err(format!("the notes URL is not versioned: {}", raw.url));
    }
    if !is_sha256(&raw.sha256) {
        return Err(format!(
            "notes sha256 {:?} is not 64 lowercase hex",
            raw.sha256
        ));
    }
    Ok(NotesAsset {
        url: raw.url,
        size: raw.size,
        sha256: raw.sha256,
    })
}

pub(crate) fn is_sha256(hex: &str) -> bool {
    hex.len() == 64
        && hex
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// Parses verified `notes.json` bytes and selects what an update from `current` shows.
pub(crate) fn select(bytes: &[u8], current: &str) -> Result<UpdateNotes, String> {
    let doc: ReleaseNotes =
        serde_json::from_slice(bytes).map_err(|e| format!("the notes are malformed: {e}"))?;
    if doc.schema != NOTES_SCHEMA {
        return Err(format!("the notes use unknown schema {}", doc.schema));
    }
    let current = Version::parse(current).map_err(|e| format!("current version: {e}"))?;
    let reaches = |v: &str| Version::parse(v).is_ok_and(|v| v.cmp_precedence(&current).is_le());

    let mut shown: Vec<(Version, NotesRelease)> = (doc.releases.into_iter())
        .map(clean_release)
        .filter_map(|r| Some((Version::parse(&r.version).ok()?, r)))
        .filter(|(v, _)| v.cmp_precedence(&current).is_gt())
        .collect();
    shown.sort_by(|(a, _), (b, _)| b.cmp_precedence(a));
    // The list reaches the installed version only when its oldest entry names a `previous`
    // that is not newer than it; no entry, or a missing or unreadable `previous`, cannot.
    let older_not_listed = !(shown.last())
        .and_then(|(_, oldest)| oldest.previous.as_deref())
        .is_some_and(reaches);
    Ok(UpdateNotes {
        releases: shown.into_iter().map(|(_, r)| r).collect(),
        older_not_listed,
    })
}

fn clean_release(r: NotesRelease) -> NotesRelease {
    let clean_opt = |s: Option<String>| s.map(|s| clean(&s)).filter(|s| !s.is_empty());
    NotesRelease {
        version: clean(&r.version),
        previous: clean_opt(r.previous),
        date: clean_opt(r.date),
        intro: clean_opt(r.intro),
        groups: (r.groups.into_iter())
            .map(|g| NotesGroup {
                kind: g.kind,
                title: clean(&g.title),
                items: (g.items.into_iter())
                    .map(|i| NotesItem {
                        text: clean(&i.text),
                        pr: i.pr,
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Removes C0 and C1 controls, bidirectional formatting and zero-width characters, turning
/// line breaks and tabs into spaces, then trims and keeps at most [`TEXT_CAP`] characters.
fn clean(text: &str) -> String {
    let kept: String = (text.chars())
        .filter_map(|c| match c {
            '\t' | '\n' | '\r' | '\u{2028}' | '\u{2029}' => Some(' '),
            c if c.is_control() || is_bidi(c) || is_zero_width(c) => None,
            c => Some(c),
        })
        .collect();
    kept.trim().chars().take(TEXT_CAP).collect()
}

fn is_bidi(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}')
}
