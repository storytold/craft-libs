//! Stories of `ThumbRow`: a levels row with split black and white thumbs, a wrapping hue row, and a hue range bar
//! ordered round from its first thumb.

use super::{Anchor, Order, Thumb, ThumbRow};
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::display::Progress;
use crate::runtime::env::Message;
use crate::style::DataColor;
use crate::testing::{Drive, Grip, StoryData, StoryOutput, story};
use crate::value_drag::TrackFace;
use craft_behavior::Mixed;
use egui::{Color32, Ui};

const LEVELS: std::ops::RangeInclusive<f64> = 0.0..=255.0;
const HUES: std::ops::RangeInclusive<f64> = 0.0..=360.0;
const LIMIT: f64 = 100.0;

fn gray(level: u8) -> DataColor {
    DataColor::new(Color32::from_gray(level))
}

/// Black and white, each a split pair: Alt-drag moves a half alone.
fn levels(value: Bind<'_, [Mixed<f64>; 4]>) -> ThumbRow<'_, 4> {
    let ordered = |below, above| Order::between(below, above, 0.0);
    let thumbs = [
        Thumb::new("Black").face(gray(20)).split().order(ordered(None, Some(2))),
        Thumb::new("Black").face(gray(20)).order(ordered(Some(0), Some(2))),
        Thumb::new("White").face(gray(240)).split().order(ordered(Some(1), None)),
        Thumb::new("White").face(gray(240)).order(ordered(Some(1), None)),
    ];
    ThumbRow::new(value, LEVELS).step(1.0).thumbs(thumbs)
}

#[story(ThumbRow<'static, 4>, covers = [Default], drive = Drive::Step)]
fn thumb_row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    levels(Bind::bound(data.value("levels", Mixed::each([30.0, 60.0, 190.0, 220.0])))).label("Levels").show(ui).into()
}

#[story(ThumbRow<'static, 4>, covers = [], drive = Drive::Drag(Grip::new(super::ROW)))]
fn thumb_row_drag(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    levels(Bind::bound(data.value("levels", Mixed::each([30.0, 60.0, 190.0, 220.0])))).label("Levels").show(ui).into()
}

#[story(ThumbRow<'static, 4>, covers = [], drive = Drive::Drag(Grip::new(Thumb::PART)))]
fn thumb_row_wrapping(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let thumbs = [
        Thumb::new("Hue from").face(DataColor::new(Color32::from_rgb(220, 60, 60))),
        Thumb::new("Hue to").face(DataColor::new(Color32::from_rgb(220, 160, 60))),
        Thumb::new("Hue to").face(DataColor::new(Color32::from_rgb(60, 160, 220))),
        Thumb::new("Hue to").face(DataColor::new(Color32::from_rgb(160, 60, 220))),
    ];
    ThumbRow::new(Bind::bound(data.value("hues", Mixed::each([330.0, 40.0, 120.0, 250.0]))), HUES)
        .step(1.0)
        .wrap()
        .thumbs(thumbs)
        .label("Hues")
        .show(ui)
        .into()
}

#[story(ThumbRow<'static, 4>, covers = [], drive = Drive::Never)]
fn thumb_row_hue_range(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let thumbs = [
        Thumb::new("Range outer start").face(gray(150)),
        Thumb::new("Range inner start").face(gray(230)),
        Thumb::new("Range inner end").face(gray(230)),
        Thumb::new("Range outer end").face(gray(150)),
    ];
    ThumbRow::new(Bind::bound(data.value("range", Mixed::each([340.0, 10.0, 50.0, 80.0]))), HUES)
        .step(1.0)
        .anchor(Anchor::new(0, 359.0))
        .thumbs(thumbs)
        .label("Hue range")
        .show(ui)
        .into()
}

#[story(ThumbRow<'static, 4>, covers = [Mixed], drive = Drive::Never)]
fn thumb_row_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    levels(Bind::bound_mixed(data.value("mixed", Mixed::<[Mixed<f64>; 4]>::Mixed))).label("Levels").show(ui).into()
}

#[story(ThumbRow<'static, 4>, covers = [], drive = Drive::Never)]
fn thumb_row_part_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let row = [Mixed::Same(30.0), Mixed::Mixed, Mixed::Same(190.0), Mixed::Mixed];
    levels(Bind::bound(data.value("part-mixed", row))).label("Levels").show(ui).into()
}

#[story(ThumbRow<'static, 4>, covers = [Invalid], drive = Drive::Never)]
fn thumb_row_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_over = |row: &[Mixed<f64>; 4]| {
        if row[0].same().is_some_and(|black| *black > LIMIT) { Validity::Refuse(Message::new("Black is above the limit")) } else { Validity::Ok }
    };
    levels(Bind::bound(data.value("over", Mixed::each([150.0, 160.0, 190.0, 220.0]))).check(refuse_over)).label("Levels").show(ui).into()
}

#[story(ThumbRow<'static, 4>, covers = [], drive = Drive::Never)]
fn thumb_row_track_face(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    levels(Bind::bound(data.value("levels", Mixed::each([30.0, 60.0, 190.0, 220.0]))))
        .track_face(TrackFace::new(Progress::new(1.0).label("Track")))
        .label("Levels")
        .show(ui)
        .into()
}
