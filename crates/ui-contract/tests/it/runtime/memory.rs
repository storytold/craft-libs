//! Memory lifetimes, eviction by owner and the runtime sub-id derivation.

use craft_ui::runtime::memory::{Remember, SaveKey};
use craft_ui::testing::memory::{Stage, dispose_evicted, forget, forget_saved, global};
use craft_ui::testing::{Frames, trial};

const PLACE: SaveKey = SaveKey::new("craft_tests.place");

/// The lifetimes of 7.2, through `Remember`: shown entries go with their owner; session and saved ones only
/// through `forget` and `forget_saved`; a press entry goes when its press ends.
#[trial]
fn lifetimes_follow_their_owner_until_forgotten() {
    let mut frames = Frames::new();
    let id = global("owner", ());
    let mut appeared = Vec::new();
    for _ in 0..2 {
        frames.frame(vec![], |ui| {
            let mut stage = Stage::open(ui.ctx(), id);
            appeared.push(stage.appeared());
            *stage.memory::<u32>() += 1;
            *stage.memory_at::<u32>("part") += 10;
            *stage.keep::<u32>() = 5;
            *stage.saved::<u32>(&PLACE) = 6;
            *stage.press_slot::<u32>() = Some(8);
            stage.finish();
        });
    }
    assert_eq!(appeared, vec![true, false], "appeared is true on the first pass only");
    frames.frame(vec![], |ui| {
        let mut stage = Stage::open(ui.ctx(), id);
        assert_eq!(*stage.memory::<u32>(), 2);
        assert_eq!(*stage.memory_at::<u32>("part"), 20, "memory_at keeps its own slot");
        assert_eq!(*stage.press_slot::<u32>(), Some(8), "a press entry lives while its press does");
        *stage.press_slot::<u32>() = None;
        stage.finish();
    });
    frames.frame(vec![], |ui| {
        let mut stage = Stage::open(ui.ctx(), id);
        assert_eq!(*stage.press_slot::<u32>(), None, "an ended press leaves no entry");
        stage.finish();
    });
    frames.idle(60, |_| {});
    dispose_evicted(&frames.ctx);
    frames.frame(vec![], |ui| {
        let mut stage = Stage::open(ui.ctx(), id);
        assert!(stage.appeared(), "a re-shown component appears again");
        assert_eq!(*stage.memory::<u32>(), 0, "the shown entry went with its owner");
        assert_eq!(*stage.memory_at::<u32>("part"), 0, "and the memory_at entry");
        assert_eq!(*stage.keep::<u32>(), 5, "the session entry stayed");
        assert_eq!(*stage.saved::<u32>(&PLACE), 6, "the saved entry stayed");
        stage.finish();
    });
    forget(&frames.ctx, id);
    frames.frame(vec![], |ui| {
        let mut stage = Stage::open(ui.ctx(), id);
        assert_eq!(*stage.keep::<u32>(), 0, "forget removes the session entry");
        assert_eq!(*stage.saved::<u32>(&PLACE), 6, "and not the saved one");
        stage.finish();
    });
    forget_saved(&frames.ctx, &PLACE);
    frames.frame(vec![], |ui| {
        let mut stage = Stage::open(ui.ctx(), id);
        assert_eq!(*stage.saved::<u32>(&PLACE), 0, "forget_saved removes the saved entry written through Remember::saved");
        stage.finish();
    });
}
