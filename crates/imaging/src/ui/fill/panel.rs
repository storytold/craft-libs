//! [`FillPanel`]: the Content-Aware Fill panel.

use std::ops::RangeInclusive;

use egui::{Color32, Response, Ui};

use craft_behavior::{EditEvents, IconId};
use craft_theme::variant::Tone;
use craft_theme::{ControlSize, Srgba};
use craft_ui::buttons::IconButton;
use craft_ui::choice::{Checkbox, Segmented, Swatch};
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Pattern, Props as _, SaveKey, Show as _, ShowContainer as _, clicked};
use craft_ui::display::Text;
use craft_ui::form::{Field, SliderField};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::patterns::form_dialogs::ParamDialogWords;
use craft_ui::pickers::Select;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::sections::Section;
use craft_ui::style::TypeRole;

use super::OVERLAY_COLOR_KEY;
use super::session::{ColorAdaptation, FillOutputTo, FillReset, FillSettings, RotationAdaptation, SamplingMode};
use super::words::FillWords;

/// What [`FillPanel::show`] reports.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FillPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// The edits of the settings this pass, in order, each with the settings as they stand after it (written to
    /// the settings already): the tint and the preview follow. A slider's drag is
    /// `Begin`, `Update` and `Commit`; a check box or a select is one `Set`.
    pub edit: EditEvents<FillSettings>,
    /// A sampling option picked (the session changes: `FillSession::set_sampling`).
    pub sampling: Option<SamplingMode>,
    /// A section's Reset pressed.
    pub reset: Option<FillReset>,
    /// The Color swatch pressed: the app opens its color picker (key [`OVERLAY_COLOR_KEY`]) and
    /// answers with `ContentAwareFillWorkspace::set_color`.
    pub pick_color: Option<ColorRequest>,
}

impl Outcome for FillPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The settings column's content, top to bottom:
///
/// - **Sampling Area Overlay** (closed at first): Show Sampling Area, Opacity (0..=100 %),
///   Color (a swatch), Indicates (Sampling Area, Excluded Area).
/// - **Sampling Area Options** with a Reset icon button in the header: Auto,
///   Rectangular, Custom (a segmented choice), Sample All Layers.
/// - **Fill Settings** with its Reset: Color Adaptation (None, Default, High, Very High), Rotation
///   Adaptation (None, Low, Medium, High, Full), Scale, Mirror.
/// - **Output Settings**: Output To (Current Layer, New Layer, Duplicate Layer).
/// - The error line (a limit, a failed Apply or preview) in the bad tone, caption role.
///
/// The sections remember their open state. The footer (Reset at the leading end; OK, Apply, Cancel in the
/// platform's order) is the dialog's, drawn by the workspace.
#[derive(Pattern)]
pub struct FillPanel<'a> {
    settings: Bind<'a, FillSettings>,
    sampling: SamplingMode,
    error: Option<&'a str>,
}

/// What the controls of one pass add up to.
#[derive(Default)]
struct Pass {
    edit: EditEvents<FillSettings>,
    sampling: Option<SamplingMode>,
    reset: Option<FillReset>,
    pick_color: Option<ColorRequest>,
}

/// Where a control writes its value in the settings.
type Write<T> = fn(&mut FillSettings, T);

/// A control's edit of one field as an edit of the settings: the settings at the start of the pass with that field
/// replaced.
fn lift<T: Clone + 'static>(events: EditEvents<T>, base: &FillSettings, set: Write<T>) -> EditEvents<FillSettings> {
    events.map(|value| {
        let mut settings = base.clone();
        set(&mut settings, value);
        settings
    })
}

impl<'a> FillPanel<'a> {
    /// The panel over `settings`, which it edits in place, with the session's `sampling` option shown as picked.
    pub fn new(settings: Bind<'a, FillSettings>, sampling: SamplingMode) -> Self {
        Self { settings, sampling, error: None }
    }

    /// Sets an error message to show under the panel; `None` shows none.
    #[must_use]
    pub fn error(mut self, error: Option<&'a str>) -> Self {
        self.error = error;
        self
    }

    /// Draws the panel and returns its response and any setting changes.
    pub fn show(self, ui: &mut Ui) -> FillPanelOutput {
        // pc: crates/ui-egui/src/dialogs/caf_ui/view.rs (the four sections).
        let Self { mut settings, sampling, error } = self;
        let env = Env::of(ui);
        let words = env.words::<FillWords>();
        let reset_label = env.words::<ParamDialogWords>().reset.clone();
        let mut done = Pass::default();
        let shown = Stack::new().show(ui, |ui, root| {
            let Some(base) = settings.reborrow().read(root).and_then(|held| held.same().cloned()) else { return };
            let mut rows = Rows { base: &base, pass: Pass::default() };
            Section::new(words.sampling_overlay.clone())
                .initially_open(false)
                .remember(SaveKey::new("craft_imaging.fill.overlay"))
                .show(ui, |ui, _| rows.overlay(ui, &words));
            let mut reset_sampling = false;
            Section::new(words.sampling_options.clone())
                .remember(SaveKey::new("craft_imaging.fill.sampling"))
                .controls(|ui, _| reset_sampling = reset_button(ui, "reset-sampling", &reset_label))
                .show(ui, |ui, _| rows.sampling_options(ui, &words, sampling));
            if reset_sampling {
                rows.pass.reset = Some(FillReset::Sampling);
            }
            let mut reset_fill = false;
            Section::new(words.fill_settings.clone())
                .remember(SaveKey::new("craft_imaging.fill.settings"))
                .controls(|ui, _| reset_fill = reset_button(ui, "reset-fill", &reset_label))
                .show(ui, |ui, _| rows.fill_settings(ui, &words));
            if reset_fill {
                rows.pass.reset = Some(FillReset::Fill);
            }
            Section::new(words.output_settings.clone()).remember(SaveKey::new("craft_imaging.fill.output")).show(ui, |ui, _| {
                let outputs = [
                    (FillOutputTo::CurrentLayer, &words.current_layer),
                    (FillOutputTo::NewLayer, &words.new_layer),
                    (FillOutputTo::DuplicateLayer, &words.duplicate_layer),
                ];
                rows.choose(ui, &words.output_to, base.output, &outputs, |s, v| s.output = v);
            });
            if let Some(error) = error {
                Text::new(error.to_owned()).role(TypeRole::Caption).tone(Tone::Bad).show(ui);
            }
            for event in &rows.pass.edit {
                let _ = settings.reborrow().write(root, event);
            }
            done = rows.pass;
        });
        FillPanelOutput { response: shown.output, edit: done.edit, sampling: done.sampling, reset: done.reset, pick_color: done.pick_color }
    }
}

