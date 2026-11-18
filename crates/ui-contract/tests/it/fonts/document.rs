//! Document faces: registering again after the definitions are replaced, and a face's chain of sibling subsets.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use craft_behavior::Metrics;
use craft_ui::contract::{Common, Component, Cx, FacesCx as _, MeasureCx, Props as _, Role, Show as _};
use craft_ui::fonts::{self, FaceBytes, FaceState};
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::testing::{Frames, trial};
use egui::{FontId, Response, Vec2};

const FONT: &[u8] = include_bytes!("../../fonts/NotoSans-test.ttf");
const ARABIC: &[u8] = include_bytes!("../../fonts/NotoSansArabic-test.ttf");

/// The frames a face may take to become ready.
const FRAMES_TO_READY: usize = 20;

#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PlainStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What the probes asked for: how often a face was loaded and the state each key was last given.
#[derive(Clone, Default)]
struct Tally {
    loads: Arc<AtomicUsize>,
    states: Arc<Mutex<HashMap<String, FaceState>>>,
}

impl Tally {
    fn loads(&self) -> usize {
        self.loads.load(Ordering::Relaxed)
    }

    fn state(&self, key: &str) -> FaceState {
        self.states.lock().expect("the states").get(key).cloned().unwrap_or_else(|| panic!("{key} was asked for"))
    }
}

/// An owner of document faces: it asks for its keys while it is measured.
#[derive(Component)]
struct Owner {
    #[common]
    common: Common,
    keys: Vec<String>,
    loads_fail: bool,
    /// The face each key loads.
    face: fn(&str) -> FaceBytes,
    tally: Tally,
}

/// Every key loads the Latin test face.
fn latin(_: &str) -> FaceBytes {
    FaceBytes::new(Arc::from(FONT), 0)
}

impl Component for Owner {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: OwnerStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        for key in &self.keys {
            let state = cx.document_face(key, || {
                self.tally.loads.fetch_add(1, Ordering::Relaxed);
                (!self.loads_fail).then(|| (self.face)(key))
            });
            self.tally.states.lock().expect("the states").insert(key.clone(), state);
        }
        Vec2::splat(1.0)
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

fn keys(range: std::ops::Range<usize>) -> Vec<String> {
    range.map(|n| format!("k{n}")).collect()
}

/// One frame in which each `(salt, keys)` owner asks for its keys.
fn frame(frames: &mut Frames, tally: &Tally, owners: &[(&'static str, Vec<String>)], loads_fail: bool) {
    frame_of(frames, tally, owners, loads_fail, latin);
}

/// One frame in which each `(salt, keys)` owner asks for its keys, each loading `face`.
fn frame_of(frames: &mut Frames, tally: &Tally, owners: &[(&'static str, Vec<String>)], loads_fail: bool, face: fn(&str) -> FaceBytes) {
    frames.frame(vec![], |ui| {
        for (salt, keys) in owners {
            Owner { common: Common::new(), keys: keys.clone(), loads_fail, face, tally: tally.clone() }.id_salt(*salt).show(ui);
        }
    });
}

/// Frames until every key of `owners` is ready.
fn until_ready(frames: &mut Frames, tally: &Tally, owners: &[(&'static str, Vec<String>)]) {
    until_ready_of(frames, tally, owners, latin);
}

/// Frames until every key of `owners`, each loading `face`, is ready.
fn until_ready_of(frames: &mut Frames, tally: &Tally, owners: &[(&'static str, Vec<String>)], face: fn(&str) -> FaceBytes) {
    for _ in 0..FRAMES_TO_READY {
        frame_of(frames, tally, owners, false, face);
        if owners.iter().flat_map(|(_, keys)| keys).all(|key| matches!(tally.state(key), FaceState::Ready(_))) {
            return;
        }
    }
    panic!("the faces did not become ready in {FRAMES_TO_READY} frames");
}

/// A stack install sets the whole definitions and drops the faces added one by one: the registry notices and adds
/// them again, within the budget.
#[trial]
fn faces_are_registered_again_after_the_definitions_are_replaced() {
    let (mut frames, tally) = (Frames::new(), Tally::default());
    let owners = [("owner", keys(0..3))];
    until_ready(&mut frames, &tally, &owners);
    assert_eq!(tally.loads(), 3);

    fonts::load(&frames.ctx, craft_ui::fonts::deflated_default_definitions());
    frame(&mut frames, &tally, &owners, false);
    assert!(keys(0..3).iter().all(|key| tally.state(key) == FaceState::Loading), "the replaced faces wait for their registration");
    assert_eq!(tally.loads(), 6, "each face loads again");
    until_ready(&mut frames, &tally, &owners);
    assert_eq!(tally.loads(), 6);
}

/// A Latin subset chained to an Arabic one ("split"), the Arabic subset, and the Latin subset alone.
fn split(key: &str) -> FaceBytes {
    match key {
        "split" => FaceBytes::new(Arc::from(FONT), 0).chain(["arabic"]),
        "arabic" => FaceBytes::new(Arc::from(ARABIC), 0),
        _ => FaceBytes::new(Arc::from(FONT), 0),
    }
}

/// A face split over subsets (Word) draws a character its own subset lacks from the subset its chain names, not from
/// the interface family; the same subset without a chain has no glyph for it.
#[trial]
fn a_chained_face_draws_what_it_lacks_from_its_sibling_subset() {
    let (mut frames, tally) = (Frames::new(), Tally::default());
    let owners = [("owner", vec!["split".to_owned(), "arabic".to_owned(), "alone".to_owned()])];
    until_ready_of(&mut frames, &tally, &owners, split);
    // The chains are linked at the start of a pass, after the faces are in.
    frame_of(&mut frames, &tally, &owners, false, split);
    let draws = |key: &str| {
        let FaceState::Ready(face) = tally.state(key) else { panic!("{key} is ready") };
        frames.ctx.fonts_mut(|fonts| fonts.has_glyph(&FontId::new(14.0, face.family()), 'ع'))
    };
    assert!(!draws("alone"), "the Latin subset alone has no Arabic glyph");
    assert!(draws("split"), "the chain draws the Arabic glyph from the sibling subset");
}
