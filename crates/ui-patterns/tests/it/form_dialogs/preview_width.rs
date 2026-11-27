//! A picture in a dialog sized from its content needs no width of its own: the dialog does not follow the window,
//! and the picture spans the width a wider sibling sets. In a panel narrower than the plot's least width, it takes
//! the panel's room.

use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::dialog::Dialog;
use craft_ui::display::Text;
use craft_ui::docking::{DockGroup, PanelKey, PanelTitle, Titles};
use craft_ui::layout_components::Stack;
use craft_ui::patterns::form_dialogs::{PreviewPane, PreviewState, PreviewView};
use craft_ui::testing::snapshot::layer_rects;
use craft_ui::testing::{Fixture, trial};
use egui::{Pos2, Rect, Vec2, vec2};

/// The dialog's rect and the preview's, in a window of `room`; `wide` puts a field wider than the preview's least
/// width above it.
fn shown(room: Vec2, wide: bool) -> (Rect, Rect) {
    let mut fixture = Fixture::new(move |ui| {
        Dialog::new("Preview").show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                if wide {
                    Text::new("A caption that sets the column wider than a preview needs").show(ui);
                }
                PreviewPane::state("Preview", PreviewState::Pending(None), &PreviewView::fitting()).show(ui);
            });
        });
    });
    fixture.room(room);
    fixture.settle();
    let layers = layer_rects(fixture.ctx());
    assert_eq!(layers.len(), 1, "one dialog layer");
    (layers[0], fixture.by_label("Preview").expect("the preview").rect)
}

/// The preview's rect in a docked panel group `width` wide.
fn in_group(width: f32) -> Rect {
    let panels = vec![PanelTitle::new(PanelKey::new(1), "Flattening")];
    let mut fixture = Fixture::new(move |ui| {
        let group = DockGroup::new(1usize, Titles::from_content(panels.clone()), panels[0].key);
        group.show_at(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(width, 400.0)), |ui, _| {
            PreviewPane::state("Preview", PreviewState::Pending(None), &PreviewView::fitting()).show(ui);
        });
    });
    fixture.room(vec2(800.0, 600.0));
    fixture.settle();
    fixture.by_label("Preview").expect("the preview").rect
}

// A panel at its narrowest offers its body less room than the plot's least width; the preview takes that room and
// stays inside the panel. A wider panel's preview spans its body as before, wider than the preview of a dialog sized
// from its content, which takes the plot's least width.
#[trial]
fn a_preview_in_a_narrow_panel_stays_inside_it() {
    let narrow = 220.0;
    let preview = in_group(narrow);
    assert!(preview.right() <= 10.0 + narrow + 0.5, "the preview stays inside a panel {narrow} wide: {preview:?}");
    assert!(preview.width() > 0.0, "the preview still shows: {preview:?}");
    let (_, least) = shown(vec2(800.0, 600.0), false);
    let wide = in_group(500.0);
    assert!(wide.width() > least.width() + 1.0, "a wider panel's preview spans its body: {wide:?} vs {least:?}");
    assert!(wide.right() <= 510.5, "and stays inside it: {wide:?}");
}

#[trial]
fn a_preview_in_a_dialog_sized_from_its_content_sets_no_width() {
    let (small, large) = (vec2(800.0, 600.0), vec2(1600.0, 1000.0));
    let (dialog, preview) = shown(small, false);
    assert_eq!(dialog.width(), shown(large, false).0.width(), "the dialog's width does not follow the window's");
    assert!(dialog.width() < small.x * 0.8 - 1.0, "the dialog is narrower than its share of the window: {dialog:?}");
    let (wide_dialog, wide_preview) = shown(small, true);
    assert!(wide_preview.width() > preview.width() + 1.0, "the preview spans the wider column: {wide_preview:?} vs {preview:?}");
    assert_eq!(wide_dialog.width(), shown(large, true).0.width());
}
