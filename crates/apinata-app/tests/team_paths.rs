//! Where the team file lives.

use crate::support::*;
use apinata_app::app::AppState;
use apinata_app::persist::Paths;
use gpui_kit::TestAppContext;
use std::path::Path;

#[test]
fn a_profile_dir_keeps_the_team_file_beside_the_session() {
    let paths = Paths::in_dir(Path::new("/p"));
    assert_eq!(paths.team, Path::new("/p/team.json"));
}

#[gpui_kit::test]
fn the_team_file_sits_in_the_data_directory(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    let team = cx.update(|cx| cx.global::<AppState>().team.clone());
    assert_eq!(team, s.files.path().join("team.json"));
}

#[gpui_kit::test]
fn the_conflict_copies_dir_is_reachable_from_a_dialog(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    let conflicts = cx.update(|cx| cx.global::<AppState>().conflicts.clone());
    assert_eq!(conflicts, s.files.path().join("conflicts"));
}
