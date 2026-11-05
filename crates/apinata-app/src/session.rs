//! The window and collection session, stored in `session.json`.

use crate::tabs::TabKind;
use apinata_core::{ItemPath, StatusRule};
use gpui_kit::{Bounds, Pixels, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The most recently opened collections kept.
pub const RECENT_LIMIT: usize = 10;
/// The narrowest saved window that is restored.
pub const MIN_WIDTH: f32 = 320.0;
/// The shortest saved window that is restored.
pub const MIN_HEIGHT: f32 = 240.0;

/// What the app restores on the next start.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    /// Recently opened collection roots, newest first.
    pub recent: Vec<PathBuf>,
    /// Collection roots open when the session was saved.
    pub open: Vec<PathBuf>,
    /// The workspace folder open when the session was saved; older files lack it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<PathBuf>,
    /// The main window's placement.
    pub window: Option<SavedBounds>,
    /// The sidebar width in pixels.
    pub sidebar_width: Option<f32>,
    /// Per-collection state, keyed by root.
    pub collections: BTreeMap<PathBuf, CollectionSession>,
    /// The active global environment's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_environment: Option<String>,
}

impl Session {
    /// Moves `root` to the front of `recent`, dropping a duplicate and the oldest entry
    /// past [`RECENT_LIMIT`]. Returns `false` and changes nothing for a root that is not
    /// UTF-8, since the file stores paths as strings.
    pub fn remember(&mut self, root: &Path) -> bool {
        if root.to_str().is_none() {
            return false;
        }
        self.recent.retain(|p| p != root);
        self.recent.insert(0, root.to_path_buf());
        self.recent.truncate(RECENT_LIMIT);
        true
    }
}

/// One collection's restorable state.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CollectionSession {
    /// Open tabs as request paths relative to the root.
    #[serde(deserialize_with = "lenient::list")]
    pub tabs: Vec<String>,
    /// The selected environment's name.
    pub environment: Option<String>,
    /// The open tab that was the preview tab, as a path relative to the root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// Open example tabs in tab order; kept apart from `tabs` so older session files load.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "lenient::list"
    )]
    pub examples: Vec<SavedExampleTab>,
    /// Open settings, environments, runner and run-selection tabs with their tab position;
    /// kept apart from `tabs` so older session files load.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "lenient::list"
    )]
    pub panels: Vec<SavedPanel>,
    /// The window's active tab, kept only by the root it belongs to.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient::option"
    )]
    pub active: Option<SavedTab>,
    /// The pinned tabs, in tab order; older session files lack it and restore unpinned.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "lenient::list"
    )]
    pub pinned: Vec<SavedTab>,
    /// The history key of the active tab when it was a kept draft.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_draft: Option<String>,
    /// The sidebar's expanded folders as paths relative to the root; the empty path is the
    /// collection row. `None` means never saved, which keeps the default of an open collection
    /// row; `Some` replaces it, so a collapsed collection row is remembered. Older files lack it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<Vec<String>>,
    /// Each runner's remembered options, keyed by the runner's folder path relative to the
    /// root; the empty key is the whole collection. Older files lack it.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "lenient::map"
    )]
    pub runners: BTreeMap<String, SavedRunnerOptions>,
    /// Whether the collection's mock server runs; it starts again when the collection opens.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mock: bool,
}

/// The runner options that outlive a restart. Tags and exclude tags are left out on purpose:
/// a stale filter would make the next run silently send nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedRunnerOptions {
    /// Stop after the first failed request.
    pub bail: bool,
    /// The pause between requests, in milliseconds.
    pub delay_ms: u64,
    /// Extra attempts for a request that fails in transit.
    pub retries: u32,
    /// The pause before each retry, in milliseconds.
    pub retry_delay_ms: u64,
    /// Whether POST and PATCH are retried too.
    pub retry_unsafe: bool,
    /// Which reply statuses fail a request; `None` leaves the status unjudged.
    #[serde(with = "status_rule")]
    pub fail_on_status: Option<StatusRule>,
    /// The most iterations, if limited.
    pub iteration_count: Option<usize>,
    /// The data file iterated over, if any.
    pub data_file: Option<PathBuf>,
}

/// A saved tab that can be the active one: a request or an example tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedTab {
    /// A request, as a path relative to the root.
    Request(String),
    /// An example tab.
    Example(SavedExample),
    /// A settings, environments or runner tab.
    Panel(SavedPanelKind),
}

