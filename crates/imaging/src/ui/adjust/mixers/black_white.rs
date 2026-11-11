//! Black & White: its value and editor.

use craft_theme::Srgba;
use craft_ui::buttons::Button;
use craft_ui::choice::{Checkbox, Swatch};
use craft_ui::contract::{Bind, EditResponse, Outcome, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _, clicked};
use craft_ui::form::FieldGroup;
use craft_ui::layout_components::Row;
use craft_ui::runtime::env::{Label, UnitKind};
use craft_ui::runtime::requests::ColorRequest;
use egui::Ui;

use crate::ui::adjust::sliders::{Run, lifted, read, words};
use crate::ui::adjust::style::{mix, tones};
use crate::ui::adjust::{AdjustContext, AdjustField, AdjustTrack};

/// The key of the tint colour request.
const TINT_KEY: &str = "blackWhite.tint";

/// A Black & White value.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BlackWhite {
    /// Reds, Yellows, Greens, Cyans, Blues, Magentas, −200..=300 %.
    pub weights: [f64; 6],
    /// Tint is on.
    pub tint: bool,
    /// The tint colour.
    pub tint_color: Srgba,
}

impl BlackWhite {
    /// A value of the six `weights`, Tint and its colour.
    pub fn new(weights: [f64; 6], tint: bool, tint_color: Srgba) -> Self {
        Self { weights, tint, tint_color }
    }

    /// Photoshop's default weights: Reds 40, Yellows 60, Greens 40, Cyans 60, Blues 20, Magentas 80.
    pub const DEFAULT_WEIGHTS: [f64; 6] = [40.0, 60.0, 40.0, 60.0, 20.0, 80.0];
}

impl Default for BlackWhite {
    fn default() -> Self {
        Self { weights: Self::DEFAULT_WEIGHTS, tint: false, tint_color: Srgba::rgb(0.882, 0.827, 0.702) }
    }
}

/// What a Black & White editor keeps between passes: the value its running gesture began with.
#[derive(Clone, Default)]
struct BlackWhiteMemory {
    began: Option<BlackWhite>,
}

/// What [`BlackWhiteEditor::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct BlackWhiteOutput {
    /// The edit of the value.
    pub edit: EditResponse<BlackWhite>,
    /// The user clicked the Tint well: open the colour picker with this request (key `"blackWhite.tint"`) and write
    /// the answer into the value's `tint_color`.
    pub pick_color: Option<ColorRequest>,
    /// The user pressed Auto: the host computes the weights from the image and sets them.
    pub auto: bool,
}

impl Outcome for BlackWhiteOutput {
    type Value = BlackWhite;

    fn response(&self) -> &egui::Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

impl std::ops::Deref for BlackWhiteOutput {
    type Target = EditResponse<BlackWhite>;

    fn deref(&self) -> &Self::Target {
        &self.edit
    }
}

/// Black & White: six colour rows, Tint with a colour well, and the Auto and Default buttons.
///
/// - Rows Reds … Magentas, −200..300 %, each over a track from black to its colour while dynamic tracks are on; a
///   reset restores that row's default.
/// - Tint is a check box with a well that asks for a colour through [`BlackWhiteOutput::pick_color`]; the well is
///   off while Tint is.
/// - Auto reports `auto`; Default sets the six default weights as one `Set`.
#[must_use = "show the editor with `show`"]
#[derive(Pattern)]
pub struct BlackWhiteEditor<'a> {
    label: Label,
    value: Bind<'a, BlackWhite>,
    context: AdjustContext,
}

impl<'a> BlackWhiteEditor<'a> {
    /// The editor named `label` of `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, BlackWhite>) -> Self {
        Self { label: label.into(), value, context: AdjustContext::dialog(true) }
    }

    /// Dialog or Properties panel; a dialog by default.
    pub fn context(mut self, context: AdjustContext) -> Self {
        self.context = context;
        self
    }

    /// Shows the editor and returns this pass's edit and requests.
    pub fn show(self, ui: &mut Ui) -> BlackWhiteOutput {
        let BlackWhiteEditor { label, mut value, context } = self;
        let words = words(ui);
        let mut out = None;
        let shown = FieldGroup::new(label).show(ui, |ui, cx| {
            let now = read(&mut value, cx).unwrap_or_default();
            let names = [&words.reds, &words.yellows, &words.greens, &words.cyans, &words.blues, &words.magentas];
            let keys = ["reds", "yellows", "greens", "cyans", "blues", "magentas"];
            let mut run = Run::start(&mut value, context, now, cx.memory::<BlackWhiteMemory>().began);
            for (i, (name, key)) in names.into_iter().zip(keys).enumerate() {
                let field = AdjustField::new(key, name.clone(), -200.0..=300.0, BlackWhite::DEFAULT_WEIGHTS[i])
                    .unit_kind(UnitKind::Percent)
                    .track(AdjustTrack::Colors(vec![tones::BLACK, mix::RANGES[i]]));
                run.slider_row(ui, cx, &field, now.weights[i], move |now: &BlackWhite, x| {
                    let mut next = *now;
                    next.weights[i] = x;
                    next
                });
            }
            let (mut tint, mut pick_color, mut auto, mut reset) = (None, None, false, false);
            Row::new().show(ui, |ui, _| {
                tint = Some(Checkbox::new(Bind::given(now.tint)).label(words.tint.clone()).show(ui));
                pick_color = Swatch::new(words.tint_color.clone(), now.tint_color).picker(TINT_KEY).enabled(now.tint).show(ui).pick_color;
                auto = clicked(&Button::new(words.auto.clone()).id_salt("auto").show(ui));
                reset = clicked(&Button::new(words.default_button.clone()).id_salt("default").show(ui));
            });
            if let Some(tint) = tint {
                run.push(cx, tint, |now, on| BlackWhite { tint: *on, ..*now });
            }
            if reset {
                run.set(cx, &words.default_button, BlackWhite { weights: BlackWhite::DEFAULT_WEIGHTS, ..now });
            }
            cx.memory::<BlackWhiteMemory>().began = run.began();
            out = Some((run.finish(), pick_color, auto));
        });
        let (edit, pick_color, auto) = out.map_or((None, None, false), |(edit, pick_color, auto)| (Some(edit), pick_color, auto));
        BlackWhiteOutput { edit: lifted(edit, shown.output), pick_color, auto }
    }
}
