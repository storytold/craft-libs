//! Stories of `TaskBar`.

use super::{TaskBar, TaskBarPlace};
use crate::buttons::{Button, IconButton};
use crate::canvas::testing::canvas_story::{StoryDocument, on_canvas};
use crate::contract::{Bind, Props as _, Show as _};
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

/// The Crop tool's bar: three buttons, the More menu, then Cancel and Done after it.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn crop_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let corners = turned_frame();
    on_canvas(ui, |ui, area, _| {
        let place = data.value("place", TaskBarPlace::default());
        TaskBar::new("Crop", &corners, Bind::bound(place))
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
        let place = data.value("place", TaskBarPlace::default());
        let mut area = area;
        if let Some(width) = width {
            area.set_width(width);
        }
        configure(TaskBar::new("Crop", corners, Bind::bound(place)))
            .show(ui, area, |ui, _| {
                for name in ["Rotate", "Flip", "Overlay", "Done"] {
                    let _ = Button::new(name).show(ui);
                }
            })
            .output
    })
}

fn open_more(fixture: &mut Fixture) {
    let more = fixture.by_label("More options").expect("the bar has its More button");
    fixture.click(&more).settle();
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

/// A bar beside the frame's right end, near the canvas's right edge so it flips to the other side, keeping clear of a
/// point.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn beside_a_point(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let corners = turned_frame().map(|corner| corner + Vec2::new(300.0, 0.0));
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

/// A bar the user moved and pinned: it keeps its place near the canvas's top left while the frame stays where it is.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn placed_and_pinned(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let corners = turned_frame();
    on_canvas(ui, |ui, area, _| {
        let place = data.value("place", TaskBarPlace::new(true, Some([24.0, 24.0])));
        TaskBar::new("Crop", &corners, Bind::bound(place))
            .show(ui, area, |ui, _| {
                let _ = Button::new("Done").show(ui);
            })
            .output
    })
}
