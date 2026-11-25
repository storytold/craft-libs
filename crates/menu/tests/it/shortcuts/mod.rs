//! The Keyboard Shortcuts dialog, driven pass by pass through `craft_ui::testing::Fixture`.
mod fixture;
pub(crate) mod hold;
pub(crate) mod import_offer;
pub(crate) mod lists;
pub(crate) mod recorder;
pub(crate) mod review;
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

/// The editable dialog's bounds, opened with `size` in the rig's 1000 by 700 point window.
fn dialog_bounds(size: Option<egui::Vec2>) -> egui::Rect {
    let rig = Rig::tuned(Platform::Windows, |options| options.size = size);
    rig.fixture.by_role(craft_ui::contract::Role::DIALOG).first().expect("the dialog").rect
}

/// An editable dialog opens at the size the app gives, larger or smaller than its own, and ignores a size that is not
/// finite.
#[trial]
fn an_editable_dialog_opens_at_the_size_the_app_gives() {
    let own = dialog_bounds(None).size();
    for asked in [own + egui::vec2(200.0, 120.0), egui::vec2(own.x + 80.0, own.y - 60.0)] {
        let opened = dialog_bounds(Some(asked)).size();
        assert!((opened - asked).abs().max_elem() < 1.0, "asked {asked:?}, opened {opened:?}, own {own:?}");
    }
    let endless = dialog_bounds(Some(egui::vec2(f32::INFINITY, f32::NAN))).size();
    assert_eq!(endless, own, "a size that is not finite opens the dialog as without one");
}
