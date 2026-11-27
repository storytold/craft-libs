//! Document faces read from a catalog: loaded off the pass, ready from the pass after, kept across a definitions
//! replacement and while drawn lately (a settled view's replay counts), served to egui without making its fonts again,
//! and held within one budget; a face's chain of sibling subsets; and a run at variation coordinates of its face.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread::ThreadId;

use craft_behavior::Metrics;
use craft_host::{AppInfo, Host, ManualJobs};
use craft_ui::contract::{Common, Component, Cx, FacesCx, MeasureCx, MemoView, Role, Show as _};
use craft_ui::fonts::{self, FaceBytes, FaceState};
use craft_ui::primitives::{DocRun, DocText};
use craft_ui::style::{ComponentStyle, DataColor, StyleCx};
use craft_ui::testing::{Frames, MemoRun, document_face_budget, last_frame_memo_runs, trial};
use egui::{Color32, FontId, Response, Vec2};

const FONT: &[u8] = include_bytes!("../../fonts/NotoSans-test.ttf");
/// A variable font (`wght`, 400 to 900).
const VARIABLE: &[u8] = include_bytes!("../../../../font-subset/tests/data/orbitron/Orbitron[wght].ttf");
const ARABIC: &[u8] = include_bytes!("../../fonts/NotoSansArabic-test.ttf");