/// A Reset icon button in a section header; whether it was pressed.
fn reset_button(ui: &mut Ui, salt: &'static str, label: &Label) -> bool {
    let icon = IconSpec::svg(IconId::new("undo"));
    clicked(&IconButton::new(icon).label(label.clone()).size(ControlSize::Sm).id_salt(salt).show(ui))
}

/// The controls of one pass: the settings they started from and what they add up to.
struct Rows<'p> {
    base: &'p FillSettings,
    pass: Pass,
}

impl Rows<'_> {
    fn overlay(&mut self, ui: &mut Ui, words: &FillWords) {
        let base = self.base;
        self.check(ui, &words.show_sampling_area, base.show_overlay, |s, v| s.show_overlay = v);
        self.slider(ui, &words.opacity, base.opacity, 0.0..=100.0, |s, v| s.opacity = v);
        let [r, g, b] = base.color;
        let color = Srgba::from(Color32::from_rgb(r, g, b));
        Field::new(words.color.clone()).show(ui, |ui, _| {
            let swatch = Swatch::new(words.color.clone(), color).picker(OVERLAY_COLOR_KEY).show(ui);
            if swatch.pick_color.is_some() {
                self.pass.pick_color = swatch.pick_color;
            }
        });
        let kinds = [(false, &words.sampling_area), (true, &words.excluded_area)];
        self.choose(ui, &words.indicates, base.excluded, &kinds, |s, v| s.excluded = v);
    }

    fn sampling_options(&mut self, ui: &mut Ui, words: &FillWords, sampling: SamplingMode) {
        let modes = [
            Entry::new(SamplingMode::Auto, words.auto.clone()),
            Entry::new(SamplingMode::Rectangular, words.rectangular.clone()),
            Entry::new(SamplingMode::Custom, words.custom.clone()),
        ];
        let picked = Segmented::new(words.sampling_options.clone(), Bind::given(sampling), &modes).show(ui).value().copied();
        if let Some(mode) = picked.filter(|mode| *mode != sampling) {
            self.pass.sampling = Some(mode);
        }
        let base = self.base;
        self.check(ui, &words.sample_all_layers, base.sample_all_layers, |s, v| s.sample_all_layers = v);
    }

    fn fill_settings(&mut self, ui: &mut Ui, words: &FillWords) {
        let base = self.base;
        let colors = [
            (ColorAdaptation::None, &words.none),
            (ColorAdaptation::Default, &words.default),
            (ColorAdaptation::High, &words.high),
            (ColorAdaptation::VeryHigh, &words.very_high),
        ];
        self.choose(ui, &words.color_adaptation, base.color_adaptation, &colors, |s, v| s.color_adaptation = v);
        let rotations = [
            (RotationAdaptation::None, &words.none),
            (RotationAdaptation::Low, &words.low),
            (RotationAdaptation::Medium, &words.medium),
            (RotationAdaptation::High, &words.high),
            (RotationAdaptation::Full, &words.full),
        ];
        self.choose(ui, &words.rotation_adaptation, base.rotation, &rotations, |s, v| s.rotation = v);
        Row::new().show(ui, |ui, _| {
            self.check(ui, &words.scale, base.scale, |s, v| s.scale = v);
            self.check(ui, &words.mirror, base.mirror, |s, v| s.mirror = v);
        });
    }

    /// The Opacity slider (0..=100 %, whole numbers); its edit is the pass's edit.
    fn slider(&mut self, ui: &mut Ui, label: &Label, value: f64, range: RangeInclusive<f64>, set: Write<f64>) {
        let field = SliderField::new(label.clone(), Bind::given(value), range).step(1.0).unit_kind(UnitKind::Percent);
        self.pass.edit.extend(lift(field.show(ui).events, self.base, set));
    }

    /// A check box over `value`; its edit is the pass's edit.
    fn check(&mut self, ui: &mut Ui, label: &Label, value: bool, set: Write<bool>) {
        let events = Checkbox::new(Bind::given(value)).label(label.clone()).show(ui).events;
        self.pass.edit.extend(lift(events, self.base, set));
    }

    /// A select over `options` captioned `label`; its edit is the pass's edit.
    fn choose<T: Clone + PartialEq + Send + Sync + 'static>(
        &mut self,
        ui: &mut Ui,
        label: &Label,
        current: T,
        options: &[(T, &Label)],
        set: Write<T>,
    ) {
        let entries: Vec<Entry<T>> = options.iter().map(|(value, text)| Entry::new(value.clone(), (*text).clone())).collect();
        Field::new(label.clone()).show(ui, |ui, _| {
            let events = Select::new(label.clone(), Bind::given(current.clone()), &entries).show(ui).events;
            self.pass.edit.extend(lift(events, self.base, set));
        });
    }
}
