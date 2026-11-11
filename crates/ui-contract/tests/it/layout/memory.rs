//! `LayoutMemory`: the tolerance rule and size tweens.

use craft_behavior::ColumnWidth;
use craft_ui::layout::BodyLayout;
use craft_ui::testing::layout::{body, measured_block};
use craft_ui::testing::{Frames, trial};
use egui::Vec2;

static COLUMNS: [ColumnWidth; 2] = [ColumnWidth::Content, ColumnWidth::Content];

/// Skeleton: `frames_state::end_aligned_readout_changing_every_frame_runs_one_pass`.
///
/// One rule covers every measured block: a first appearance and a change after a settled frame run one
/// discard pass, and a size that changes every frame is placed from last frame's value and runs none.
#[trial]
fn continuous_change_runs_no_discard_and_a_settled_change_runs_one() {
    const FIRST: f32 = 20.0;
    const STEP: f32 = 30.0;
    const SETTLED: f32 = 290.0;
    const LONGER: f32 = 400.0;
    const WITHIN_TOLERANCE: f32 = 400.4;
    let mut frames = Frames::new();
    let grid = BodyLayout::grid(&COLUMNS);
    let run = |frames: &mut Frames, width: f32| {
        frames.frame(vec![], |ui| {
            measured_block(ui, "readout", Vec2::new(width, FIRST));
            body(ui, "grid", &grid, &[Vec2::new(width, FIRST), Vec2::new(width, FIRST)]);
        })
    };
    let (once, twice) = (1, 2);
    assert_eq!(run(&mut frames, FIRST), twice, "the first appearance runs a discard pass");
    let growing: Vec<usize> = (1..10).map(|step| run(&mut frames, FIRST + STEP * step as f32)).collect();
    assert!(growing[1..].iter().all(|passes| *passes == once), "continuous change runs one pass per frame: {growing:?}");
    for _ in 0..3 {
        assert!(run(&mut frames, SETTLED) <= twice, "settling takes at most one extra pass");
    }
    assert_eq!(run(&mut frames, SETTLED), once, "a settled block runs one pass");
    assert_eq!(run(&mut frames, LONGER), twice, "a change after a settled frame runs a discard pass, for both blocks together");
    assert_eq!(run(&mut frames, WITHIN_TOLERANCE), once, "a change under half a point is the same size");
}
