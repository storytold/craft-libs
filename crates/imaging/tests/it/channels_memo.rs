//! The Channels panel in a host's memoized view: a pointer move beside it replays it (#2812).

use craft_ui::contract::{MemoProps, MemoView};
use craft_ui::testing::{Fixture, MemoRun, last_frame_memo_runs, pointer, trial};
use egui::{Modifiers, Pos2, Rect, UiBuilder, Vec2};

use crate::ui::channels::{ChannelKind, ChannelRow, ChannelsPanel};

const PANEL: Vec2 = Vec2::new(320.0, 400.0);

/// What a host's view of the panel reads: its rows, which the host's revision stands for.
#[derive(MemoProps)]
struct Host<'a> {
    revision: u64,
    #[memo_props(covered_by = "revision", reason = "the rows are fixed in this trial")]
    rows: &'a [ChannelRow<u8>],
}

fn host(props: &mut Host<'_>, ui: &mut egui::Ui) {
    let _ = ChannelsPanel::new(props.rows).show(ui);
}

/// The panel reads the modifiers its commands take, not the pointer: a pointer move beside it replays its view, and a
/// modifier change runs it. Before, every pointer move in the window ran it.
#[trial]
fn a_pointer_move_beside_the_panel_replays_it_and_a_modifier_runs_it() {
    let rows =
        [(0, ChannelKind::Composite, "RGB"), (1, ChannelKind::Color, "Red"), (2, ChannelKind::Color, "Green"), (3, ChannelKind::Color, "Blue")]
            .map(|(key, kind, name)| ChannelRow::new(key, kind, name));
    let mut fixture = Fixture::new(move |ui| {
        let at = Rect::from_min_size(ui.max_rect().min, PANEL);
        ui.scope_builder(UiBuilder::new().max_rect(at), |ui| {
            MemoView::new("channels", Host { revision: 0, rows: &rows }, host).show(ui);
        });
    });
    fixture.room(Vec2::new(PANEL.x + 400.0, PANEL.y)).settle().idle(2);
    let runs = |fixture: &Fixture| {
        let runs = last_frame_memo_runs(fixture.ctx());
        let count = |wanted: MemoRun| runs.iter().filter(|(name, _, run)| name.contains("Host") && *run == wanted).count();
        (count(MemoRun::Ran), count(MemoRun::Replayed))
    };
    // The pointer enters the window beside the panel; in an app it is already there.
    fixture.frame(vec![pointer::moved(Pos2::new(PANEL.x + 200.0, 90.0))]);
    for step in 0..3 {
        fixture.frame(vec![pointer::moved(Pos2::new(PANEL.x + 200.0, 100.0 + step as f32 * 10.0))]);
        assert_eq!(runs(&fixture), (0, 1), "pointer move {step} beside the panel replays its view");
    }
    fixture.hold(Modifiers::ALT).frame(Vec::new());
    assert_eq!(runs(&fixture).0, 1, "Alt held runs the view");
    fixture.frame(Vec::new());
    assert_eq!(runs(&fixture), (0, 1), "and it replays while Alt stays held");
}
