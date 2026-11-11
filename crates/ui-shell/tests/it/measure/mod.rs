//! A measuring run delivers nothing and consumes nothing: what the real run after it needs is still there.

mod record;

use craft_ui::contract::{Bind, Politeness, Show as _};
use craft_ui::runtime::commands::{FireSource, fire};
use craft_ui::runtime::toast::{self, Toast};
use craft_ui::testing::{Frames, is_measuring, measuring_run, take_effects, trial};
use craft_ui::text::TextField;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[trial]
fn a_measuring_run_fires_and_announces_nothing() {
    let mut frames = Frames::new();
    frames.frame(vec![], |ui| {
        let ctx = ui.ctx().clone();
        measuring_run(&ctx, || {
            assert!(!fire(&ctx, "save", FireSource::Palette));
            craft_ui::contract::announce(&ctx, "Saved", Politeness::Polite);
            let id = toast::post(&ctx, Toast::new("Saved"));
            toast::dismiss(&ctx, id);
        });
        assert_eq!(take_effects(&ctx), ["commands::fire", "announce", "toast::post"]);
        assert!(toast::cards(&ctx).is_empty(), "no toast was posted");
    });
    assert!(craft_ui::runtime::frame::take_output(&frames.ctx).announcements.is_empty(), "nothing was announced");
}

// A measuring run a panic leaves ends with the unwind: the frames after it on the same thread (a test harness runs its
// next trials there) are real runs, whose service touches the fixture's measuring-run guard lets through.
#[trial]
fn a_measuring_run_that_panics_ends_with_the_unwind() {
    let mut frames = Frames::new();
    let mut name = String::from("Ada");
    frames.frame(vec![], |ui| {
        let ctx = ui.ctx().clone();
        assert!(catch_unwind(AssertUnwindSafe(|| measuring_run(&ctx, || panic!("a measure fails")))).is_err());
        assert!(!is_measuring(&ctx));
        let _ = TextField::new("Name", Bind::bound(&mut name)).show(ui);
    });
    frames.frame(vec![], |ui| {
        let _ = TextField::new("Name", Bind::bound(&mut name)).show(ui);
    });
}
