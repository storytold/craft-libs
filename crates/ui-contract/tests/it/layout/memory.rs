//! `LayoutMemory`: the tolerance rule, size tweens and the check of a kept transcript.

use craft_behavior::ColumnWidth;
use craft_ui::contract::{ContainerProps, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::layout::BodyLayout;
use craft_ui::testing::layout::{Block, body, measured_block};
use craft_ui::testing::{Frames, last_pass_measuring_runs, trial};
use craft_ui_test_kit::body::Slab;
use egui::{Rect, Ui, Vec2};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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

/// A real run under a kept transcript checks each leaf against the answer the leaf's own run just gave, when the
/// transcript's call is at the same proposal and room: a settled frame measures each leaf once, and a leaf that
/// changes under the same body inputs shows its new size in that frame.
#[trial]
fn a_settled_body_measures_each_leaf_once_and_a_changed_leaf_shows_in_its_frame() {
    const LEAVES: usize = 3;
    let measures = Arc::new(AtomicUsize::new(0));
    let size = Cell::new(Vec2::new(80.0, 20.0));
    let last = Cell::new(Rect::NOTHING);
    let show = |ui: &mut Ui| {
        let at = Rect::from_min_size(ui.cursor().min, Vec2::new(300.0, 200.0));
        let _ = Slab::new(ContainerProps::new().sizing(Sizing::Fill)).id_salt("slab").show_at(ui, at, |ui, _| {
            for _ in 0..LEAVES {
                Block::new(Vec2::new(60.0, 20.0)).counted(&measures).show(ui);
            }
            last.set(Block::new(size.get()).show(ui).rect);
        });
    };
    let mut frames = Frames::new();
    frames.idle(4, show);
    measures.store(0, Ordering::Relaxed);
    assert_eq!(frames.frame(vec![], show), 1, "a settled frame runs one pass");
    let runs = last_pass_measuring_runs(&frames.ctx);
    assert!(runs.is_empty(), "the body takes its kept transcript: {runs:?}");
    assert_eq!(measures.load(Ordering::Relaxed), LEAVES, "each leaf is measured once, in its own run");
    size.set(Vec2::new(140.0, 20.0));
    frames.frame(vec![], show);
    assert!((last.get().width() - 140.0).abs() < 0.5, "the changed leaf shows its new size in its frame: {:?}", last.get());
}
