//! Document faces read from a catalog: loaded off the pass, ready from the pass after, kept across a definitions
//! replacement and while drawn lately, and served to egui without making its fonts again.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use craft_behavior::Metrics;
use craft_host::{AppInfo, Host, ManualJobs};
use craft_ui::contract::{Common, Component, Cx, FacesCx as _, MeasureCx, Role, Show as _};
use craft_ui::fonts::{self, FaceBytes, FaceState};
use craft_ui::primitives::{DocRun, DocText};
use craft_ui::style::{ComponentStyle, DataColor, StyleCx};
use craft_ui::testing::{Frames, trial};
use egui::{Color32, Response, Vec2};

const FONT: &[u8] = include_bytes!("../../fonts/NotoSans-test.ttf");

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

    fn all_ready(&self, keys: &[String]) -> bool {
        keys.iter().all(|key| matches!(self.state(key), FaceState::Ready(_)))
    }
}

/// An owner of document faces: it asks for its keys while it is measured, and lays text out in each ready one.
#[derive(Component)]
struct Owner {
    #[common]
    common: Common,
    keys: Vec<String>,
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
            let loads = Arc::clone(&self.tally.loads);
            let state = cx.document_face(key, move || {
                loads.fetch_add(1, Ordering::Relaxed);
                Some(FaceBytes::new(Arc::from(FONT), 0))
            });
            if let FaceState::Ready(font) = &state {
                let runs = [DocRun::new("Ag", font.clone(), 14.0, DataColor::new(Color32::BLACK))];
                let _ = cx.layout_doc(&DocText::new(&runs));
            }
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

/// Frames whose host runs background jobs only when the trial says so.
fn rig() -> (Frames, Arc<ManualJobs>, Tally) {
    let jobs = Arc::new(ManualJobs::new());
    let host = Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).jobs(jobs.clone()).build());
    let mut frames = Frames::new();
    frames.host(host);
    (frames, jobs, Tally::default())
}

/// One frame in which an owner asks for `keys`.
fn frame(frames: &mut Frames, tally: &Tally, keys: &[String]) {
    frames.frame(vec![], |ui| {
        Owner { common: Common::new(), keys: keys.to_vec(), tally: tally.clone() }.show(ui);
    });
}

/// Asks for `keys`, runs the loads and shows the frame after: every key is ready.
fn shown(frames: &mut Frames, jobs: &ManualJobs, tally: &Tally, keys: &[String]) {
    frame(frames, tally, keys);
    jobs.run_all();
    frame(frames, tally, keys);
    assert!(tally.all_ready(keys), "the faces are ready in the pass after their loads");
}

/// The atlas generation, after the owners laid text out in their ready faces: egui makes its fonts again, and a new
/// atlas with them, when a face changes its definitions.
fn generation(frames: &Frames) -> u64 {
    frames.ctx.fonts(|fonts| fonts.atlas_generation())
}

#[trial]
fn a_catalog_face_loads_off_the_pass_and_is_ready_in_the_pass_after_its_load() {
    let (mut frames, jobs, tally) = rig();
    let wanted = keys(0..3);
    frame(&mut frames, &tally, &wanted);
    frame(&mut frames, &tally, &wanted);
    assert_eq!(tally.loads(), 0, "no pass loads a face itself");
    assert!(wanted.iter().all(|key| tally.state(key) == FaceState::Loading));
    jobs.run_all();
    frame(&mut frames, &tally, &wanted);
    assert!(tally.all_ready(&wanted));
    assert_eq!(tally.loads(), 3, "each face loads once, however many passes asked");
}

#[trial]
fn faces_arriving_keep_the_fonts_egui_made() {
    let (mut frames, jobs, tally) = rig();
    let first = keys(0..2);
    shown(&mut frames, &jobs, &tally, &first);
    let before = generation(&frames);
    let more = keys(0..8);
    shown(&mut frames, &jobs, &tally, &more);
    frame(&mut frames, &tally, &more);
    assert_eq!(generation(&frames), before, "the new faces join the fonts egui has, which it keeps");
}

#[trial]
fn scrolling_back_over_faces_drawn_before_loads_none_again() {
    let (mut frames, jobs, tally) = rig();
    for start in (0..120).step_by(20) {
        shown(&mut frames, &jobs, &tally, &keys(start..start + 20));
    }
    assert_eq!(tally.loads(), 120);
    frame(&mut frames, &tally, &keys(0..20));
    assert!(tally.all_ready(&keys(0..20)), "the first faces are still kept");
    assert_eq!(tally.loads(), 120, "and load nothing again");
}

#[trial]
fn faces_stay_ready_after_the_definitions_are_replaced() {
    let (mut frames, jobs, tally) = rig();
    let wanted = keys(0..3);
    shown(&mut frames, &jobs, &tally, &wanted);
    fonts::load(&frames.ctx, craft_ui::fonts::deflated_default_definitions());
    frame(&mut frames, &tally, &wanted);
    assert!(tally.all_ready(&wanted), "egui finds the served faces again");
    assert_eq!(tally.loads(), 3, "without loading them again");
}
