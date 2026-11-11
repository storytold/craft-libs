//! The label step: scrub across a pointer warp and the label beside or above a fill-width control.

mod align;
pub mod group;
pub mod kit;

use self::kit::{Dial, Field, Seen, Wide};
use craft_host::target::ShellInfo;
use craft_ui::contract::{Props as _, Show as _, ShowContainer as _, SideLabel};
use craft_ui::runtime::InstallSpec;
use craft_ui::testing::pointer::{moved, press};
use craft_ui::testing::windows::FakeSystem;
use craft_ui::testing::{Frames, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;
use std::sync::Arc;

/// The label drag of the scrub trial, in points.
const DRAG: f32 = 30.0;
/// The value the scrub trial starts from.
const START: f64 = 10.0;
/// How far right of the row start a label beside its control begins at least.
const BESIDE: f32 = 20.0;
/// How far below the row start a stacked label begins at least.
const STACKED: f32 = 5.0;
const ROW_A: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(310.0, 40.0) };

/// The window's right edge as the scrub trial's pointer reaches it, and the point the warp puts it at.
const EDGE_X: f32 = 799.0;
const LANDED_X: f32 = 4.0;
/// How far two values may differ and still count as the same.
const SAME: f64 = 0.01;

#[trial]
fn a_wrapped_scrub_keeps_its_value_continuous() {
    let system = Arc::new(FakeSystem::new());
    let mut frames = Frames::uninstalled();
    let installed = craft_ui::install(&frames.ctx, InstallSpec::new(ShellInfo::default(), system.clone()));
    assert!(installed.is_ok(), "the context takes the runtime once");
    let side = SideLabel::new().scrub(true);
    let seen = RefCell::new(Seen::default());
    let mut value = START;
    let show = |ui: &mut Ui, value: &mut f64| {
        Field::new("Opacity", side.clone()).id_salt("warp").show_at(ui, ROW_A, |ui, _| {
            Dial::new(value, &seen).warping().show(ui);
        });
    };
    frames.idle(3, |ui| show(ui, &mut value));
    let label = seen.borrow().label.clone().map(|label| label.rect.center());
    let from = label.unwrap_or_else(|| panic!("the dial ran inside a label scope"));
    let step = |frames: &mut Frames, value: &mut f64, events: Vec<egui::Event>, at: Pos2| {
        system.set_pointer(Some([at.x, at.y]));
        frames.frame(events, |ui| show(ui, value));
    };
    step(&mut frames, &mut value, vec![moved(from)], from);
    step(&mut frames, &mut value, vec![press(from)], from);
    step(&mut frames, &mut value, vec![moved(from + Vec2::new(20.0, 0.0))], from + Vec2::new(20.0, 0.0));
    let edge = Pos2::new(EDGE_X, from.y);
    step(&mut frames, &mut value, vec![moved(edge)], edge);
    assert_eq!(system.warps().len(), 1, "reaching the edge warps the pointer once");
    assert_eq!(frames.cursor_icon(), egui::CursorIcon::None, "the scrub hides the pointer");
    let before = value;
    let landed = Pos2::new(LANDED_X, from.y);
    step(&mut frames, &mut value, vec![moved(landed)], landed);
    assert!((value - before).abs() < SAME, "the warp does not move the value: {before} -> {value}");
    let on = landed + Vec2::new(DRAG, 0.0);
    step(&mut frames, &mut value, vec![moved(on)], on);
    assert!((value - before - f64::from(DRAG)).abs() < SAME, "the drag goes on from the value before the warp: {before} -> {value}");
    assert_eq!(system.warps().len(), 1, "the pointer far from an edge does not warp again");
}

#[trial]
fn a_label_stays_beside_a_fill_width_control_and_follows_the_row_width() {
    let mut frames = Frames::new();
    let rect = std::cell::Cell::new(Rect::NOTHING);
    let at = |frames: &mut Frames, width: f32, count: usize| {
        let row = Rect::from_min_size(ROW_A.min, Vec2::new(width, 60.0));
        frames.idle(count, |ui| {
            Field::new("Opacity", SideLabel::new()).id_salt("fill").show_at(ui, row, |ui, _| {
                rect.set(Wide::new().show(ui).rect);
            });
        });
        rect.get()
    };
    let wide = at(&mut frames, 300.0, 6);
    assert!(wide.min.x > ROW_A.min.x + BESIDE, "a wide row keeps the label beside a control that fills its width: {wide:?}");
    let narrow = at(&mut frames, 80.0, 4);
    assert!(narrow.min.y > ROW_A.min.y + STACKED, "a row narrower than label, gap and the control's minimum stacks the label: {narrow:?}");
    let again = at(&mut frames, 300.0, 4);
    assert!(again.min.x > ROW_A.min.x + BESIDE, "the label returns beside the control when the row widens: {again:?}");
}
