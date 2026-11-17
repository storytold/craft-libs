//! Stories of `ReferencePoint`.

use super::{PointPresentation, ReferencePoint};
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

/// Canvas Size's picker, as the old gallery's card showed it: the canvas grows both ways, so every cell around the
/// centre pick shows its arrow.
#[story(ReferencePoint<'static>, covers = [Default], drive = Drive::Custom(pick_top_left))]
fn reference_point(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ReferencePoint::new("Reference point", Bind::bound(data.value("point", Anchor9::Centre))).growth((1.0, 1.0)).show(ui).into()
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

/// The options bar's size: an icon square, every point a dot of the one width, the pick's in the accent colour.
#[story(ReferencePoint<'static>, covers = [], drive = Drive::Never)]
fn reference_point_compact(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ReferencePoint::new("Reference point", Bind::bound(data.value("compact", Anchor9::Centre))).compact().show(ui).into()
}

/// The nine points as squares joined into the box's grid: the pick's square filled, the others outlined.
#[story(ReferencePoint<'static>, covers = [], drive = Drive::Custom(pick_top_left))]
fn reference_point_connected_squares(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = Bind::bound(data.value("squares", Anchor9::Centre));
    ReferencePoint::new("Reference point", value).presentation(PointPresentation::ConnectedSquares).show(ui).into()
}

/// The squares keep the growth arrows: the box grows both ways, so every square beside the pick frames an arrow
/// pointing away from it.
#[story(ReferencePoint<'static>, covers = [], drive = Drive::Never)]
fn reference_point_connected_squares_growth(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = Bind::bound(data.value("squares_growing", Anchor9::Centre));
    ReferencePoint::new("Reference point", value).presentation(PointPresentation::ConnectedSquares).growth((1.0, 1.0)).show(ui).into()
}

/// The squares at the options bar's size: every square the one side, the pick's filled where the rest are outlined, so
/// the presentation is the mark a point shows and not the size the picker takes.
#[story(ReferencePoint<'static>, covers = [], drive = Drive::Never)]
fn reference_point_connected_squares_compact(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = Bind::bound(data.value("squares_compact", Anchor9::BottomLeft));
    ReferencePoint::new("Reference point", value).presentation(PointPresentation::ConnectedSquares).compact().show(ui).into()
}
