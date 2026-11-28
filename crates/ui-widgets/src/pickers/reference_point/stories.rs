//! Stories of `ReferencePoint`.

use super::{ReferencePoint, ReferencePointKind, ReferencePointLook};
use crate::contract::{Show as _, Validity};
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
    ReferencePoint::new("Reference point", data.bind("point", Some(Anchor9::Centre))).growth((1.0, 1.0)).show(ui).into()
}

#[story(ReferencePoint<'static>, covers = [Mixed], drive = Drive::Never)]
fn reference_point_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ReferencePoint::new("Reference point", data.bind_mixed("mixed", Mixed::<Option<Anchor9>>::Mixed)).show(ui).into()
}

#[story(ReferencePoint<'static>, covers = [Invalid], drive = Drive::Never)]
fn reference_point_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_bottom =
        |point: &Option<Anchor9>| if *point == Some(Anchor9::Bottom) { Validity::Refuse(Message::new("Not a corner")) } else { Validity::Ok };
    ReferencePoint::new("Reference point", data.bind("bottom", Some(Anchor9::Bottom)).check(refuse_bottom)).show(ui).into()
}

/// The canvas grows wider and shrinks shorter: the arrows beside the pick point away along the width and back along
/// the height.
#[story(ReferencePoint<'static>, covers = [], drive = Drive::Never)]
fn reference_point_growth(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ReferencePoint::new("Reference point", data.bind("grown", Some(Anchor9::TopLeft))).growth((200.0, -100.0)).show(ui).into()
}

/// The alignment of text in a cell: three bars in each cell, the pick's cell filled; a click on the pick clears it.
#[story(ReferencePoint<'static>, covers = [], drive = Drive::Custom(pick_top_left))]
fn reference_point_alignment(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ReferencePoint::new("Alignment", data.bind("alignment", Some(Anchor9::Centre))).kind(ReferencePointKind::Alignment).show(ui).into()
}

/// The alignment picker with no alignment picked: no cell is filled.
#[story(ReferencePoint<'static>, covers = [], drive = Drive::Never)]
fn reference_point_alignment_unset(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ReferencePoint::new("Alignment", data.bind("alignment_unset", None::<Anchor9>)).kind(ReferencePointKind::Alignment).show(ui).into()
}

/// The squares look: eight squares joined by a line round the box, the centre one alone, the pick filled in the text
/// ink, with no well behind them.
#[story(ReferencePoint<'static>, covers = [], drive = Drive::Never)]
fn reference_point_squares(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ReferencePoint::new("Reference point", data.bind("squares", Some(Anchor9::TopRight))).look(ReferencePointLook::Squares).show(ui).into()
}

/// The squares look with a mixed value: no square is filled.
#[story(ReferencePoint<'static>, covers = [], drive = Drive::Never)]
fn reference_point_squares_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ReferencePoint::new("Reference point", data.bind_mixed("squares_mixed", Mixed::<Option<Anchor9>>::Mixed))
        .look(ReferencePointLook::Squares)
        .show(ui)
        .into()
}

/// The options bar's size: the grid is an icon square and every point is a dot.
#[story(ReferencePoint<'static>, covers = [], drive = Drive::Never)]
fn reference_point_compact(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ReferencePoint::new("Reference point", data.bind("compact", Some(Anchor9::Centre))).compact().show(ui).into()
}

/// The squares look in the compact picker, as an inspector's transform fields lead with it: the same size as the
/// full picker, from `shape.locator`.
#[story(ReferencePoint<'static>, covers = [], drive = Drive::Never)]
fn reference_point_squares_compact(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ReferencePoint::new("Reference point", data.bind("squares_compact", Some(Anchor9::Centre)))
        .look(ReferencePointLook::Squares)
        .compact()
        .show(ui)
        .into()
}