/// A tab that shows a panel rather than a request; its target is a path relative to the root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedPanelKind {
    /// The collection's settings.
    Collection,
    /// A folder's settings.
    Folder(String),
    /// The collection's environments.
    Environments,
    /// A runner; the empty path is the whole collection.
    Runner(String),
    /// A run-selection tab's members, in tab order.
    Selection(Vec<String>),
}

impl SavedPanelKind {
    /// The tab kind this saves, or `None` for a path that is not a valid item path.
    pub fn to_tab_kind(&self) -> Option<TabKind> {
        Some(match self {
            SavedPanelKind::Collection => TabKind::Collection,
            SavedPanelKind::Environments => TabKind::Environments,
            SavedPanelKind::Folder(p) => TabKind::Folder(ItemPath::new(p)?),
            SavedPanelKind::Runner(p) => TabKind::Runner(ItemPath::new(p)?),
            SavedPanelKind::Selection(items) => {
                TabKind::RunSelection(items.iter().filter_map(|i| ItemPath::new(i)).collect())
            }
        })
    }

    /// What `kind` saves as, or `None` for a tab that is not a remembered panel.
    pub fn from_tab_kind(kind: &TabKind) -> Option<SavedPanelKind> {
        Some(match kind {
            TabKind::Collection => SavedPanelKind::Collection,
            TabKind::Environments => SavedPanelKind::Environments,
            TabKind::Folder(p) => SavedPanelKind::Folder(p.as_str().to_owned()),
            TabKind::Runner(p) => SavedPanelKind::Runner(p.as_str().to_owned()),
            // Drafts come back under new keys, so only saved members are kept.
            TabKind::RunSelection(items) => {
                let saved: Vec<String> = (items.iter())
                    .filter(|i| !i.is_draft())
                    .map(|i| i.as_str().to_owned())
                    .collect();
                if saved.is_empty() {
                    return None;
                }
                SavedPanelKind::Selection(saved)
            }
            _ => return None,
        })
    }
}

impl CollectionSession {
    /// Every saved tab in saved tab order. Panels and examples with a position claim that
    /// index; requests fill the other slots in order, each followed by its examples saved
    /// without a position, and examples of an unsaved request go last.
    pub fn saved_order(&self) -> Vec<SavedTab> {
        let mut placed: Vec<(usize, SavedTab)> = (self.panels.iter())
            .map(|p| (p.position, SavedTab::Panel(p.kind.clone())))
            .chain(self.examples.iter().filter_map(|e| {
                e.position
                    .map(|at| (at, SavedTab::Example(e.example.clone())))
            }))
            .collect();
        placed.sort_by_key(|(at, _)| *at);
        let loose: Vec<&SavedExample> = (self.examples.iter())
            .filter(|e| e.position.is_none())
            .map(|e| &e.example)
            .collect();
        let flowing = self.tabs.iter().flat_map(|name| {
            let examples = (loose.iter()).filter(move |e| e.item == *name);
            std::iter::once(SavedTab::Request(name.clone()))
                .chain(examples.map(|e| SavedTab::Example((*e).clone())))
        });
        let mut placed = placed.into_iter().peekable();
        let mut order = Vec::new();
        for tab in flowing {
            while let Some((_, t)) = placed.next_if(|(at, _)| *at <= order.len()) {
                order.push(t);
            }
            order.push(tab);
        }
        order.extend(placed.map(|(_, t)| t));
        let orphans = loose.into_iter().filter(|e| !self.tabs.contains(&e.item));
        order.extend(orphans.map(|e| SavedTab::Example(e.clone())));
        order
    }
}

/// An open panel tab and where it sat among the root's saved tabs (requests, examples and
/// panels together, in tab order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedPanel {
    /// What the tab shows.
    pub kind: SavedPanelKind,
    /// Its index among the root's saved tabs.
    pub position: usize,
}

/// An open example tab: example `index` of the request at `item`, relative to the root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedExample {
    /// The request holding the example, as a path relative to the root.
    pub item: String,
    /// The example's position in the request's `examples`.
    pub index: usize,
}

/// An open example tab and where it sat among the root's saved tabs; files saved before the
/// position was kept restore the tab after its request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedExampleTab {
    /// The example the tab shows.
    #[serde(flatten)]
    pub example: SavedExample,
    /// Its index among the root's saved tabs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<usize>,
}

