//! The Keyboard Shortcuts dialog, driven pass by pass through `craft_ui::testing::Fixture`.
mod fixture;
pub(crate) mod import_offer;
pub(crate) mod lists;
pub(crate) mod recorder;
pub(crate) mod rows;
pub(crate) mod sets;
pub(crate) mod tabs;

use craft_keymap::Platform;
use craft_ui::testing::trial;

use fixture::Rig;

/// A read-only shortcuts reference does not stretch to the window's height: it opens at a moderate, fixed height and
/// scrolls inside it.
#[trial]
fn a_read_only_dialog_opens_at_a_moderate_height() {
    let rig = Rig::tuned(Platform::Windows, |options| options.editable = false);
    let close = rig.fixture.by_label("Close").expect("the close button");
    assert!(close.rect.bottom() < 560.0, "the dialog's footer sits within a moderate height, not the 700-point window: {:?}", close.rect);
}
