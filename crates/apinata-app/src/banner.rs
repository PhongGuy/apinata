//! The conflict banners an editor shows, and the commands each choice sends.

use gpui_kit::component::button::Button;
use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, Styled, TestSupportExt,
    Window, div,
};

use crate::command::{Command, Edit, Target};
use crate::snapshot::DiskState;

/// A conflict between the draft and the disk, or edits held back by a reload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Banner {
    /// The file changed on disk under a draft.
    ChangedOnDisk,
    /// The file is gone.
    DeletedOnDisk,
    /// The file no longer loads.
    BrokenOnDisk,
    /// This many edits were typed against a version a reload replaced.
    HeldEdits(usize),
}

/// The button a banner was answered with; `Primary` is the first one shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Choice {
    Primary,
    Secondary,
    /// The optional third button; today only Compare, which sends no command.
    Tertiary,
}

/// The banners for a document in `disk` state with `held` held edits, the disk one first.
pub fn banners(disk: DiskState, held: usize) -> Vec<Banner> {
    let disk = match disk {
        DiskState::Present => None,
        DiskState::Changed => Some(Banner::ChangedOnDisk),
        DiskState::Removed => Some(Banner::DeletedOnDisk),
        DiskState::Broken => Some(Banner::BrokenOnDisk),
    };
    let held = (held > 0).then_some(Banner::HeldEdits(held));
    disk.into_iter().chain(held).collect()
}

/// The commands that answer `banner` with `choice` for `target`; applying held edits
/// forgets them and then replays each of `held`, so a replay that a newer reload holds
/// again stays held.
pub fn commands(banner: Banner, choice: Choice, target: &Target, held: &[Edit]) -> Vec<Command> {
    let t = || target.clone();
    match (banner, choice) {
        (_, Choice::Tertiary) => Vec::new(),
        (Banner::ChangedOnDisk, Choice::Primary) => vec![Command::Revert(t())],
        (Banner::ChangedOnDisk, Choice::Secondary) => vec![Command::Overwrite(t())],
        (Banner::DeletedOnDisk, Choice::Primary) => vec![Command::Save(t())],
        (Banner::BrokenOnDisk, Choice::Primary) => vec![Command::Overwrite(t())],
        (Banner::DeletedOnDisk | Banner::BrokenOnDisk, Choice::Secondary) => {
            vec![Command::Revert(t())]
        }
        (Banner::HeldEdits(_), Choice::Primary) => [Command::DiscardStale(t())]
            .into_iter()
            .chain(held.iter().map(|edit| Command::Edit {
                target: t(),
                edit: edit.clone(),
            }))
            .collect(),
        (Banner::HeldEdits(_), Choice::Secondary) => vec![Command::DiscardStale(t())],
    }
}

/// One banner's row: its text and two buttons (three for a changed file, with Compare),
/// each calling `answer` with its choice.
pub fn banner_row(
    banner: Banner,
    answer: impl Fn(Banner, Choice, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let (id, text, primary, secondary) = match banner {
        Banner::ChangedOnDisk => (
            "banner-changed",
            "This file changed on disk.".to_string(),
            ("banner-reload", "Reload"),
            ("banner-overwrite", "Overwrite"),
        ),
        Banner::DeletedOnDisk => (
            "banner-deleted",
            "This file was deleted on disk.".to_string(),
            ("banner-save", "Save"),
            ("banner-discard", "Discard"),
        ),
        Banner::BrokenOnDisk => (
            "banner-broken",
            "This file no longer loads.".to_string(),
            ("banner-save", "Save"),
            ("banner-discard", "Discard"),
        ),
        Banner::HeldEdits(n) => (
            "banner-held",
            format!("{n} edit(s) were made against a version reloaded from disk."),
            ("banner-apply", "Apply mine"),
            ("banner-drop", "Discard"),
        ),
    };
    let button = |(bid, label): (&'static str, &'static str), choice| {
        let answer = answer.clone();
        Button::new(bid)
            .label(label)
            .on_click(move |_, window, cx| answer(banner, choice, window, cx))
    };
    h_flex()
        .id(id)
        .test_support()
        .gap_2()
        .w_full()
        .min_w_0()
        .child(div().flex_1().min_w_0().child(text))
        .child(button(primary, Choice::Primary))
        .child(button(secondary, Choice::Secondary))
        .when(banner == Banner::ChangedOnDisk, |row| {
            row.child(button(("banner-compare", "Compare"), Choice::Tertiary))
        })
        .into_any_element()
}
