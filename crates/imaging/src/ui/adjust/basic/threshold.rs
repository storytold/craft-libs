//! Threshold: its editor.

use craft_ui::contract::{Bind, EditResponse, Pattern, Remember as _, Show as _, ShowContainer as _};
use craft_ui::form::FieldGroup;
use craft_ui::runtime::env::Label;
use egui::Ui;

use crate::ui::adjust::sliders::{Run, lifted, read, words};
use crate::ui::adjust::{AdjustContext, AdjustField};
use crate::ui::histogram::{ChannelColor, Histogram, HistogramData};

/// Threshold Level's range and default.
const THRESHOLD_MIN: f64 = 1.0;
const THRESHOLD_MAX: f64 = 255.0;
const THRESHOLD_DEFAULT: f64 = 128.0;
/// The bins of the plot while the host gives no histogram.
const THRESHOLD_BINS: usize = 256;

/// What a Threshold editor keeps between passes: the level its running gesture began with.
#[derive(Clone, Default)]
struct ThresholdMemory {
    began: Option<f64>,
}

/// Threshold: the image's histogram with a marker at the level, then the Threshold Level row (1..=255, default 128,
/// whole numbers).
///
/// The histogram is the bordered plot of [`Histogram`] with its marker at the level; AccessKit names it "Histogram".
/// The row behaves as an [`AdjustSliders`](super::super::AdjustSliders) row.
#[must_use = "show the editor with `show`"]
#[derive(Pattern)]
pub struct ThresholdEditor<'a> {
    label: Label,
    level: Bind<'a, f64>,
    histogram: Option<&'a HistogramData>,
    context: AdjustContext,
}

impl<'a> ThresholdEditor<'a> {
    /// The editor named `label` of `level` (1..=255).
    pub fn new(label: impl Into<Label>, level: Bind<'a, f64>) -> Self {
        Self { label: label.into(), level, histogram: None, context: AdjustContext::dialog(true) }
    }

    /// The image's histogram; an empty plot without it.
    pub fn histogram(mut self, data: &'a HistogramData) -> Self {
        self.histogram = Some(data);
        self
    }

    /// Dialog or Properties panel; a dialog by default.
    pub fn context(mut self, context: AdjustContext) -> Self {
        self.context = context;
        self
    }

    /// Shows the editor and returns this pass's edit of the level.
    pub fn show(self, ui: &mut Ui) -> EditResponse<f64> {
        let ThresholdEditor { label, mut level, histogram, context } = self;
        let words = words(ui);
        let mut edit = None;
        let shown = FieldGroup::new(label).show(ui, |ui, cx| {
            let now = read(&mut level, cx).unwrap_or(THRESHOLD_DEFAULT).clamp(THRESHOLD_MIN, THRESHOLD_MAX);
            let empty = HistogramData::tone(&[0u32; THRESHOLD_BINS][..], ChannelColor::Neutral);
            let data = histogram.unwrap_or(&empty);
            let _ = Histogram::new(words.histogram.clone(), data).bordered().marker(now as f32).show(ui);
            let row = AdjustField::new("level", words.threshold_level.clone(), THRESHOLD_MIN..=THRESHOLD_MAX, THRESHOLD_DEFAULT);
            let mut run = Run::start(&mut level, context, now, cx.memory::<ThresholdMemory>().began);
            run.slider_row(ui, cx, &row, now, |_, x| x);
            cx.memory::<ThresholdMemory>().began = run.began();
            edit = Some(run.finish());
        });
        lifted(edit, shown.output)
    }
}
