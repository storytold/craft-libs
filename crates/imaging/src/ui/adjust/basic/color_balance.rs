//! Color Balance: its value, tone and editor.

use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, EditResponse, Outcome, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::runtime::env::Label;
use egui::Ui;

use crate::ui::adjust::sliders::{Run, lifted, read, words};
use crate::ui::adjust::{AdjustContext, AdjustField};

/// The tone Color Balance shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColorBalanceTone {
    /// The shadows.
    Shadows,
    /// The default, and where a reset returns.
    #[default]
    Midtones,
    /// The highlights.
    Highlights,
}

/// A Color Balance value: per tone the Cyan–Red, Magenta–Green and Yellow–Blue shifts, −100..=100, whole numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ColorBalance {
    /// Shifts for the shadows: Cyan–Red, Magenta–Green, Yellow–Blue.
    pub shadows: [f64; 3],
    /// Shifts for the midtones, in the same order.
    pub midtones: [f64; 3],
    /// Shifts for the highlights, in the same order.
    pub highlights: [f64; 3],
    /// Preserve Luminosity; on by default.
    pub preserve_luminosity: bool,
}

impl Default for ColorBalance {
    fn default() -> Self {
        Self { shadows: [0.0; 3], midtones: [0.0; 3], highlights: [0.0; 3], preserve_luminosity: true }
    }
}

impl ColorBalance {
    /// A value of the shifts of each tone and Preserve Luminosity.
    pub fn new(shadows: [f64; 3], midtones: [f64; 3], highlights: [f64; 3], preserve_luminosity: bool) -> Self {
        Self { shadows, midtones, highlights, preserve_luminosity }
    }

    /// Photoshop's reset to defaults (Alt+Cancel; the Properties panel's Reset once the layer is as the panel found
    /// it): every tone back to 0, Preserve Luminosity kept. The editor shows Midtones again when the host gives
    /// [`ColorBalanceEditor::tone`] `Midtones`.
    pub fn reset(&mut self) {
        self.shadows = [0.0; 3];
        self.midtones = [0.0; 3];
        self.highlights = [0.0; 3];
    }

    /// The shifts of `tone`.
    fn of_tone(&self, tone: ColorBalanceTone) -> [f64; 3] {
        match tone {
            ColorBalanceTone::Shadows => self.shadows,
            ColorBalanceTone::Midtones => self.midtones,
            ColorBalanceTone::Highlights => self.highlights,
        }
    }

    /// The value with the shifts of `tone` replaced.
    fn with_tone(mut self, tone: ColorBalanceTone, shifts: [f64; 3]) -> Self {
        match tone {
            ColorBalanceTone::Shadows => self.shadows = shifts,
            ColorBalanceTone::Midtones => self.midtones = shifts,
            ColorBalanceTone::Highlights => self.highlights = shifts,
        }
        self
    }
}

/// What a Color Balance editor keeps between passes: the tone it shows and the value its running gesture began with.
#[derive(Clone, Default)]
struct BalanceMemory {
    tone: Option<ColorBalanceTone>,
    began: Option<ColorBalance>,
}

/// What a [`ColorBalanceEditor`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ColorBalanceOutput {
    /// The edit of the value.
    pub edit: EditResponse<ColorBalance>,
    /// The tone shown after the pass; a host that keeps the tone stores it.
    pub tone: ColorBalanceTone,
}

impl Outcome for ColorBalanceOutput {
    type Value = ColorBalance;

    fn response(&self) -> &egui::Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

impl std::ops::Deref for ColorBalanceOutput {
    type Target = EditResponse<ColorBalance>;

