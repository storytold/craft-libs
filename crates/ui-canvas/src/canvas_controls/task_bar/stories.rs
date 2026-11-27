//! Stories of `TaskBar`.

use super::{TaskBar, TaskBarDismiss, TaskBarGrip, TaskBarHang, TaskBarPlace};
use crate::buttons::{Button, IconButton};
use crate::canvas::testing::canvas_story::{StoryDocument, on_canvas};
use crate::contract::{Gap, Props as _, Show as _};
use crate::menu::PanelRow;
use crate::runtime::commands::{CommandRef, EditCommands};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::IconId;
use craft_behavior::kurbo::{Point, Vec2};
use craft_behavior::overlay::{Align, Side};
use egui::Ui;

/// The corners of a crop frame turned by a few degrees about the document's middle: the Crop tool hangs its bar from
/// them, so the bar sits under the frame's lowest corner.
fn turned_frame() -> [Point; 4] {
    let centre = StoryDocument::STANDARD.centre();
    let (sin, cos) = 0.2_f64.sin_cos();
    [(-220.0, -130.0), (220.0, -130.0), (220.0, 130.0), (-220.0, 130.0)].map(|(x, y)| centre + Vec2::new(x * cos - y * sin, x * sin + y * cos))
}

/// A strip of document at its top edge, where a bar above it has no room.
fn top_strip() -> [Point; 4] {
    [(120.0, 0.0), (520.0, 0.0), (520.0, 30.0), (120.0, 30.0)].map(|(x, y)| Point::new(x, y))
}

/// A selection near the document's start edge, with room after it for a bar on its end side.
fn start_box() -> [Point; 4] {
    [(40.0, 120.0), (200.0, 120.0), (200.0, 240.0), (40.0, 240.0)].map(|(x, y)| Point::new(x, y))
}

/// The members of a selection bar.
fn formatting(ui: &mut Ui) {
    for (icon, name) in [("rotate_ccw", "Rotate left"), ("rotate_cw", "Rotate right")] {
        let _ = IconButton::new(IconId::new(icon)).label(name).show(ui);
    }
    let _ = Button::new("Done").show(ui);
}

/// A selection bar over a selection: above it, centred.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn selection_bar_over_a_selection(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let corners = turned_frame();
    on_canvas(ui, |ui, area, _| TaskBar::selection("Selection", &corners).show(ui, area, |ui, _| formatting(ui)).output)
}

/// A selection bar flips below its selection when there is no room above it.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn selection_bar_flipped_near_the_top_edge(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let strip = top_strip();
    on_canvas(ui, |ui, area, _| TaskBar::selection("Selection", &strip).show(ui, area, |ui, _| formatting(ui)).output)
}

/// A selection bar on the other side of its selection: after it in reading order.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn selection_bar_on_the_other_side(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let corners = start_box();
    on_canvas(ui, |ui, area, _| {
        TaskBar::selection("Selection", &corners).side(Side::End, Align::Center).show(ui, area, |ui, _| formatting(ui)).output
    })
}

/// A selection bar that survives presses outside it while its graphic is selected.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn selection_bar_kept_while_selected(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let corners = turned_frame();
    on_canvas(ui, |ui, area, _| TaskBar::selection("Selection", &corners).dismiss(TaskBarDismiss::Keep).show(ui, area, |ui, _| formatting(ui)).output)
}

/// The Crop tool's bar: three buttons, the More menu, then Cancel and Done after it.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn crop_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let corners = turned_frame();
    on_canvas(ui, |ui, area, _| {
        let (place, place_at) = data.value_at("place", TaskBarPlace::default());
        TaskBar::new("Crop", &corners, place_at.bind(place))
            .show(ui, area, |ui, cx| {
                for (icon, name) in [("rotate_ccw", "Rotate left"), ("rotate_cw", "Rotate right"), ("grid", "Overlay")] {
                    let _ = IconButton::new(IconId::new(icon)).label(name).show(ui);
                }
                cx.more_menu(ui);
                let _ = Button::new("Cancel").show(ui);
                let _ = Button::new("Done").show(ui);
            })
            .output
    })
}

/// The Crop bar's frame `corners` with a Done button, the bar set up by `configure`, in a story area as wide as `width`
/// when it is given.
fn configured(
    ui: &mut Ui,
    data: &mut StoryData,
    corners: &[Point],
    width: Option<f32>,
    configure: impl for<'a> Fn(TaskBar<'a>) -> TaskBar<'a>,
) -> StoryOutput {
    on_canvas(ui, |ui, area, _| {
        let (place, place_at) = data.value_at("place", TaskBarPlace::default());
        let mut area = area;
        if let Some(width) = width {
            area.set_width(width);
        }
        configure(TaskBar::new("Crop", corners, place_at.bind(place)))
            .show(ui, area, |ui, _| {
                for name in ["Rotate", "Flip", "Overlay", "Done"] {
                    let _ = Button::new(name).show(ui);
                }
            })
            .output
    })
}

