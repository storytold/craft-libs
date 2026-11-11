//! Stories of `ReferencePoint`.

use super::ReferencePoint;
use crate::contract::{Bind, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{Anchor9, Mixed};
use egui::Ui;

fn pick_top_left(fixture: &mut Fixture) {
    if let Some(corner) = fixture.by_label("Top left") {
        fixture.click(&corner);
    }
}

#[story(ReferencePoint<'static>, covers = [Default], drive = Drive::Custom(pick_top_left))]
fn reference_point(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ReferencePoint::new("Reference point", Bind::bound(data.value("point", Anchor9::Centre))).show(ui).into()
}

#[story(ReferencePoint<'static>, covers = [Mixed], drive = Drive::Never)]
fn reference_point_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ReferencePoint::new("Reference point", Bind::bound_mixed(data.value("mixed", Mixed::Mixed))).show(ui).into()
}

#[story(ReferencePoint<'static>, covers = [Invalid], drive = Drive::Never)]
fn reference_point_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_bottom = |point: &Anchor9| if *point == Anchor9::Bottom { Validity::Refuse(Message::new("Not a corner")) } else { Validity::Ok };
    ReferencePoint::new("Reference point", Bind::bound(data.value("bottom", Anchor9::Bottom)).check(refuse_bottom)).show(ui).into()
}

/// The canvas grows wider and shrinks shorter: the arrows beside the pick point away along the width and back along
/// the height.
#[story(ReferencePoint<'static>, covers = [], drive = Drive::Never)]
fn reference_point_growth(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ReferencePoint::new("Reference point", Bind::bound(data.value("grown", Anchor9::TopLeft))).growth((200.0, -100.0)).show(ui).into()
}

/// The options bar's size: the grid is an icon square and every point is a dot.
#[story(ReferencePoint<'static>, covers = [], drive = Drive::Never)]
fn reference_point_compact(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ReferencePoint::new("Reference point", Bind::bound(data.value("compact", Anchor9::Centre))).compact().show(ui).into()
}
