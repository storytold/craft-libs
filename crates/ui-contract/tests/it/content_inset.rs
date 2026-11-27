//! Regression coverage for content insets.

use craft_ui::layout::ContentInset;
use craft_ui::testing::trial;
use egui::UiBuilder;
use egui::{Context, RawInput, Rect, Vec2};

#[trial]
fn trailing_padding_separates_a_following_horizontal_sibling() {
    let ctx = Context::default();
    ctx.run_ui(RawInput::default(), |ui| {
        ui.horizontal(|ui| {
            let field = ContentInset::new(12.5).right(9.25).show(ui, |ui| ui.button("Field"));
            let next = ui.button("Next");
            assert!((field.response.rect.right() - field.inner.rect.right() - 9.25).abs() < 0.01);
            assert!(next.rect.left() >= field.inner.rect.right() + 9.25 + ui.spacing().item_spacing.x - 0.01);
        });
    })
    .drop_without_applying_deltas();
}

#[trial]
fn fractional_padding_keeps_fields_below_heading_and_following_rows_below_contents() {
    let ctx = Context::default();
    ctx.run_ui(RawInput::default(), |ui| {
        let heading = ui.label("Settings");
        let original_indent = ui.spacing().indent;
        let fields = ContentInset::new(12.5).right(4.25).show(ui, |ui| {
            assert!((ui.max_rect().left() - heading.rect.left() - 12.5).abs() < 0.01);
            ui.label("First");
            ui.label("Second");
            7
        });
        assert_eq!(fields.inner, 7);
        let footer = ui.label("Footer");
        assert!(footer.rect.top() >= fields.response.rect.bottom());
        assert_eq!(ui.spacing().indent, original_indent);
    })
    .drop_without_applying_deltas();
}

#[trial]
fn narrow_and_horizontal_parents_and_invalid_insets_do_not_invert_child_rectangles() {
    let ctx = Context::default();
    for value in [f32::NAN, f32::INFINITY, -1.0, 0.0, 12.5, f32::MAX] {
        ctx.run_ui(RawInput::default(), |ui| {
            ui.horizontal(|ui| {
                ui.scope_builder(UiBuilder::new().max_rect(Rect::from_min_size(ui.cursor().min, Vec2::new(5.0, 50.0))), |ui| {
                    ContentInset::new(value).right(value).show(ui, |ui| {
                        assert!(ui.max_rect().is_finite());
                        assert!(ui.max_rect().width() >= 0.0);
                    });
                });
            });
        })
        .drop_without_applying_deltas();
    }
}
