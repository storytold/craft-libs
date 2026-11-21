//! Channel Mixer: its values and editors, of a colour document's red, green and blue channels and of a CMYK
//! document's four inks.

use craft_theme::variant::Tone;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, EditResponse, Entry, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _, TooltipSpec};
use craft_ui::display::Text;
use craft_ui::form::{Field, Help};
use craft_ui::runtime::env::{Env, Label, UnitKind, Units};
use craft_ui::style::TypeRole;
use egui::Ui;

use super::Tables;
use crate::ui::adjust::sliders::{Run, editor_group, lifted, read, words};
use crate::ui::adjust::style::{mix, tones};
use crate::ui::adjust::{AdjustContext, AdjustField, AdjustTrack, AdjustWords};

/// A Channel Mixer value: per output channel the Red, Green, Blue and Constant percentages.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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

/// The Total readout as text: whole percent, with a sign for zero and up. A Total over 100 % that whole percent would
/// show as 100 % keeps its hundredths, so the readout agrees with its warning.
fn total_text(units: &Units, words: &AdjustWords, total: f64) -> String {
    let whole = total.round();
    let text =
        if total_warns(total) && whole <= 100.0 { units.text(UnitKind::Percent, total, 0..=2) } else { units.text(UnitKind::Percent, whole, 0..=0) };
    if whole < 0.0 { text } else { words.plus.fill(&[("n", &text)]) }
}

/// Whether the Total is over what the channels can show. It is compared to the hundredth, the most the readout shows:
/// sources that add up to 100 % can sum a hair over it in floating point (67.89 + 28.35 + 3.76), and the readout would
/// then warn of an excess it cannot show.
fn total_warns(total: f64) -> bool {
    (total * 100.0).round() > 100.0 * 100.0
}

/// What a Channel Mixer editor keeps between passes: the output channel shown and the value its running gesture began
/// with.
#[derive(Clone)]
struct MixerMemory<V> {
    output: usize,
    began: Option<V>,
}

impl<V> Default for MixerMemory<V> {
    fn default() -> Self {
        Self { output: 0, began: None }
    }
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
        let shown = editor_group(ui, label.clone(), context, |ui, cx| {
            let now = read(&mut value, cx).unwrap_or_default();
            let mono = now.gray.is_some();
            // Monochrome has the one output, Gray.
            let outputs = if mono { 1 } else { 3 };
            let memory = cx.memory::<MixerMemory<ChannelMixer>>().clone();
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
            let tables =
                Tables { name: words.output_channel.clone(), entries: &entries, picked: out, fields: &fields, values: &row[..3], enabled: true };
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
            let memory = cx.memory::<MixerMemory<ChannelMixer>>();
            memory.began = run.began();
            memory.output = picked.unwrap_or(out);
            edit = Some(run.finish());
        });
        lifted(edit, shown.output)
    }
}

/// A CMYK Channel Mixer value: per ink output the Cyan, Magenta, Yellow, Black and Constant percentages.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct CmykChannelMixer {
    /// The Cyan output: Cyan, Magenta, Yellow, Black, Constant.
    pub cyan: [f64; 5],
    /// The Magenta output.
    pub magenta: [f64; 5],
    /// The Yellow output.
    pub yellow: [f64; 5],
    /// The Black output.
    pub black: [f64; 5],
    /// The Gray output while Monochrome is on.
    pub gray: Option<[f64; 5]>,
}

impl CmykChannelMixer {
    /// A value of the four ink output rows and the Gray row of Monochrome.
    pub fn new(cyan: [f64; 5], magenta: [f64; 5], yellow: [f64; 5], black: [f64; 5], gray: Option<[f64; 5]>) -> Self {
        Self { cyan, magenta, yellow, black, gray }
    }

    /// The row of output `out`; the one Gray row in Monochrome.
    fn shown_row(&self, out: usize) -> [f64; 5] {
        match (self.gray, out) {
            (Some(gray), _) => gray,
            (None, 0) => self.cyan,
            (None, 1) => self.magenta,
            (None, 2) => self.yellow,
            (None, _) => self.black,
        }
    }

    /// The value with the row of output `out` replaced. The rest of the row stays as it was: an imported value need
    /// not be whole percent, and the edit that ends a gesture already comes rounded to its row's step
    /// (`Run::slider_row`).
    fn with_row(mut self, out: usize, row: [f64; 5]) -> Self {
        match (self.gray, out) {
            (Some(_), _) => self.gray = Some(row),
            (None, 0) => self.cyan = row,
            (None, 1) => self.magenta = row,
            (None, 2) => self.yellow = row,
            (None, _) => self.black = row,
        }
        self
    }

    /// The sum of the four sources.
    fn total(row: [f64; 5]) -> f64 {
        row[0] + row[1] + row[2] + row[3]
    }
}

impl Default for CmykChannelMixer {
    fn default() -> Self {
        let identity = |out: usize| std::array::from_fn(|i| if i == out { 100.0 } else { 0.0 });
        Self { cyan: identity(0), magenta: identity(1), yellow: identity(2), black: identity(3), gray: None }
    }
}

