//! Channel Mixer: its value and editor.

use craft_theme::variant::Tone;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, EditResponse, Entry, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::form::{Field, FieldGroup, Help};
use craft_ui::runtime::env::{Env, Label, UnitKind, Units};
use craft_ui::style::TypeRole;
use egui::Ui;

use super::Tables;
use crate::ui::adjust::sliders::{Run, lifted, read, words};
use crate::ui::adjust::style::{mix, tones};
use crate::ui::adjust::{AdjustContext, AdjustField, AdjustTrack, AdjustWords};

/// A Channel Mixer value: per output channel the Red, Green, Blue and Constant percentages.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ChannelMixer {
    /// The Red output: Red, Green, Blue, Constant.
    pub red: [f64; 4],
    /// The Green output.
    pub green: [f64; 4],
    /// The Blue output.
    pub blue: [f64; 4],
    /// The Gray output while Monochrome is on.
    pub gray: Option<[f64; 4]>,
}

impl ChannelMixer {
    /// A value of the three output rows and the Gray row of Monochrome.
    pub fn new(red: [f64; 4], green: [f64; 4], blue: [f64; 4], gray: Option<[f64; 4]>) -> Self {
        Self { red, green, blue, gray }
    }

    /// The Gray row Photoshop starts Monochrome with.
    pub const MONOCHROME_START: [f64; 4] = [40.0, 40.0, 20.0, 0.0];
}

impl Default for ChannelMixer {
    fn default() -> Self {
        Self { red: [100.0, 0.0, 0.0, 0.0], green: [0.0, 100.0, 0.0, 0.0], blue: [0.0, 0.0, 100.0, 0.0], gray: None }
    }
}

impl ChannelMixer {
    /// The row of output `out`; the one Gray row in Monochrome.
    fn shown_row(&self, out: usize) -> [f64; 4] {
        match (self.gray, out) {
            (Some(gray), _) => gray,
            (None, 0) => self.red,
            (None, 1) => self.green,
            (None, _) => self.blue,
        }
    }

    /// The value with the row of output `out` replaced, in whole percent.
    fn with_row(mut self, out: usize, row: [f64; 4]) -> Self {
        let row = row.map(f64::round);
        match (self.gray, out) {
            (Some(_), _) => self.gray = Some(row),
            (None, 0) => self.red = row,
            (None, 1) => self.green = row,
            (None, _) => self.blue = row,
        }
        self
    }

    /// The sum of the three sources.
    fn total(row: [f64; 4]) -> f64 {
        row[0] + row[1] + row[2]
    }
}

/// The Total readout as text: whole percent, with a sign for zero and up.
fn total_text(units: &Units, words: &AdjustWords, total: f64) -> String {
    let whole = total.round();
    let text = units.text(UnitKind::Percent, whole, 0..=0);
    if whole < 0.0 { text } else { words.plus.fill(&[("n", &text)]) }
}

/// Whether the Total is over what the channels can show.
fn total_warns(total: f64) -> bool {
    total > 100.0
}

/// What a Channel Mixer editor keeps between passes: the output channel shown and the value its running gesture began
/// with.
#[derive(Clone, Default)]
struct MixerMemory {
    output: usize,
    began: Option<ChannelMixer>,
}

/// Channel Mixer: the Output Channel select, the three source rows of that output, the Total readout, Constant and
/// Monochrome.
///
/// - The output channel is view state of the editor. With Monochrome on the one output is Gray, and turning it off
///   removes the Gray row.
/// - Rows Red, Green, Blue (−200..200 %, tracks from black to the channel while dynamic tracks are on) start from the
///   identity of their output on a reset; Constant resets to 0.
/// - The Total readout is the sum of the three sources and warns above 100 %.
#[must_use = "show the editor with `show`"]
#[derive(Pattern)]
pub struct ChannelMixerEditor<'a> {
    label: Label,
    value: Bind<'a, ChannelMixer>,
    context: AdjustContext,
}