/// A frame low on the story document: the canvas has no room below it, so a fixed bar hangs above it.
fn low_frame() -> [Point; 4] {
    [(120.0, 290.0), (520.0, 290.0), (520.0, 390.0), (120.0, 390.0)].map(|(x, y)| Point::new(x, y))
}

/// A fixed bar: no grip and no More menu, hung above its frame where there is no room below it.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn fixed_bar(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    let corners = low_frame();
    on_canvas(ui, |ui, area, _| {
        TaskBar::fixed("Signature", &corners)
            .show(ui, area, |ui, _| {
                let _ = Button::new("Cancel").show(ui);
                let _ = Button::new("Place").show(ui);
            })
            .output
    })
}

fn open_more(fixture: &mut Fixture) {
    if let Some(more) = fixture.by_label("More options") {
        fixture.click(&more).settle();
    }
}

/// The More menu open with the host's rows after its own: two command rows and a submenu.
#[story(TaskBar<'static>, covers = [], drive = Drive::Custom(open_more))]
fn host_rows(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = vec![
        PanelRow::Command(CommandRef::from(EditCommands::Undo)),
        PanelRow::Command(CommandRef::from(EditCommands::Redo)),
        PanelRow::Submenu("Arrange".into(), vec![PanelRow::Command(CommandRef::from(EditCommands::Cut))]),
    ];
    configured(ui, data, &turned_frame(), None, |bar| bar.more_rows(rows.clone()))
}

/// How far the beside-a-point story moves the frame to the right, in document points: near the canvas's right edge.
const NEAR_RIGHT_EDGE: Vec2 = Vec2::new(300.0, 0.0);

/// A bar beside the frame's right end, near the canvas's right edge so it flips to the other side, keeping clear of a
/// point.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn beside_a_point(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let corners = turned_frame().map(|corner| corner + NEAR_RIGHT_EDGE);
    let point = corners[0];
    configured(ui, data, &corners, None, |bar| bar.side(Side::End, Align::Start).keep_clear(point))
}

/// A bar resting above the frame.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn resting_above(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    configured(ui, data, &turned_frame(), None, |bar| bar.side(Side::Above, Align::Center))
}

/// A bar in a narrow area: its members continue on a new line.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn wrapped(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    configured(ui, data, &turned_frame(), Some(200.0), |bar| bar)
}

/// A fixed bar at the canvas view's top edge, at its end in reading direction: it takes no anchor.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn edge_bar(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |ui, area, _| {
        TaskBar::at_edge("Camera", Align::End)
            .show(ui, area, |ui, _| {
                let _ = Button::new("Capture").show(ui);
                let _ = Button::new("Done").show(ui);
            })
            .output
    })
}

/// A bar the user moved and pinned: it keeps its place near the canvas's top left while the frame stays where it is.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn placed_and_pinned(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let corners = turned_frame();
    on_canvas(ui, |ui, area, _| {
        let (place, place_at) = data.value_at("place", TaskBarPlace::new(true, Some([24.0, 24.0])));
        TaskBar::new("Crop", &corners, place_at.bind(place))
            .show(ui, area, |ui, _| {
                let _ = Button::new("Done").show(ui);
            })
            .output
    })
}

/// Points the pointer at the grip.
fn hover_grip(fixture: &mut Fixture) {
    if let Some(grip) = fixture.by_label("Move bar") {
        fixture.frame(vec![crate::testing::pointer::moved(grip.center())]);
        fixture.idle(8);
    }
}

/// How far the grip is dragged, in drag thresholds: left and down, far enough that the bar visibly moves.
const GRIP_DRAG_THRESHOLDS: [f32; 2] = [-8.0, 6.0];

/// Drags the bar by its grip and holds it there, the button down.
fn drag_grip(fixture: &mut Fixture) {
    if let Some(grip) = fixture.by_label("Move bar") {
        let threshold = crate::testing::input::slop(fixture.ctx());
        let [x, y] = GRIP_DRAG_THRESHOLDS;
        fixture.drag_hold_at(grip.center(), egui::vec2(x * threshold, y * threshold));
    }
}

/// The bar grip: one rounded upright bar in the line colour.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn bar_grip(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    configured(ui, data, &turned_frame(), None, |bar| bar.grip(TaskBarGrip::Bar))
}

/// The bar grip under the pointer: muted text.
#[story(TaskBar<'static>, covers = [], drive = Drive::Custom(hover_grip))]
fn bar_grip_hovered(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    configured(ui, data, &turned_frame(), None, |bar| bar.grip(TaskBarGrip::Bar))
}

/// The bar grip while the bar moves: muted text.
#[story(TaskBar<'static>, covers = [], drive = Drive::Custom(drag_grip))]
fn bar_grip_dragged(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    configured(ui, data, &turned_frame(), None, |bar| bar.grip(TaskBarGrip::Bar))
}

/// A bar whose members stand a control gap apart.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn control_gap(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    configured(ui, data, &turned_frame(), None, |bar| bar.member_gap(Gap::Item))
}

/// A bar hung clear of the frame's handles: the handle ring and a section gap below it.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn clear_of_handles(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    configured(ui, data, &turned_frame(), None, |bar| bar.hang_gap(TaskBarHang::ClearOfHandles))
}
