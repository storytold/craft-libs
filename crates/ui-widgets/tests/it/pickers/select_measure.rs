//! A select's measure lays out each face's text once per ask: the whole line, and at its least or in a fitted box the
//! cut it elides to. The measure call is the layout memory's record of the select: its faces keep no entries of their
//! own, so a face change that leaves the select's size alone asks for no discard pass, and one that changes it shows in
//! its frame.

use craft_behavior::Proposal;
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::pickers::Select;
use craft_ui::primitives::TextRun;
use craft_ui::testing::{Frames, kept_measurement_leaves, last_pass_text_layouts, trial};
use egui::Rect;

/// The blend modes of a brush's Mode select.
const MODES: [&str; 29] = [
    "Normal",
    "Dissolve",
    "Behind",
    "Clear",
    "Darken",
    "Multiply",
    "Color Burn",
    "Linear Burn",
    "Darker Color",
    "Lighten",
    "Screen",
    "Color Dodge",
    "Linear Dodge",
    "Lighter Color",
    "Overlay",
    "Soft Light",
    "Hard Light",
    "Vivid Light",
    "Linear Light",
    "Pin Light",
    "Hard Mix",
    "Difference",
    "Exclusion",
    "Subtract",
    "Divide",
    "Hue",
    "Saturation",
    "Color",
    "Luminosity",
];

/// The text layouts of one measure ask of a select of `labels` at `proposal`.
fn layouts(labels: &[&'static str], sizing: Option<Sizing>, proposal: Proposal) -> u32 {
    let entries: Vec<Entry<u8>> = labels.iter().zip(0u8..).map(|(label, at)| Entry::new(at, *label)).collect();
    let mut frames = Frames::new();
    let passes = frames.frame(vec![], |ui| {
        let select = Select::new("Mode", Bind::given(0u8), &entries);
        let mut select = match sizing {
            Some(sizing) => select.sizing(sizing),
            None => select,
        };
        select.measure_in(ui, proposal);
    });
    assert_eq!(passes, 1, "a frame that only measures takes one pass");
    last_pass_text_layouts(&frames.ctx)
}

/// With 29 entries, one ask lays out each entry's face once more than with 3: its whole line at `Ideal`, and at
/// `Min` or in a fitted box its whole line and, for a label longer than its readable least, the cut.
#[trial]
fn a_select_measure_lays_out_each_face_once() {
    let added = &MODES[3..];
    let cuts = added.iter().filter(|label| label.chars().count() > TextRun::READABLE_CHARACTERS + 1).count() as u32;
    let lines = added.len() as u32;
    let more = |sizing, proposal| layouts(&MODES, sizing, proposal) - layouts(&MODES[..3], sizing, proposal);
    assert_eq!(more(None, Proposal::Ideal), lines, "an Ideal ask lays out each face's line once");
    assert_eq!(more(None, Proposal::Min), lines + cuts, "a Min ask lays out each face's line and its cut once");
    assert_eq!(more(Some(Sizing::Content), Proposal::Ideal), lines + cuts, "a fitted box lays out each face's line and its cut once");
}

/// How far two widths may differ and still count as the same, in points.
const SLACK: f32 = 0.5;
/// The widest entry: wider than a field, so it sets the select's width.
const WIDEST: &str = "Linear Dodge, the widest blend mode name of them all";

/// The blend modes with the widest entry at index 5.
fn modes() -> Vec<&'static str> {
    let mut labels = MODES.to_vec();
    labels[5] = WIDEST;
    labels
}

/// A content-sized stack in a row, holding a select of `labels` (or a button in its place), with a button after it.
/// Returns the select's rect.
fn scene(ui: &mut egui::Ui, labels: &[&'static str], select: bool) -> Rect {
    let mut rect = Rect::NOTHING;
    Row::new().show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            if select {
                let entries: Vec<Entry<u8>> = labels.iter().zip(0u8..).map(|(label, at)| Entry::new(at, *label)).collect();
                rect = Select::new("Mode", Bind::given(1u8), &entries).show(ui).response.rect;
            } else {
                Button::new("Mode").show(ui);
            }
        });
        Button::new("Link").show(ui);
    });
    rect
}

/// The select's width once settled with `labels`.
fn settled_width(labels: &[&'static str]) -> f32 {
    let mut frames = Frames::new();
    let mut rect = Rect::NOTHING;
    frames.idle(30, |ui| rect = scene(ui, labels, true));
    rect.width()
}

/// A settled select whose first entry widens, still narrower than its widest, takes its frame in one pass: the change
/// leaves the select's size alone, so no kept measurement is dropped.
#[trial]
fn a_face_change_that_leaves_the_select_size_alone_asks_for_no_discard_pass() {
    let mut labels = modes();
    let mut frames = Frames::new();
    let mut rect = Rect::NOTHING;
    frames.idle(30, |ui| rect = scene(ui, &labels, true));
    let before = rect.width();
    // Entry 0 is the first face `Select::measure` visits: with a face leaf, the transcript kept that face's call, so
    // this edit was the one that caused a discard pass.
    labels[0] = "Normal, wider than a field yet narrower";
    let passes = frames.frame(vec![], |ui| rect = scene(ui, &labels, true));
    assert!((rect.width() - before).abs() <= SLACK, "the wider first entry leaves the select's width alone: {before} {}", rect.width());
    assert_eq!(passes, 1, "a face change that leaves the select's size alone takes one pass");
}

/// A label change at any index that widens the select shows in its frame, at the width a fresh select of those labels
/// settles at.
#[trial]
fn a_label_change_that_widens_the_select_shows_in_its_frame() {
    for at in [0, 7, 28] {
        let mut labels = modes();
        let mut frames = Frames::new();
        let mut rect = Rect::NOTHING;
        frames.idle(30, |ui| rect = scene(ui, &labels, true));
        let before = rect.width();
        labels[at] = "Linear Dodge, a blend mode name wider still than the widest of them all";
        frames.frame(vec![], |ui| rect = scene(ui, &labels, true));
        let fresh = settled_width(&labels);
        assert!(fresh > before + SLACK, "the new label at {at} widens the select: {before} {fresh}");
        assert!((rect.width() - fresh).abs() <= SLACK, "the label change at {at} shows in its frame: {} {fresh}", rect.width());
    }
}

/// The layout memory keeps one entry for a select, as for a button in its place: its 29 faces keep none.
#[trial]
fn a_select_keeps_one_layout_memory_entry() {
    let leaves = |select: bool| {
        let labels = modes();
        let mut frames = Frames::new();
        frames.idle(30, |ui| {
            scene(ui, &labels, select);
        });
        kept_measurement_leaves(&frames.ctx)
    };
    let with_button = leaves(false);
    assert!(with_button > 0, "the scene keeps measurements");
    assert_eq!(leaves(true), with_button, "a select keeps one entry, as a button does");
}
