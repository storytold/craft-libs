//! The screenshot slot: one completion per slot, a bounded image, and a wake of the owning viewport only.

use std::sync::Arc;

use craft_ui::runtime::egui::screenshot::ScreenshotSlot;
use craft_ui::testing::trial;
use egui::{Color32, ColorImage, Context, ViewportId};

fn image(size: [usize; 2]) -> Arc<ColorImage> {
    Arc::new(ColorImage::filled(size, Color32::default()))
}

#[trial]
fn callback_clones_share_one_completion_and_take_is_single_use() {
    let ctx = Context::default();
    let (slot, callback) = ScreenshotSlot::detached(&ctx, ViewportId::ROOT, 1024);
    let duplicate = callback.clone();
    let first_image = image([1, 1]);
    let second_image = image([1, 1]);
    let weak_loser = Arc::downgrade(&second_image);

    callback.complete(first_image.clone());
    duplicate.complete(second_image);
    assert!(weak_loser.upgrade().is_none(), "the losing completion releases its image");

    let taken = slot.take().unwrap().unwrap();
    assert!(Arc::ptr_eq(&taken, &first_image), "the first completion wins");
    assert!(slot.take().is_none(), "the result is taken once");
}