impl From<SavedExample> for SavedExampleTab {
    fn from(example: SavedExample) -> Self {
        SavedExampleTab {
            example,
            position: None,
        }
    }
}

/// A window's restore bounds and mode, in a form that serializes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedBounds {
    /// Left edge in pixels.
    pub x: f32,
    /// Top edge in pixels.
    pub y: f32,
    /// Width in pixels.
    pub width: f32,
    /// Height in pixels.
    pub height: f32,
    /// Windowed, maximized or fullscreen; a mode this build does not know restores windowed.
    #[serde(default, deserialize_with = "lenient::or_default")]
    pub mode: WindowMode,
}

/// How a window was shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    /// A normal window.
    #[default]
    Windowed,
    /// Maximized; the bounds are the restore size.
    Maximized,
    /// Fullscreen; the bounds are the restore size.
    Fullscreen,
}

impl SavedBounds {
    /// Captures a window's bounds and mode.
    pub fn from_window(bounds: WindowBounds) -> SavedBounds {
        let (mode, b) = match bounds {
            WindowBounds::Windowed(b) => (WindowMode::Windowed, b),
            WindowBounds::Maximized(b) => (WindowMode::Maximized, b),
            WindowBounds::Fullscreen(b) => (WindowMode::Fullscreen, b),
        };
        SavedBounds {
            x: f32::from(b.origin.x),
            y: f32::from(b.origin.y),
            width: f32::from(b.size.width),
            height: f32::from(b.size.height),
            mode,
        }
    }

    /// The bounds to restore, or `None` when they are smaller than the minimum or
    /// intersect none of `displays` (a monitor that has since been disconnected).
    pub fn to_window(&self, displays: &[Bounds<Pixels>]) -> Option<WindowBounds> {
        if self.width < MIN_WIDTH || self.height < MIN_HEIGHT {
            return None;
        }
        let b = Bounds::new(
            point(px(self.x), px(self.y)),
            size(px(self.width), px(self.height)),
        );
        if !displays.iter().any(|d| d.intersects(&b)) {
            return None;
        }
        Some(match self.mode {
            WindowMode::Windowed => WindowBounds::Windowed(b),
            WindowMode::Maximized => WindowBounds::Maximized(b),
            WindowMode::Fullscreen => WindowBounds::Fullscreen(b),
        })
    }
}

// Stores the rule by core's own token (`error`, `4xx`, `5xx`); a token this build does not
// know reads back as no rule, so a newer session file never blocks the app from starting.
mod status_rule {
    use apinata_core::StatusRule;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(rule: &Option<StatusRule>, s: S) -> Result<S::Ok, S::Error> {
        match rule {
            Some(r) => s.serialize_some(r.as_str()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<StatusRule>, D::Error> {
        let token = Option::<String>::deserialize(d)?;
        Ok(token.and_then(|t| t.parse().ok()))
    }
}

/// Entries a newer build may write in a shape this one does not know: each such entry is
/// dropped alone, or a lone value falls back to its default, instead of failing the session.
mod lenient {
    use serde::de::IgnoredAny;
    use serde::{Deserialize, Deserializer};
    use std::collections::BTreeMap;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Entry<T> {
        Known(T),
        Unknown(IgnoredAny),
    }

    impl<T> Entry<T> {
        fn known(self) -> Option<T> {
            match self {
                Entry::Known(value) => Some(value),
                Entry::Unknown(_) => None,
            }
        }
    }

    pub fn list<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        let entries = Vec::<Entry<T>>::deserialize(d)?;
        Ok(entries.into_iter().filter_map(Entry::known).collect())
    }

    pub fn option<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Ok(Option::<Entry<T>>::deserialize(d)?.and_then(Entry::known))
    }

    pub fn map<'de, D, K, T>(d: D) -> Result<BTreeMap<K, T>, D::Error>
    where
        D: Deserializer<'de>,
        K: Deserialize<'de> + Ord,
        T: Deserialize<'de>,
    {
        let entries = BTreeMap::<K, Entry<T>>::deserialize(d)?;
        let known = entries
            .into_iter()
            .filter_map(|(k, v)| Some((k, v.known()?)));
        Ok(known.collect())
    }

    pub fn or_default<'de, D, T>(d: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de> + Default,
    {
        Ok(Entry::<T>::deserialize(d)?.known().unwrap_or_default())
    }
}
