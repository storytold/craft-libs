//! Stories of `TimelineOverview`: eight tracks of runs, the visible window and the playhead.

use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Color32, Pos2, Ui, Vec2};

use super::{TimelineOverview, overview_runs};
use crate::time::{Tick, TimeRange, TimeView};
use crate::ui::LabelPalette;

/// The tracks of the story and the clips of each.
const TRACKS: usize = 8;
const CLIPS: usize = 6;

/// Eight tracks of six clips in the label colours, of uneven lengths and gaps, and the end of the last clip.
fn tracks() -> (Vec<Vec<(TimeRange, Color32)>>, Tick) {
    let palette = LabelPalette::default();
    let mut end = Tick::ZERO;
    let tracks = (0..TRACKS)
        .map(|track| {
            let mut at = (track % 3) as f64 * 2.0;
            (0..CLIPS)
                .map(|clip| {
                    let length = 6.0 + ((track + clip) % 4) as f64 * 3.0;
                    let range = TimeRange::new(Tick::from_seconds_f64(at), Tick::from_seconds_f64(length));
                    at += length + 1.0 + ((track + 2 * clip) % 3) as f64 * 2.0;
                    end = end.max(range.end());
                    (range, palette.0[(track * 2 + clip) % palette.0.len()])
                })
                .collect()
        })
        .collect();
    (tracks, end)
}

/// Where the story's view starts and its zoom, in points per second, and its playhead, in seconds.
const VIEW_START: f64 = 20.0;
const VIEW_ZOOM: f64 = 10.0;
const PLAYHEAD: f64 = 30.0;

/// The closest zoom the invalid story accepts, in points per second.
const CLOSEST_ZOOM: f64 = 100.0;
/// Where along the overview the drive presses and how far it drags, as shares of its width.
const PRESS_SHARE: f32 = 0.2;
const DRAG_SHARE: f32 = 0.15;

/// Presses the overview off the window's middle and drags: the window follows the pointer.
fn centre_the_window(fixture: &mut Fixture) {
    let shown = fixture.shown();
    let from = Pos2::new(shown.left() + shown.width() * PRESS_SHARE, shown.center().y);
    fixture.drag_at(from, Vec2::X * (shown.width() * DRAG_SHARE));
}

/// Shows the overview of the eight tracks over `view`.
fn shown(ui: &mut Ui, view: Bind<'_, TimeView>) -> StoryOutput {
    let width = ui.available_width();
    let (tracks, end) = tracks();
    let runs = overview_runs(&tracks, end, width);
    TimelineOverview::new("Overview", &runs, view, end).shown_width(width).playhead(Tick::from_seconds_f64(PLAYHEAD)).show(ui).into()
}

#[story(TimelineOverview<'static>, covers = [Default], drive = Drive::Custom(centre_the_window))]
fn eight_tracks(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    shown(ui, Bind::bound(data.value("view", TimeView::new(Tick::from_seconds_f64(VIEW_START), VIEW_ZOOM))))
}

#[story(TimelineOverview<'static>, covers = [Invalid], drive = Drive::Never)]
fn window_closer_than_the_limit(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let not_too_close = |view: &TimeView| {
        if view.points_per_second > CLOSEST_ZOOM { Validity::Refuse(Message::new("The window cannot be that narrow")) } else { Validity::Ok }
    };
    shown(ui, Bind::bound(data.value("close", TimeView::new(Tick::ZERO, CLOSEST_ZOOM * 2.0))).check(not_too_close))
}
