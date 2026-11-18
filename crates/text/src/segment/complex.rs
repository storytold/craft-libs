//! Word breaks inside Thai, Lao, Khmer and Burmese runs. UAX #14 gives these scripts class SA
//! ("complex context") and leaves breaking inside them to a dictionary; ICU4X's segmenter supplies
//! it, with its dictionaries or, with the `lstm` feature, its LSTM models. Only runs of the scripts
//! whose language feature is on are segmented.

use icu_segmenter::options::LineBreakOptions;
use icu_segmenter::{LineSegmenter, LineSegmenterBorrowed};
use unicode_linebreak::{BreakClass, break_property};

/// Builds ICU4X's line segmenter with one model for the complex scripts. The models are static
/// data; building the segmenter only looks them up.
pub(super) type Model = fn(LineBreakOptions<'static>) -> LineSegmenterBorrowed<'static>;

/// The model `line_breaks` uses: the LSTM models with the `lstm` feature, else the dictionaries.
#[cfg(feature = "lstm")]
pub(super) const MODEL: Model = LineSegmenter::new_lstm;
#[cfg(not(feature = "lstm"))]
pub(super) const MODEL: Model = LineSegmenter::new_dictionary;

/// Byte offsets inside `text` where a run of an enabled script may break between words, in order.
/// Run starts and ends are left to UAX #14.
pub(super) fn word_breaks(text: &str, model: Model) -> Vec<usize> {
    let mut out = Vec::new();
    let mut segmenter = None;
    let mut run_start = None;
    let ends = text.char_indices().map(|(i, c)| (i, segmented(c))).chain(std::iter::once((text.len(), false)));
    for (i, inside) in ends {
        match (run_start, inside) {
            (None, true) => run_start = Some(i),
            (Some(start), false) => {
                run_start = None;
                let Some(run) = text.get(start..i) else { continue };
                let seg = *segmenter.get_or_insert_with(|| model(LineBreakOptions::default()));
                out.extend(seg.segment_str(run).filter(|&o| o > 0 && o < run.len()).map(|o| start + o));
            }
            _ => {}
        }
    }
    out
}

/// A character of an enabled script that UAX #14 leaves to the word segmenter.
fn segmented(c: char) -> bool {
    let u = c as u32;
    let enabled = (cfg!(feature = "thai") && matches!(u, 0x0E00..=0x0E7F))
        || (cfg!(feature = "lao") && matches!(u, 0x0E80..=0x0EFF))
        || (cfg!(feature = "khmer") && matches!(u, 0x1780..=0x17FF | 0x19E0..=0x19FF))
        || (cfg!(feature = "burmese") && matches!(u, 0x1000..=0x109F | 0xA9E0..=0xA9FF | 0xAA60..=0xAA7F));
    enabled && break_property(u) == BreakClass::ComplexContext
}
