//! `WordRun`: a wrapping run of pickable words in one Tab stop.

use crate::contract::Bind;
use crate::runtime::env::Label;
use egui::{Response, Ui};

/// How a word of a `WordRun` shows.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WordState {
    /// Plain.
    #[default]
    Normal,
    /// Struck through.
    Struck,
    /// Underlined.
    Underlined,
}

/// A word of a `WordRun`.
#[derive(Clone, Debug, PartialEq)]
pub struct RunWord<K> {
    /// What the word is picked as.
    pub key: K,
    /// The word's text.
    pub text: String,
    /// How the word shows.
    pub state: WordState,
}

/// A wrapping run of pickable words in one Tab stop.
#[allow(dead_code)] // P3(pdf-print-ocr-w2): PO-P15
pub struct WordRun<'a, K> {
    label: Label,
    words: &'a [RunWord<K>],
    selected: Bind<'a, Option<K>>,
}

impl<'a, K> WordRun<'a, K> {
    /// A run named `label` of `words`; the picked word's key is `selected`.
    pub fn new(label: impl Into<Label>, words: &'a [RunWord<K>], selected: Bind<'a, Option<K>>) -> Self {
        let _ = (label, words, selected);
        todo!("PO-P15")
    }

    /// Shows the run.
    pub fn show(self, ui: &mut Ui) -> WordRunOutput<K> {
        let _ = ui;
        todo!("PO-P15")
    }
}

/// What a `WordRun` returns.
#[non_exhaustive]
pub struct WordRunOutput<K> {
    /// The run's response.
    pub response: Response,
    /// The key of the word picked in this pass.
    pub picked: Option<K>,
}
