//! Where commands were shown (`commands::holds`, `commands::shown`).

use craft_ui::contract::{CommandSet, MemoProps, MemoView, Props as _, Show as _};
use craft_ui::runtime::commands::{self, CommandRef};
use craft_ui::runtime::env::Label;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Fixture, Frames, MemoRun, last_frame_memo_runs, trial};
use craft_ui_test_kit::contract::{Probe, Seen};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;
use std::sync::Arc;

/// A container that holds a hundred commands, as a menu title holds its menu's, registers once per pass: a replay of
/// the menu bar then makes one registration per title, not one per command (photocraft#15493). Each command is still
/// found, at the container's rect, as held.
#[trial]
fn a_container_registers_its_commands_once() {
    let held: Vec<CommandRef> = (0..100).map(|i| CommandRef::new(format!("test.held{i}"))).collect();
    let shown: Arc<[CommandRef]> = held.clone().into();
    let mut fixture = Fixture::new(move |ui| {
        let rect = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(40.0, 20.0));
        commands::holds(ui.ctx(), ui.layer_id(), rect, &shown, Label::from("Image"));
    });
    fixture.settle();
    assert_eq!(commands::shown_registrations(fixture.ctx()), 1, "one registration for the container");
    for command in [&held[0], &held[57], &held[99]] {
        let found = commands::shown(fixture.ctx(), command).expect("a held command is shown");
        assert!(found.holder, "{command} is shown as held");
        assert_eq!(found.rect.size(), egui::vec2(40.0, 20.0), "{command} is shown at the container's rect");
    }
    assert!(commands::shown(fixture.ctx(), &CommandRef::new("test.other")).is_none(), "a command nothing holds is not shown");
}

/// The command every probe of the leaves view is bound to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "shownTrial")]
#[non_exhaustive]
enum Bound {
    #[command(label = "Apply")]
    Apply,
}

/// A memoized view of `count` probes bound to `Bound::Apply`.
#[derive(MemoProps)]
struct Leaves {
    count: usize,
}

fn leaves(props: &mut Leaves, ui: &mut Ui) {
    for _ in 0..props.count {
        let seen = RefCell::new(Seen::default());
        Probe::new(&seen).command(Bound::Apply).show(ui);
    }
}

/// A settled view's replay registers where its bound components show their commands with the very commands the run
/// registered, so a settled pass copies no command, however many it shows.
#[trial]
fn a_settled_pass_shows_its_commands_without_copying_them() {
    for count in [10, 40] {
        let mut frames = Frames::new();
        frames.screen(Vec2::new(400.0, 1400.0));
        let mut show = |ui: &mut Ui| {
            in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(200.0, 1300.0)), |ui| {
                MemoView::new("leaves", Leaves { count }, leaves).show(ui);
            });
        };
        frames.idle(30, &mut show);
        frames.frame(vec![], &mut show);
        let before = commands::shown_commands(&frames.ctx);
        frames.frame(vec![], &mut show);
        let after = commands::shown_commands(&frames.ctx);
        assert!(last_frame_memo_runs(&frames.ctx).iter().all(|(_, _, run)| *run != MemoRun::Ran), "the view of {count} probes is settled");
        assert_eq!(before.len(), count, "each of the {count} probes is shown");
        assert_eq!(after.len(), count, "the settled pass registers each of the {count} probes again");
        assert!(
            before.iter().zip(&after).all(|(before, after)| Arc::ptr_eq(before, after)),
            "a settled pass of {count} probes shows the commands the pass before it showed, not copies"
        );
    }
}