    fn deref(&self) -> &Self::Target {
        &self.edit
    }
}

/// Color Balance: the Tone radios, three two-colour slider rows for the shown tone, and Preserve Luminosity.
///
/// - The Tone radios (Shadows, Midtones, Highlights) pick the tone shown. The choice is view state: the editor keeps
///   it, Midtones at first, and it survives value changes. A host that keeps it sets [`tone`](Self::tone) and stores
///   the output's.
/// - Rows "Cyan · Red", "Magenta · Green", "Yellow · Blue", −100..100, with two-colour tracks while dynamic tracks are
///   on. A reset zeroes only that row of the shown tone.
/// - Preserve Luminosity is one `Set`.
#[must_use = "show the editor with `show`"]
#[derive(Pattern)]
pub struct ColorBalanceEditor<'a> {
    label: Label,
    value: Bind<'a, ColorBalance>,
    tone: Option<ColorBalanceTone>,
    context: AdjustContext,
}

impl<'a> ColorBalanceEditor<'a> {
    /// The editor named `label` of `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, ColorBalance>) -> Self {
        Self { label: label.into(), value, tone: None, context: AdjustContext::dialog(true) }
    }

    /// The tone shown, kept by the host.
    pub fn tone(mut self, tone: ColorBalanceTone) -> Self {
        self.tone = Some(tone);
        self
    }

    /// Dialog or Properties panel; a dialog by default.
    pub fn context(mut self, context: AdjustContext) -> Self {
        self.context = context;
        self
    }

    /// Shows the editor and returns this pass's edit of the `ColorBalance` and the tone shown.
    pub fn show(self, ui: &mut Ui) -> ColorBalanceOutput {
        let ColorBalanceEditor { label, mut value, tone: host_tone, context } = self;
        let words = words(ui);
        let mut out = None;
        let shown = FieldGroup::new(label).show(ui, |ui, cx| {
            let now = read(&mut value, cx).unwrap_or_default();
            let tone = host_tone.or(cx.memory::<BalanceMemory>().tone).unwrap_or_default();
            let mut picked = None;
            Field::new(words.tone.clone()).show(ui, |ui, _| {
                let group = RadioGroup::new(words.tone.clone(), Bind::given(tone)).show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        for (member, name) in [
                            (ColorBalanceTone::Shadows, &words.shadows),
                            (ColorBalanceTone::Midtones, &words.midtones),
                            (ColorBalanceTone::Highlights, &words.highlights),
                        ] {
                            Radio::new(member).label(name.clone()).show(ui);
                        }
                    });
                });
                picked = group.output.value().copied();
            });
            let rows = [
                ("cyanRed", &words.cyan_red, super::super::style::balance::CYAN_RED),
                ("magentaGreen", &words.magenta_green, super::super::style::balance::MAGENTA_GREEN),
                ("yellowBlue", &words.yellow_blue, super::super::style::balance::YELLOW_BLUE),
            ];
            let mut run = Run::start(&mut value, context, now, cx.memory::<BalanceMemory>().began);
            let shifts = now.of_tone(tone);
            for (i, (key, name, stops)) in rows.into_iter().enumerate() {
                let field = AdjustField::new(key, name.clone(), -100.0..=100.0, 0.0).track(super::super::AdjustTrack::Colors(stops.to_vec()));
                run.slider_row(ui, cx, &field, shifts[i], move |now: &ColorBalance, x| {
                    let mut next = now.of_tone(tone);
                    next[i] = x;
                    now.with_tone(tone, next)
                });
            }
            let preserve = Checkbox::new(Bind::given(now.preserve_luminosity)).label(words.preserve_luminosity.clone()).show(ui);
            run.push(cx, preserve, |now, on| ColorBalance { preserve_luminosity: *on, ..*now });
            let memory = cx.memory::<BalanceMemory>();
            memory.began = run.began();
            memory.tone = Some(picked.unwrap_or(tone));
            out = Some((run.finish(), picked.unwrap_or(tone)));
        });
        let (edit, tone) = out.map_or((None, host_tone.unwrap_or_default()), |(edit, tone)| (Some(edit), tone));
        ColorBalanceOutput { edit: lifted(edit, shown.output), tone }
    }
}
