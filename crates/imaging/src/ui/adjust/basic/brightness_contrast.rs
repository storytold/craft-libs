//! Brightness/Contrast: its value and editor.

use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, EditResponse, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _};
use craft_ui::form::FieldGroup;
use craft_ui::runtime::env::Label;
use egui::Ui;

use crate::ui::adjust::sliders::{Run, lifted, read, words};
use crate::ui::adjust::{AdjustContext, AdjustField};

/// Contrast's lowest value, and with Use Legacy.
const CONTRAST_MIN: f64 = -50.0;
const LEGACY_CONTRAST_MIN: f64 = -100.0;

/// A Brightness/Contrast value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct BrightnessContrast {
    /// −150..=150, whole numbers.
    pub brightness: f64,
    /// −50..=100, or −100..=100 with `legacy`; whole numbers.
    pub contrast: f64,
    /// Use Legacy.
    pub legacy: bool,
}

impl BrightnessContrast {
    /// A value of `brightness`, `contrast` and Use Legacy.
    pub fn new(brightness: f64, contrast: f64, legacy: bool) -> Self {
        Self { brightness, contrast, legacy }
    }
}

/// What a Brightness/Contrast editor keeps between passes: the value its running gesture began with.
#[derive(Clone, Default)]
struct BrightnessMemory {
    began: Option<BrightnessContrast>,
}

/// Brightness/Contrast: two slider rows and Use Legacy.
///
/// - Brightness −150..150, Contrast −50..100 (−100..100 with Use Legacy), both default 0 on a reset, whole numbers.
/// - Turning Use Legacy off clamps Contrast to at least −50 in the same `Set`.
/// - Keyboard and AccessKit as [`AdjustSliders`](super::super::AdjustSliders); the check box is a Tab stop.
#[must_use = "show the editor with `show`"]
#[derive(Pattern)]
pub struct BrightnessContrastEditor<'a> {
    label: Label,
    value: Bind<'a, BrightnessContrast>,
    context: AdjustContext,
}

impl<'a> BrightnessContrastEditor<'a> {
    /// The editor named `label` of `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, BrightnessContrast>) -> Self {
        Self { label: label.into(), value, context: AdjustContext::dialog(true) }
    }

    /// Dialog or Properties panel; a dialog by default.
    pub fn context(mut self, context: AdjustContext) -> Self {
        self.context = context;
        self
    }

    /// Shows the editor and returns this pass's edit of the `BrightnessContrast`.
    pub fn show(self, ui: &mut Ui) -> EditResponse<BrightnessContrast> {
        let BrightnessContrastEditor { label, mut value, context } = self;
        let words = words(ui);
        let mut edit = None;
        let shown = FieldGroup::new(label).show(ui, |ui, cx| {
            let now = read(&mut value, cx).unwrap_or_default();
            let lowest = if now.legacy { LEGACY_CONTRAST_MIN } else { CONTRAST_MIN };
            let brightness = AdjustField::new("brightness", words.brightness.clone(), -150.0..=150.0, 0.0);
            let contrast = AdjustField::new("contrast", words.contrast.clone(), lowest..=100.0, 0.0);
            let mut run = Run::start(&mut value, context, now, cx.memory::<BrightnessMemory>().began);
            run.slider_row(ui, cx, &brightness, now.brightness, |now, x| BrightnessContrast { brightness: x, ..*now });
            run.slider_row(ui, cx, &contrast, now.contrast, |now, x| BrightnessContrast { contrast: x, ..*now });
            let legacy = Checkbox::new(Bind::given(now.legacy)).label(words.use_legacy.clone()).show(ui);
            // Turning Use Legacy off puts a Contrast below the plain range back in it in the same `Set`.
            run.push(cx, legacy, |now, legacy| BrightnessContrast {
                legacy: *legacy,
                contrast: if *legacy { now.contrast } else { now.contrast.max(CONTRAST_MIN) },
                ..*now
            });
            cx.memory::<BrightnessMemory>().began = run.began();
            edit = Some(run.finish());
        });
        lifted(edit, shown.output)
    }
}
