//! A select's measure lays out each face's text once per ask: the whole line, and at its least or in a fitted box the
//! cut it elides to.

use craft_behavior::Proposal;
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, Sizing};
use craft_ui::pickers::Select;
use craft_ui::primitives::TextRun;
use craft_ui::testing::{Frames, last_pass_text_layouts, trial};

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