/// Channel Mixer of a CMYK document: the Output Channel select of the four inks, the four source rows of that output,
/// the Total readout, Constant and Monochrome.
///
/// - The output channel is view state of the editor.
/// - Rows Cyan, Magenta, Yellow, Black (−200..200 %, tracks from white to the ink while dynamic tracks are on) start
///   from the identity of their output on a reset; Constant resets to 0.
/// - The Total readout is the sum of the four sources and warns above 100 %.
/// - Monochrome cannot be turned on. A value that has it on (an imported file) shows its one Gray output with the rows
///   and Constant disabled, and Monochrome can only be turned off, back to the four ink outputs.
#[must_use = "show the editor with `show`"]
#[derive(Pattern)]
pub struct CmykChannelMixerEditor<'a> {
    label: Label,
    value: Bind<'a, CmykChannelMixer>,
    context: AdjustContext,
    monochrome_tip: Option<Label>,
}

impl<'a> CmykChannelMixerEditor<'a> {
    /// The editor named `label` of `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, CmykChannelMixer>) -> Self {
        Self { label: label.into(), value, context: AdjustContext::dialog(true), monochrome_tip: None }
    }

    /// Dialog or Properties panel; a dialog by default.
    pub fn context(mut self, context: AdjustContext) -> Self {
        self.context = context;
        self
    }

    /// The tooltip of the Monochrome check box, which says why it cannot be turned on; none by default. The disabled box
    /// stays a Tab stop and shows it on keyboard focus.
    pub fn monochrome_tip(mut self, tip: impl Into<Label>) -> Self {
        self.monochrome_tip = Some(tip.into());
        self
    }

    /// Shows the editor and returns this pass's edit of the `CmykChannelMixer`.
    pub fn show(self, ui: &mut Ui) -> EditResponse<CmykChannelMixer> {
        let CmykChannelMixerEditor { label, mut value, context, monochrome_tip } = self;
        let words = words(ui);
        let units = Env::of(ui).units();
        let mut edit = None;
        let shown = editor_group(ui, label.clone(), context, |ui, cx| {
            let now = read(&mut value, cx).unwrap_or_default();
            let mono = now.gray.is_some();
            let outputs = if mono { 1 } else { 4 };
            let memory = cx.memory::<MixerMemory<CmykChannelMixer>>().clone();
            let out = if memory.output < outputs { memory.output } else { 0 };
            let row = now.shown_row(out);
            // A row starts from the identity of its output. Monochrome's Gray row takes no edits, so its reset never runs.
            let identity: [f64; 5] = std::array::from_fn(|i| if i == out { 100.0 } else { 0.0 });
            let sources = [("cyan", &words.cyan), ("magenta", &words.magenta), ("yellow", &words.yellow), ("black", &words.black)];
            let fields: Vec<AdjustField> = sources
                .iter()
                .enumerate()
                .map(|(i, (key, name))| {
                    AdjustField::new(key, (*name).clone(), -200.0..=200.0, identity[i])
                        .unit_kind(UnitKind::Percent)
                        .track(AdjustTrack::Colors(vec![tones::WHITE, mix::INKS[i]]))
                })
                .collect();
            let entries: Vec<Entry<usize>> = if mono {
                vec![Entry::new(0, words.gray.clone())]
            } else {
                sources.iter().enumerate().map(|(i, (_, name))| Entry::new(i, (*name).clone())).collect()
            };
            let mut run = Run::start(&mut value, context, now, memory.began);
            let set = move |now: &CmykChannelMixer, i: usize, x: f64| {
                let mut row = now.shown_row(out);
                row[i] = x;
                now.with_row(out, row)
            };
            let tables =
                Tables { name: words.output_channel.clone(), entries: &entries, picked: out, fields: &fields, values: &row[..4], enabled: !mono };
            let picked = tables.show(ui, cx, &mut run, set);
            let total = CmykChannelMixer::total(row);
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
            run.slider_row_enabled(ui, cx, &constant, row[4], !mono, move |now: &CmykChannelMixer, x| set(now, 4, x));
            // Monochrome only turns off: the box is enabled only while it is on. The reason is its disabled text too, which
            // keeps the disabled box a Tab stop, so the keyboard reaches the reason as well as the pointer.
            let mut monochrome = Checkbox::new(Bind::given(mono)).label(words.monochrome.clone()).enabled(mono);
            if let Some(tip) = &monochrome_tip {
                monochrome = monochrome.tooltip(TooltipSpec::new(tip.clone()).disabled_text(tip.clone()));
            }
            let monochrome = monochrome.show(ui);
            run.push(cx, monochrome, |now, on| if *on { *now } else { CmykChannelMixer { gray: None, ..*now } });
            let memory = cx.memory::<MixerMemory<CmykChannelMixer>>();
            memory.began = run.began();
            memory.output = picked.unwrap_or(out);
            edit = Some(run.finish());
        });
        lifted(edit, shown.output)
    }
}