#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PlainStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What the probes asked for: how often a face was loaded, on which threads, and the state each key was last given.
#[derive(Clone, Default)]
struct Tally {
    loads: Arc<AtomicUsize>,
    threads: Arc<Mutex<Vec<ThreadId>>>,
    states: Arc<Mutex<HashMap<String, FaceState>>>,
    /// The widths of the same run in the face "variable" at its default, at `wght` 400 and at 900.
    widths: Arc<Mutex<Vec<f32>>>,
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

/// The face `key` loads: "split" is a Latin subset chained to the Arabic subset "arabic", "variable" a variable
/// font; every other key loads the Latin subset alone.
fn face_of(key: &str) -> FaceBytes {
    match key {
        "split" => FaceBytes::new(Arc::from(FONT), 0).chain(["arabic"]),
        "arabic" => FaceBytes::new(Arc::from(ARABIC), 0),
        "variable" => FaceBytes::new(Arc::from(VARIABLE), 0),
        _ => FaceBytes::new(Arc::from(FONT), 0),
    }
}

/// Asks for the face `key`, whose load counts itself in `tally`; a key starting with `panic` names a load that panics.
fn ask(cx: &impl FacesCx, key: &str, tally: &Tally) -> FaceState {
    cx.document_face(key, || {
        let (loads, threads, panics) = (Arc::clone(&tally.loads), Arc::clone(&tally.threads), key.starts_with("panic"));
        let face = face_of(key);
        move || {
            loads.fetch_add(1, Ordering::Relaxed);
            threads.lock().expect("the threads").push(std::thread::current().id());
            assert!(!panics, "a load that panics");
            Some(face)
        }
    })
}

/// An owner of document faces: it asks for its keys while it is measured, and lays text out in each ready one; or,
/// with `in_run`, asks for them in its run only.
#[derive(Component)]
struct Owner {
    #[common]
    common: Common,
    keys: Vec<String>,
    tally: Tally,
    in_run: bool,
}

impl Component for Owner {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: OwnerStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        if self.in_run {
            return Vec2::splat(1.0);
        }
        for key in &self.keys {
            let state = ask(cx, key, &self.tally);
            if let FaceState::Ready(font) = &state {
                let runs = [DocRun::new("Ag", font.clone(), 14.0, DataColor::new(Color32::BLACK))];
                let _ = cx.layout_doc(&DocText::new(&runs));
                if key == "variable" {
                    let at = [vec![], vec![(*b"wght", 400.0)], vec![(*b"wght", 900.0)]];
                    let width = |coords: &Vec<([u8; 4], f32)>| {
                        let runs = [DocRun::new("Hamburg", font.clone(), 100.0, DataColor::new(Color32::BLACK)).coords(coords)];
                        cx.layout_doc(&DocText::new(&runs)).size().x
                    };
                    *self.tally.widths.lock().expect("the widths") = at.iter().map(width).collect();
                }
            }
            self.tally.states.lock().expect("the states").insert(key.clone(), state);
        }
        Vec2::splat(1.0)
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        if self.in_run {
            for key in &self.keys {
                let state = ask(&cx, key, &self.tally);
                self.tally.states.lock().expect("the states").insert(key.clone(), state);
            }
        }
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
        Owner { common: Common::new(), keys: keys.to_vec(), tally: tally.clone(), in_run: false }.show(ui);
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
fn a_run_is_laid_out_at_its_variation_coordinates() {
    let (mut frames, jobs, tally) = rig();
    shown(&mut frames, &jobs, &tally, &["variable".to_owned()]);
    let widths = tally.widths.lock().expect("the widths").clone();
    let [plain, regular, black] = widths[..] else { panic!("{widths:?}") };
    assert_ne!(black, regular, "the run is laid out at its coordinates");
    assert_eq!(plain, regular, "the face's default weight");
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

/// Without a host the loads run on the jobs the runtime was installed with, inline in a trial: every face asked for in
/// one pass is ready in the next on every machine, each loaded once.
#[trial]
fn without_a_host_every_face_asked_for_in_one_pass_is_ready_in_the_next() {
    let mut frames = Frames::new();
    let tally = Tally::default();
    let wanted = keys(0..8);
    frame(&mut frames, &tally, &wanted);
    frame(&mut frames, &tally, &wanted);
    assert!(tally.all_ready(&wanted), "every face lands in the next pass");
    assert_eq!(tally.loads(), 8, "and none was cancelled and loaded again");
    let me = std::thread::current().id();
    assert!(tally.threads.lock().expect("the threads").iter().all(|thread| *thread == me), "the loads ran on the trial's thread");
}

#[trial]
fn a_load_that_panics_leaves_its_face_unavailable_and_asks_for_no_more_passes() {
    let (mut frames, jobs, tally) = rig();
    let wanted = vec!["panic".to_owned()];
    frame(&mut frames, &tally, &wanted);
    jobs.run_all();
    frame(&mut frames, &tally, &wanted);
    assert_eq!(tally.state("panic"), FaceState::Unavailable);
    frame(&mut frames, &tally, &wanted);
    assert!(!frames.repaint_requested(), "the row stops asking for passes");
}

/// The tally of the memoized view's owner: its `show` is a function, so it reaches the tally by name.
static MEMOIZED: LazyLock<Tally> = LazyLock::new(Tally::default);

fn memoized(_: &mut u32, ui: &mut egui::Ui) {
    Owner { common: Common::new(), keys: vec!["m0".to_owned(), "m1".to_owned()], tally: MEMOIZED.clone(), in_run: true }.show(ui);
}

/// One frame of the memoized view beside an owner that asks for `keys`.
fn memo_frame(frames: &mut Frames, tally: &Tally, keys: &[String]) {
    frames.frame(vec![], |ui| {
        MemoView::new("faces", 1u32, memoized).show(ui);
        Owner { common: Common::new(), keys: keys.to_vec(), tally: tally.clone(), in_run: false }.show(ui);
    });
}

#[trial]
fn a_settled_view_that_replays_keeps_its_faces() {
    let (mut frames, jobs, tally) = rig();
    let font = FONT.len();
    document_face_budget(&frames.ctx, 3 * font);
    memo_frame(&mut frames, &tally, &[]);
    jobs.run_all();
    for _ in 0..3 {
        memo_frame(&mut frames, &tally, &[]);
    }
    assert!(MEMOIZED.all_ready(&["m0".to_owned(), "m1".to_owned()]));
    let mut replayed = 0;
    for start in (0..8).step_by(2) {
        let others = keys(start..start + 2);
        memo_frame(&mut frames, &tally, &others);
        jobs.run_all();
        memo_frame(&mut frames, &tally, &others);
        replayed += last_frame_memo_runs(&frames.ctx).iter().filter(|(_, _, run)| *run == MemoRun::Replayed).count();
    }
    assert!(replayed > 0, "the view replays while the other faces push the registry past its budget");
    frame(&mut frames, &tally, &["m0".to_owned(), "m1".to_owned()]);
    assert!(tally.all_ready(&["m0".to_owned(), "m1".to_owned()]), "the faces the replays showed are kept");
    assert_eq!(MEMOIZED.loads(), 2, "and were loaded once");
}

#[trial]
fn past_the_budget_the_faces_drawn_longest_ago_go_and_the_fonts_are_made_again_once() {
    let (mut frames, jobs, tally) = rig();
    let font = FONT.len();
    document_face_budget(&frames.ctx, 6 * font);
    // `keep` is drawn in every frame; one more face joins it each step.
    let step = |frames: &mut Frames, key: &str| {
        let wanted = vec!["keep".to_owned(), key.to_owned()];
        shown(frames, &jobs, &tally, &wanted);
        generation(frames)
    };
    let before = step(&mut frames, "k0");
    let filling: Vec<u64> = ["k1", "k2", "k3", "k4"].iter().map(|key| step(&mut frames, key)).collect();
    assert!(filling.iter().all(|g| *g == before), "within the budget nothing is taken out");
    let after = step(&mut frames, "k5");
    assert_ne!(after, before, "past it, egui's fonts are made again to free what went");
    for _ in 0..3 {
        frame(&mut frames, &tally, &["keep".to_owned(), "k5".to_owned()]);
    }
    assert_eq!(generation(&frames), after, "once, not every frame");
    let all: Vec<String> = ["keep", "k0", "k1", "k2", "k3", "k4", "k5"].iter().map(|key| (*key).to_owned()).collect();
    frame(&mut frames, &tally, &all);
    let ready = |key: &str| matches!(tally.state(key), FaceState::Ready(_));
    assert!(ready("keep"), "a face drawn in the last frame stays, the first loaded included");
    assert!(["k0", "k1", "k2", "k3"].iter().all(|key| !ready(key)), "the faces drawn longest ago went");
    assert!(ready("k4") && ready("k5"), "down to half the budget, the faces drawn last stay");
}

/// Whether the ready face `key` draws an Arabic letter.
fn draws_arabic(frames: &Frames, tally: &Tally, key: &str) -> bool {
    let FaceState::Ready(face) = tally.state(key) else { panic!("{key} is ready") };
    frames.ctx.fonts_mut(|fonts| fonts.has_glyph(&FontId::new(14.0, face.family()), '\u{639}'))
}

/// A face split over subsets (Word) draws a character its own subset lacks from the subset its chain names, not from
/// the interface family; the same subset without a chain has no glyph for it.
#[trial]
fn a_chained_face_draws_what_it_lacks_from_its_sibling_subset() {
    let (mut frames, jobs, tally) = rig();
    let wanted: Vec<String> = ["split", "arabic", "alone"].iter().map(|key| (*key).to_owned()).collect();
    shown(&mut frames, &jobs, &tally, &wanted);
    frame(&mut frames, &tally, &wanted);
    assert!(!draws_arabic(&frames, &tally, "alone"), "the Latin subset alone has no Arabic glyph");
    assert!(draws_arabic(&frames, &tally, "split"), "the chain draws the Arabic glyph from the sibling subset");
}

/// A sibling subset that lands after the face it follows was laid out still joins its chain.
#[trial]
fn a_sibling_that_lands_late_joins_the_chain() {
    let (mut frames, jobs, tally) = rig();
    shown(&mut frames, &jobs, &tally, &["split".to_owned()]);
    assert!(!draws_arabic(&frames, &tally, "split"), "no sibling yet");
    let both = vec!["split".to_owned(), "arabic".to_owned()];
    shown(&mut frames, &jobs, &tally, &both);
    frame(&mut frames, &tally, &both);
    assert!(draws_arabic(&frames, &tally, "split"), "the late sibling draws the Arabic glyph");
}
