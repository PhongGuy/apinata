//! Icons beyond the kit's default set, and the asset source that serves both.

use std::borrow::Cow;

use gpui_kit::assets::{Assets, IconName};
use gpui_kit::{AssetSource, Result, SharedString};

// Private: the macro emits no doc comment, which `missing_docs` rejects on a public item.
gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        FilePlus,
        FolderPlus,
        GitBranch,
        Cookie,
        Columns2,
        Rows2,
        Send,
        Command,
        Import,
        Download,
        Bookmark,
        RotateCcwClock,
        CodeXml,
        Lock,
        LockOpen,
        SlidersHorizontal,
        FoldVertical,
        UnfoldVertical,
        Pin,
        GripVertical,
        FileOutput,
        Regex,
        WholeWord
    ]
);

/// The icons `AppAssets` adds to the kit's default set, in `ExtraIcons` order.
pub const EXTRA: [IconName; 23] = [
    IconName::FilePlus,
    IconName::FolderPlus,
    IconName::GitBranch,
    IconName::Cookie,
    IconName::Columns2,
    IconName::Rows2,
    IconName::Send,
    IconName::Command,
    IconName::Import,
    IconName::Download,
    IconName::Bookmark,
    IconName::RotateCcwClock,
    IconName::CodeXml,
    IconName::Lock,
    IconName::LockOpen,
    IconName::SlidersHorizontal,
    IconName::FoldVertical,
    IconName::UnfoldVertical,
    IconName::Pin,
    IconName::GripVertical,
    IconName::FileOutput,
    IconName::Regex,
    IconName::WholeWord,
];

/// The app's asset source: the extra icons first, then the kit's default `Assets`.
#[derive(Clone, Copy, Debug, Default)]
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        // An extra icon wins; anything else, including errors for unknown paths, is the kit's.
        match ExtraIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all = ExtraIcons.list(path)?;
        all.extend(Assets.list(path)?);
        Ok(all)
    }
}
