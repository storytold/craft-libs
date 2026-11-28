//! The gesture record a component reads: the value its running gesture began with.

use craft_behavior::Mixed;
use craft_ui::contract::{Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Props as _, Role, Show as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::Vec2;

/// What a [`Dragger`] does in one pass.
#[derive(Clone, Copy)]
enum Step {
    Idle,
    Propose(u8),
    ProposeNoted(u8, &'static str),
    Commit,
}

/// A leaf over a `u8` binding that proposes, commits or does nothing as `step` says, and reports its gesture's start.
#[derive(Component)]
struct Dragger<'a> {
    #[common]
    common: Common,
    value: Bind<'a, u8>,
    step: Step,
    before: &'a mut Option<Mixed<u8>>,
}

impl Component for Dragger<'_> {
    const ROLE: Role = Role::None;
    type Style = PlainStyle;
    type Output = EditResponse<u8>;

    fn style(_: &StyleCx<'_>, _: DraggerStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::splat(10.0))
    }

    fn bind(&mut self) -> Option<BindMut<'_, u8>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        match self.step {
            Step::Idle => {}
            Step::Propose(to) => cx.propose(to),
            Step::ProposeNoted(to, note) => cx.propose_with(to, note),
            Step::Commit => cx.commit(),
        }
        *self.before = cx.gesture_before();
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<u8> {
        cx.edit_response()
    }
}

/// The start a running gesture reads stays the value at its begin while the bound value follows each proposal, and
/// there is none before or after the gesture.
#[trial]
fn a_gesture_reads_the_value_it_began_with_until_it_ends() {
    let mut frames = Frames::new();
    let mut value = 4u8;
    let mut show = |step: Step, value: &mut u8| {
        let mut before = None;
        frames.frame(vec![], |ui| {
            Dragger { common: Common::new(), value: Bind::bound(&mut *value), step, before: &mut before }.id_salt("dragger").show(ui);
        });
        before
    };
    assert_eq!(show(Step::Idle, &mut value), None, "no gesture yet");
    assert_eq!(show(Step::Propose(5), &mut value), Some(Mixed::Same(4)), "the begin pass reads the value at the press");
    assert_eq!(show(Step::Propose(9), &mut value), Some(Mixed::Same(4)), "an update reads the same start");
    assert_eq!(value, 9, "the binding follows the proposals");
    assert_eq!(show(Step::Idle, &mut value), Some(Mixed::Same(4)), "a pass with no proposal still reads it");
    assert_eq!(show(Step::Commit, &mut value), None, "the pass that commits has none");
    assert_eq!(show(Step::Idle, &mut value), None, "none after the gesture");
}

/// The note a gesture sets is readable outside the pass while the gesture runs, a refused proposal leaves the last
/// note as it was, and the note goes when the gesture ends.
#[trial]
fn a_gesture_note_is_set_by_accepted_proposals_and_ends_with_the_gesture() {
    let mut frames = Frames::new();
    let mut value = 4u8;
    let mut show = |step: Step, enabled: bool, value: &mut u8| {
        let mut before = None;
        let mut id = None;
        frames.frame(vec![], |ui| {
            id = Some(
                Dragger { common: Common::new(), value: Bind::bound(&mut *value), step, before: &mut before }
                    .enabled(enabled)
                    .id_salt("dragger")
                    .show(ui)
                    .response
                    .id,
            );
        });
        (frames.ctx.clone(), id.unwrap())
    };
    let (ctx, id) = show(Step::ProposeNoted(5, "first"), true, &mut value);
    assert_eq!(craft_ui::runtime::edit::gesture_note::<u8, &'static str>(&ctx, id), Some("first"));
    show(Step::ProposeNoted(6, "refused"), false, &mut value);
    assert_eq!(craft_ui::runtime::edit::gesture_note::<u8, &'static str>(&ctx, id), Some("first"), "a refused proposal sets no note");
    show(Step::ProposeNoted(7, "second"), true, &mut value);
    assert_eq!(craft_ui::runtime::edit::gesture_note::<u8, &'static str>(&ctx, id), Some("second"));
    show(Step::Commit, true, &mut value);
    assert_eq!(craft_ui::runtime::edit::gesture_note::<u8, &'static str>(&ctx, id), None, "no note after the gesture");
}
