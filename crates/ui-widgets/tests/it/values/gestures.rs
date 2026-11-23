//! Gestures: Escape restores, an app change or an undo cancels, a cancelled gesture stays cancelled under its press, a
//! reset is one `Set`, a read-only component refuses edits and a composite adopts its children's edits.

use super::kit::{APP_VALUE, AT, HELD, PlainStyle, Scrub, near};
use craft_behavior::{EditEvent, Mixed, Sense};
use craft_ui::contract::{Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, PaintUi, Props as _, Role, Show as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use egui::{Pos2, Ui, Vec2};
use std::cell::RefCell;

fn start() -> Pos2 {
    Pos2::new(AT.left() + 10.0, AT.center().y)
}

/// Pressed on the track, past the slop and moved once more: a drag in progress.
fn dragging(frames: &mut Frames, show: &mut impl FnMut(&mut Ui)) {
    frames.idle(3, &mut *show);
    frames.frame(vec![moved(start())], &mut *show);
    frames.frame(vec![press(start())], &mut *show);
    frames.frame(vec![moved(start() + Vec2::new(30.0, 0.0))], &mut *show);
    frames.frame(vec![moved(start() + Vec2::new(35.0, 0.0))], &mut *show);
}

/// A composite whose child is a value control: it adopts the child's edit as its own gesture.
#[derive(craft_ui::contract::Component)]
struct Wrapper<'a> {
    #[common]
    common: Common,
    value: Bind<'a, f32>,
}

impl Component for Wrapper<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = EditResponse<f32>;

    fn style(_: &StyleCx<'_>, _: WrapperStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        AT.size()
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn bind(&mut self) -> Option<BindMut<'_, f32>> {
        Some(self.value.reborrow())
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let shown = cx.value().copied().unwrap_or(0.0);
        let child = ui.child("scrub", cx.rect, Scrub::new(Bind::given(shown)));
        cx.adopt(&child, &APP_VALUE, |value, mine| *mine = *value);
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<f32> {
        cx.edit_response()
    }
}

/// A composite whose value is mixed maps the child's edit onto the start value: one `Begin` and one `Commit` that
/// begins from the mixed value, instead of dropping the edit.
#[trial]
fn a_mixed_composite_adopts_its_childs_edit_onto_the_start_value() {
    let mut frames = Frames::new();
    let mut field = Mixed::Mixed;
    let seen = RefCell::new(Vec::new());
    let mut show = |ui: &mut Ui| {
        let out = Wrapper { common: Common::new(), value: Bind::bound_mixed(&mut field) }.id_salt("wrapper").show_at(ui, AT);
        seen.borrow_mut().extend(out.events.clone());
    };
    dragging(&mut frames, &mut show);
    frames.frame(vec![release(start() + Vec2::new(35.0, 0.0))], &mut show);
    frames.idle(2, &mut show);
    drop(show);
    let events = seen.take();
    let commits: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            EditEvent::Commit { before, after, .. } => Some((*before, *after)),
            EditEvent::Set { before, after, .. } => Some((*before, *after)),
            _ => None,
        })
        .collect();
    assert_eq!(commits.len(), 1, "one commit for one gesture: {events:?}");
    assert_eq!(commits[0].0, Mixed::Mixed, "the edit began from the mixed value");
    assert!(near(commits[0].1, HELD), "the child's last value is the commit's: {}", commits[0].1);
    assert!(matches!(field, Mixed::Same(value) if near(value, HELD)), "every target took the value: {field:?}");
}
