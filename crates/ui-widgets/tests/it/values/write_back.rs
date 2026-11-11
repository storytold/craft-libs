//! A host that stores a proposal at the field's precision has not changed the document under the gesture.

use crate::label::kit::Field;
use craft_behavior::{CancelReason, EditEvent, NumberCodec, Reader};
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use craft_ui::text::NumberField;
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;

const ROW: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(310.0, 40.0) };

const START: f64 = 10.0;

/// The decimals of the field's codec.
const DECIMALS: usize = 2;

/// How far the host moves a value it overwrites.
const OVERWRITE: f64 = 5.0;

/// What the host stores for a proposal: its value as the float32 the document keeps, at the precision of the field.
fn stored(value: f64) -> f64 {
    f64::from(value as f32)
}

/// Scrubs a given-bound field by its label; the host applies each event after `host` changed what it stores.
/// Returns the cancel reasons and whether a commit arrived.
fn scrub(host: impl Fn(f64) -> f64) -> (Vec<CancelReason>, bool) {
    let mut frames = Frames::new();
    let mut doc = Some(START);
    let cancels = RefCell::new(Vec::new());
    let committed = RefCell::new(false);
    let mut show = |ui: &mut Ui| {
        Field::new("Size", SideLabel::new()).id_salt("row").show_at(ui, ROW, |ui, _| {
            let codec = NumberCodec::new(Reader::new(), DECIMALS);
            let out = NumberField::new("Size", Bind::given(doc)).codec(codec).scrub_by_label().id_salt("scrubbed").show(ui);
            for event in &out.events {
                match event {
                    EditEvent::Update { value, .. } => doc = value.map(&host),
                    EditEvent::Commit { after, .. } => {
                        doc = after.map(&host);
                        *committed.borrow_mut() = true;
                    }
                    EditEvent::Cancel { reason, .. } => cancels.borrow_mut().push(*reason),
                    _ => {}
                }
            }
        });
    };
    frames.idle(4, &mut show);
    let from = ROW.min + Vec2::new(4.0, 12.0);
    frames.frame(vec![moved(from)], &mut show);
    frames.frame(vec![press(from)], &mut show);
    for travel in [20.0_f32, 23.37, 29.173, 36.357] {
        frames.frame(vec![moved(from + Vec2::new(travel, 0.0))], &mut show);
    }
    frames.frame(vec![release(from + Vec2::new(36.357, 0.0))], &mut show);
    frames.idle(2, &mut show);
    (cancels.take(), committed.take())
}

/// A drag whose proposals the host stores at a float32's precision ends with a commit; a host that writes a different
/// value cancels with `HostChanged`.
#[trial]
fn a_write_back_at_the_codecs_precision_is_the_gestures_own() {
    let (cancels, committed) = scrub(stored);
    assert!(cancels.is_empty() && committed, "rounded write-back: cancels {cancels:?}, committed {committed}");
    let (cancels, _) = scrub(|value| value + OVERWRITE);
    assert_eq!(cancels, vec![CancelReason::HostChanged], "a different value is a host change");
}
