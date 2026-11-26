//! The check dialog's choices in each state.

use super::*;
use craft_ui::testing::trial;

fn release() -> Release {
    Release { tag: "v2.0.0".into(), html_url: "https://example.org/releases/v2.0.0".into(), ..Release::default() }
}

/// A release the updater still holds after checks were turned off does not hide the Off
/// message: the dialog offers OK alone, with no release notes link.
#[trial]
fn checks_off_shows_the_off_state_over_a_held_release() {
    let mut dialog = UpdateCheckDialog::new("1.0.0").with_self_updating(true);
    dialog.set_available(Some(release()));
    assert_eq!(dialog.offered(true), vec![(CheckChoice::Update, DialogRole::Accept), (CheckChoice::CheckAgain, DialogRole::Other)]);
    dialog.set_status(Status::Off);
    assert_eq!(dialog.offered(true), vec![(CheckChoice::Close, DialogRole::Accept)]);
    assert!(dialog.notes_url().is_none());
    dialog.set_installing(true);
    assert!(dialog.answerable(CheckChoice::Close));
    assert_eq!(dialog.confirm(&Context::default()), Some(CheckChoice::Close));
}