impl<'a> ChannelMixerEditor<'a> {
    /// The editor named `label` of `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, ChannelMixer>) -> Self {
        Self { label: label.into(), value, context: AdjustContext::dialog(true) }
    }

    /// Dialog or Properties panel; a dialog by default.
    pub fn context(mut self, context: AdjustContext) -> Self {
        self.context = context;
        self
    }

    /// Shows the editor and returns this pass's edit of the `ChannelMixer`.
    pub fn show(self, ui: &mut Ui) -> EditResponse<ChannelMixer> {
        let ChannelMixerEditor { label, mut value, context } = self;
        let words = words(ui);
        let units = Env::of(ui).units();
        let mut edit = None;
        let shown = FieldGroup::new(label.clone()).show(ui, |ui, cx| {
            let now = read(&mut value, cx).unwrap_or_default();
            let mono = now.gray.is_some();
            // Monochrome has the one output, Gray.
            let outputs = if mono { 1 } else { 3 };
            let memory = cx.memory::<MixerMemory>().clone();
            let out = if memory.output < outputs { memory.output } else { 0 };
            let row = now.shown_row(out);
            // A row starts from the identity of its output (Gray from Photoshop's mix).
            let identity: [f64; 4] = if mono { ChannelMixer::MONOCHROME_START } else { std::array::from_fn(|i| if i == out { 100.0 } else { 0.0 }) };
            let sources = [("red", &words.red), ("green", &words.green), ("blue", &words.blue)];
            let fields: Vec<AdjustField> = sources
                .iter()
                .enumerate()
                .map(|(i, (key, name))| {
                    AdjustField::new(key, (*name).clone(), -200.0..=200.0, identity[i])
                        .unit_kind(UnitKind::Percent)
                        .track(AdjustTrack::Colors(vec![tones::BLACK, mix::SOURCES[i]]))
                })
                .collect();
            let entries: Vec<Entry<usize>> = if mono {
                vec![Entry::new(0, words.gray.clone())]
            } else {
                sources.iter().enumerate().map(|(i, (_, name))| Entry::new(i, (*name).clone())).collect()
            };
            let mut run = Run::start(&mut value, context, now, memory.began);
            let set = move |now: &ChannelMixer, i: usize, x: f64| {
                let mut row = now.shown_row(out);
                row[i] = x;
                now.with_row(out, row)
            };
            let tables = Tables { name: words.output_channel.clone(), entries: &entries, picked: out, fields: &fields, values: &row[..3] };
            let picked = tables.show(ui, cx, &mut run, set);
            let total = ChannelMixer::total(row);
            let warn = total_warns(total);
            let text = total_text(&units, &words, total);
            let mut readout = Field::new(words.total.clone()).id_salt("total");
            if warn {
                readout = readout.help(Help::Status(Tone::Warn, Label::from(words.total_over.fill(&[("value", &text)]))));
            }
            readout.show(ui, |ui, _| {
                let mut shown = Text::new(text.clone()).role(TypeRole::Mono);
                if warn {
                    shown = shown.tone(Tone::Warn);
                }
                shown.show(ui);
            });
            let constant = AdjustField::new("constant", words.constant.clone(), -200.0..=200.0, 0.0).unit_kind(UnitKind::Percent);
            run.slider_row(ui, cx, &constant, row[3], move |now: &ChannelMixer, x| set(now, 3, x));
            let monochrome = Checkbox::new(Bind::given(mono)).label(words.monochrome.clone()).show(ui);
            run.push(cx, monochrome, |now, on| ChannelMixer { gray: on.then_some(ChannelMixer::MONOCHROME_START), ..*now });
            let memory = cx.memory::<MixerMemory>();
            memory.began = run.began();
            memory.output = picked.unwrap_or(out);
            edit = Some(run.finish());
        });
        lifted(edit, shown.output)
    }
}
