//! The notice text of a failed sync.

use apinata_app::sync_text::{Direction, failure};
use apinata_core::SyncError;

#[test]
fn a_missing_identity_says_where_to_set_it() {
    for direction in [Direction::Pull, Direction::Push] {
        assert_eq!(
            failure(&SyncError::NoIdentity, direction),
            "Git needs your name and email before it can replay local commits. Commit once \
             from Changes to set them."
        );
    }
}
