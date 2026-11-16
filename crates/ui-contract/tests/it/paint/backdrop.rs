//! Frosted backdrops: composition over paper, the blur, the limits and the upload to the texture store.

use craft_ui::primitives::backdrop::{BackdropLayer, MAX_SIDE, MAX_TEXELS, Thumbnail, compose, upload};
use craft_ui::primitives::{TextureStore, TextureStoreOptions};
use craft_ui::testing::trial;
use egui::{Color32, ColorImage, Context, Rect, TextureOptions};

fn area() -> Rect {
    Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(40.0, 20.0))
}

fn linear_store(ctx: &Context, salt: &str) -> TextureStore<u32> {
    TextureStore::in_context(ctx, salt, TextureStoreOptions::default().filter(TextureOptions::LINEAR))
}

#[trial]
fn transparent_capture_composites_over_paper_and_missing_capture_uses_fallback() {
    let image = ColorImage::new([2, 2], vec![Color32::TRANSPARENT; 4]);
    let thumbnail = Thumbnail::from_image(&image, 2, Color32::RED);
    assert!(thumbnail.is_some());
    let layers = [BackdropLayer::new(area(), thumbnail.as_ref(), Color32::BLUE)];
    let image = compose(area(), &layers, Color32::GREEN);
    assert!(image.as_ref().is_some_and(|image| image.pixels.iter().all(|p| *p == Color32::RED)));
    let layers = [BackdropLayer::new(area(), None, Color32::BLUE)];
    let image = compose(area(), &layers, Color32::GREEN);
    assert!(image.as_ref().is_some_and(|image| image.pixels.iter().all(|p| *p == Color32::BLUE)));
}

#[trial]
fn downsampling_and_softening_mix_neighbours_without_changing_opaque_output() {
    let image = ColorImage::new([2, 1], vec![Color32::BLACK, Color32::WHITE]);
    let thumbnail = Thumbnail::from_image(&image, 1, Color32::WHITE);
    let layers = [BackdropLayer::new(area(), thumbnail.as_ref(), Color32::RED)];
    let image = compose(area(), &layers, Color32::GREEN);
    assert!(image.as_ref().is_some_and(|image| image.pixels.iter().all(|p| p.a() == 255 && p.r() > 0 && p.r() < 255)));
}

#[trial]
fn malformed_dimensions_empty_images_and_large_allocations_are_bounded() {
    let malformed = ColorImage { size: [usize::MAX, 2], pixels: vec![], source_size: egui::Vec2::ZERO };
    assert!(Thumbnail::from_image(&malformed, usize::MAX, Color32::WHITE).is_none());
    assert!(Thumbnail::from_image(&ColorImage::new([0, 0], vec![]), 0, Color32::WHITE).is_none());
    assert!(compose(Rect::NAN, &[], Color32::WHITE).is_none());
    assert!(compose(Rect::NOTHING, &[], Color32::WHITE).is_none());
    let large = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1_000_000.0, 1_000_000.0));
    let image = compose(large, &[], Color32::WHITE);
    assert!(image.as_ref().is_some_and(|image| image.pixels.len() <= MAX_TEXELS && image.size.iter().all(|&side| side <= MAX_SIDE)));
}

#[trial]
fn texture_cache_updates_changed_content_but_skips_identical_uploads() {
    let ctx = Context::default();
    let store = linear_store(&ctx, "backdrop");
    let image = ColorImage::new([1, 1], vec![Color32::RED]);
    let mut first_id = None;
    let out = ctx.run_ui(egui::RawInput::default(), |_| {
        first_id = upload(&store, &ctx, 0, image.clone()).map(|texture| texture.id);
    });
    assert!(first_id.is_some());
    assert!(out.textures_delta.set.iter().any(|(id, _)| Some(*id) == first_id));
    out.drop_without_applying_deltas();
    let out = ctx.run_ui(egui::RawInput::default(), |_| {
        assert_eq!(upload(&store, &ctx, 0, image.clone()).map(|texture| texture.id), first_id);
    });
    assert!(out.textures_delta.set.iter().all(|(id, _)| Some(*id) != first_id));
    out.drop_without_applying_deltas();
    let out = ctx.run_ui(egui::RawInput::default(), |_| {
        let changed = ColorImage::new([1, 1], vec![Color32::BLUE]);
        assert_eq!(upload(&store, &ctx, 0, changed).map(|texture| texture.id), first_id);
    });
    assert_eq!(out.textures_delta.set.iter().filter(|(id, _)| Some(**id) == first_id).count(), 1);
    out.drop_without_applying_deltas();
}

#[trial]
fn texture_upload_rejects_long_thin_images_without_replacing_the_cache() {
    let ctx = Context::default();
    let store = linear_store(&ctx, "bounded");
    let image = ColorImage::new([1, 1], vec![Color32::RED]);
    let mut kept = None;
    ctx.run_ui(egui::RawInput::default(), |_| {
        kept = upload(&store, &ctx, 0, image.clone()).map(|texture| texture.id);
    })
    .drop_without_applying_deltas();
    assert!(kept.is_some());
    for size in [[MAX_SIDE + 1, 1], [1, MAX_SIDE + 1]] {
        let out = ctx.run_ui(egui::RawInput::default(), |_| {
            assert!(upload(&store, &ctx, 0, ColorImage::new(size, vec![Color32::BLUE; size[0] * size[1]])).is_none());
            assert_eq!(upload(&store, &ctx, 0, image.clone()).map(|texture| texture.id), kept);
        });
        assert!(out.textures_delta.set.iter().all(|(id, _)| Some(*id) != kept));
        out.drop_without_applying_deltas();
    }
}
