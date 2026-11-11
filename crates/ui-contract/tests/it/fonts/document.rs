//! Document faces: registering again after the definitions are replaced.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use craft_behavior::Metrics;
use craft_ui::contract::{Common, Component, Cx, FacesCx as _, MeasureCx, Props as _, Role, Show as _};
use craft_ui::fonts::{self, FaceBytes, FaceState};
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::testing::{Frames, trial};
use egui::{Response, Vec2};

const FONT: &[u8] = include_bytes!("../../fonts/NotoSans-test.ttf");

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
    tally: Tally,
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
                (!self.loads_fail).then(|| FaceBytes::new(Arc::from(FONT), 0))
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
    frames.frame(vec![], |ui| {
        for (salt, keys) in owners {
            Owner { common: Common::new(), keys: keys.clone(), loads_fail, tally: tally.clone() }.id_salt(*salt).show(ui);
        }
    });
}

/// Frames until every key of `owners` is ready.
fn until_ready(frames: &mut Frames, tally: &Tally, owners: &[(&'static str, Vec<String>)]) {
    for _ in 0..FRAMES_TO_READY {
        frame(frames, tally, owners, false);
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
